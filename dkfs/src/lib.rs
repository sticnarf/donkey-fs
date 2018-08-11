//! Attention! This filesystem does not work
//! in a multi-threaded environment!
#![feature(nll)]
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate failure;
extern crate bincode;
#[macro_use]
extern crate nix;
extern crate byteorder;
extern crate im;

use block::*;
use device::Device;
use failure::Error;
use file::{DkDir, DkFile};
use std::cell::RefCell;
use std::collections::hash_map::HashMap;
use std::ffi::OsStr;
use std::io;
use std::ops::Deref;
use std::rc::Rc;
use std::time::SystemTime;

const BOOT_BLOCK_SIZE: u64 = 1024;
const SUPER_BLOCK_SIZE: u64 = 1024;
const INODE_SIZE: u64 = 256;
const BOOT_BLOCK_PTR: u64 = 0;
const SUPER_BLOCK_PTR: u64 = BOOT_BLOCK_PTR + BOOT_BLOCK_SIZE;
const FIRST_INODE_PTR: u64 = SUPER_BLOCK_PTR + SUPER_BLOCK_SIZE;
pub const DEFAULT_BYTES_PER_INODE: u64 = 16384;
/// This cannot be a very small integer. Inode numbers of
/// small integers are reserved for special use.
pub const ROOT_INODE: u64 = 114514;
const MAX_NAMELEN: u32 = 256;

pub use device::dev;
pub use file::{DkDirHandle, DkFileHandle};
pub use ops::Handle;

pub type DkResult<T> = std::result::Result<T, Error>;

pub fn open(mut dev: Box<Device>) -> DkResult<Handle> {
    let sb = SuperBlock::from_bytes(dev.read_at(SUPER_BLOCK_PTR)?)?;
    Ok(Handle::new(Donkey::new(dev, sb)))
}

pub fn format(mut dev: Box<Device>, opts: FormatOptions) -> DkResult<Handle> {
    let block_size = dev.block_size();
    let inode_count = dev.size() / opts.bytes_per_inode;
    let used_blocks = (FIRST_INODE_PTR + INODE_SIZE * inode_count + block_size - 1) / block_size;
    let first_db_ptr = used_blocks * block_size;

    // No plan to implement a real boot block here.

    // Make the initial super block
    let sb = SuperBlock {
        magic_number: block::MAGIC_NUMBER,
        block_size,
        inode_count,
        used_inode_count: 0,
        db_count: dev.block_count() - first_db_ptr / block_size,
        used_db_count: 0,
        inode_fl_ptr: FIRST_INODE_PTR,
        db_fl_ptr: first_db_ptr,
    };
    dev.write_at(&sb, SUPER_BLOCK_PTR)?;

    // Make the initial free inode
    let fi = FreeList {
        next_ptr: 0,
        size: inode_count * INODE_SIZE,
    };
    dev.write_at(&fi, FIRST_INODE_PTR)?;

    // Make the initial free data block
    let fb = FreeList {
        next_ptr: 0,
        size: dev.size() - first_db_ptr,
    };
    dev.write_at(&fb, first_db_ptr)?;

    let mut dk = Donkey::new(dev, sb);
    dk.create_root()?;
    Ok(Handle::new(dk))
}

#[derive(Debug)]
pub struct Donkey {
    dev: Box<Device>,
    sb: SuperBlock,
    opened_files: HashMap<u64, Rc<RefCell<DkFile>>>,
    opened_dirs: HashMap<u64, Rc<RefCell<DkDir>>>,
    close_file_list: Rc<RefCell<Vec<u64>>>,
    close_dir_list: Rc<RefCell<Vec<u64>>>,
}

impl Donkey {
    fn new(dev: Box<Device>, sb: SuperBlock) -> Donkey {
        Donkey {
            dev,
            sb,
            opened_files: HashMap::new(),
            opened_dirs: HashMap::new(),
            close_file_list: Rc::new(RefCell::new(Vec::new())),
            close_dir_list: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// This function is only called in `format`
    /// because we assume root inode is not allocated yet.
    fn create_root(&mut self) -> DkResult<()> {
        let perm = FileMode::USER_RWX
            | FileMode::GROUP_READ
            | FileMode::GROUP_EXECUTE
            | FileMode::OTHERS_READ
            | FileMode::OTHERS_EXECUTE;
        let root_inode = self.mkdir(ROOT_INODE, perm, 0, 0)?;

        if root_inode == ROOT_INODE {
            self.close_dirs_in_list()?;
            self.close_files_in_list()?;
            Ok(())
        } else {
            Err(format_err!(
                "Expected root inode number to be {}, but got {}.",
                ROOT_INODE,
                root_inode
            ))
        }
    }

    fn read_into(&mut self, ptr: u64, mut dst: &mut [u8]) -> DkResult<u64> {
        Ok(io::copy(
            &mut self.dev.read_len_at(ptr, dst.len() as u64)?,
            &mut dst,
        )?)
    }

    fn read<T: Readable>(&mut self, ptr: u64) -> DkResult<T> {
        <T as Readable>::from_bytes(self.dev.read_at(ptr)?)
    }

    fn read_block<T: Readable>(&mut self, ptr: u64) -> DkResult<T> {
        <T as Readable>::from_bytes(self.dev.read_block_at(ptr)?)
    }

    fn write(&mut self, ptr: u64, writable: &Writable) -> DkResult<()> {
        // println!("write {:?} at {}", writable, ptr);
        self.dev.write_at(writable, ptr)
    }

    fn block_size(&self) -> u64 {
        self.sb.block_size
    }

    fn flush_sb(&mut self) -> DkResult<()> {
        self.dev.write_at(&self.sb, SUPER_BLOCK_PTR)
    }

    fn close_files_in_list(&mut self) -> DkResult<()> {
        loop {
            let ino = self.close_file_list.borrow_mut().pop();
            match ino {
                Some(ino) => {
                    let drop = self.opened_files.get(&ino).and_then(|rc| {
                        if Rc::strong_count(rc) == 1 {
                            Some(rc.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(rc) = drop {
                        rc.borrow_mut().flush(self)?;
                        self.opened_files.remove(&ino);
                    }
                }
                None => return Ok(()),
            }
        }
    }

    fn close_dirs_in_list(&mut self) -> DkResult<()> {
        loop {
            let ino = self.close_dir_list.borrow_mut().pop();
            match ino {
                Some(ino) => {
                    let drop = self.opened_dirs.get(&ino).and_then(|rc| {
                        if Rc::strong_count(rc) == 1 {
                            Some(rc.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(rc) = drop {
                        rc.borrow_mut().flush(self)?;
                        self.opened_dirs.remove(&ino);
                    }
                }
                None => return Ok(()),
            }
        }
    }

    /// Allocated a block of size `size` from `FreeList` at `ptr`.
    /// Returns the pointer of the allocated block and the pointer
    /// of the new `FreeList`.
    fn allocate_from_free(&mut self, ptr: u64, size: u64) -> DkResult<(u64, u64)> {
        let fl: FreeList = self.read(ptr)?;
        if fl.size >= size {
            // Split this free list
            let new_fl = FreeList {
                size: fl.size - size,
                ..fl
            };
            let new_ptr = ptr + size;
            self.write(new_ptr, &new_fl)?;
            Ok((ptr, new_ptr))
        } else {
            self.allocate_from_free(fl.next_ptr, size)
        }
    }

    /// Returns the inode number of the allocated inode
    fn allocate_inode(&mut self) -> DkResult<u64> {
        if self.sb.used_inode_count < self.sb.inode_count {
            let fl_ptr = self.sb.inode_fl_ptr;
            let (fi_ptr, new_fl_ptr) = self.allocate_from_free(fl_ptr, INODE_SIZE)?;
            self.sb.inode_fl_ptr = new_fl_ptr;
            self.sb.used_inode_count += 1;
            self.flush_sb()?;
            Ok(Inode::ino(fi_ptr))
        } else {
            Err(format_err!("Inodes are used up!"))
        }
    }

    fn read_inode(&mut self, ino: u64) -> DkResult<Inode> {
        self.read(Inode::ptr(ino))
    }

    fn write_inode(&mut self, inode: &Inode) -> DkResult<()> {
        self.write(Inode::ptr(inode.ino), inode)
    }

    /// Returns the pointer of the allocated data block
    fn allocate_db(&mut self) -> DkResult<u64> {
        if self.sb.used_db_count < self.sb.db_count {
            let fl_ptr = self.sb.db_fl_ptr;
            let (fd_ptr, new_fl_ptr) = self.allocate_from_free(fl_ptr, self.block_size())?;
            self.sb.db_fl_ptr = new_fl_ptr;
            self.sb.used_db_count += 1;
            self.flush_sb()?;
            Ok(fd_ptr)
        } else {
            Err(format_err!("Data blocks are used up!"))
        }
    }

    fn fill_zero(&mut self, ptr: u64, size: u64) -> DkResult<()> {
        let v = vec![0u8; size as usize];
        let b = RefData(v.as_slice());
        self.write(ptr, &b)
    }

    /// Returns the inode number of the new node.
    fn mknod(
        &mut self,
        mode: FileMode,
        uid: u32,
        gid: u32,
        nlink: u64,
        rdev: Option<u64>,
    ) -> DkResult<u64> {
        let ino = self.allocate_inode()?;
        let time = SystemTime::now().into();
        let inode = Inode {
            ino,
            mode,
            uid,
            gid,
            nlink,
            atime: time,
            mtime: time,
            ctime: time,
            crtime: time,
            size: 0,
            blocks: 0,
            device: rdev.unwrap_or(0),
            ptrs: Default::default(),
        };
        self.write_inode(&inode)?;
        Ok(ino)
    }

    /// Returns the inode number of the new directory.
    /// This method **DOES NOT** link the new directory to
    /// the parent directory!
    fn mkdir(&mut self, parent_ino: u64, mode: FileMode, uid: u32, gid: u32) -> DkResult<u64> {
        let mode = FileMode::DIRECTORY | mode;
        let ino = self.mknod(mode, uid, gid, 0, None)?;

        // Create `.` and `..` entry
        let dir = self.open_dir(ino)?;
        self.link(ino, dir.clone(), OsStr::new("."))?;
        self.link(parent_ino, dir, OsStr::new(".."))?;

        Ok(ino)
    }

    fn link(&mut self, ino: u64, parent: DkDirHandle, name: &OsStr) -> DkResult<()> {
        parent.add_entry(name, ino)?;
        let file = self.open(ino, Flags::READ_ONLY)?;
        file.inner.borrow_mut().inode.nlink += 1;
        Ok(())
    }

    fn open(&mut self, ino: u64, flags: Flags) -> DkResult<DkFileHandle> {
        self.close_files_in_list()?;
        // We do not use entry API here to prevent `self` being borrowed twice
        let inner = match self.opened_files.get(&ino) {
            Some(inner) => inner.clone(),
            None => {
                let inode = self.read_inode(ino)?;
                let f = DkFile::new(inode, self.close_file_list.clone());
                let rc = Rc::new(RefCell::new(f));
                self.opened_files.insert(ino, rc.clone());
                rc
            }
        };
        let df = DkFileHandle { inner, flags };
        Ok(df)
    }

    fn open_dir(&mut self, ino: u64) -> DkResult<DkDirHandle> {
        self.close_dirs_in_list()?;
        // We do not use entry API here to prevent `self` being borrowed twice
        let inner = match self.opened_dirs.get(&ino) {
            Some(inner) => inner.clone(),
            None => {
                let fh = self.open(ino, Flags::READ_WRITE)?;
                let mut dir = DkDir::from_file(fh, self.close_dir_list.clone())?;
                dir.read_fully(self)?;
                let rc = Rc::new(RefCell::new(dir));
                self.opened_dirs.insert(ino, rc.clone());
                rc
            }
        };
        let entries = inner.borrow().entries.clone();
        let dd = DkDirHandle { inner, entries };

        Ok(dd)
    }
}

impl Drop for Donkey {
    fn drop(&mut self) {
        if let Err(e) = self.close_dirs_in_list() {
            eprintln!(
                "Failed to close some directories: {}. This may lead to filesystem corruption!",
                e
            );
        }
        if let Err(e) = self.close_files_in_list() {
            eprintln!(
                "Failed to close some files: {}. This may lead to filesystem corruption!",
                e
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    bytes_per_inode: u64,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
        }
    }
}

impl FormatOptions {
    pub fn bytes_per_inode(mut self, bytes_per_inode: u64) -> Self {
        self.bytes_per_inode = bytes_per_inode;
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct DkTimespec {
    pub sec: i64,
    pub nsec: u32,
}

impl From<SystemTime> for DkTimespec {
    fn from(t: SystemTime) -> Self {
        // We can simply unwrap this result because no time precedes UNIX_EPOCH
        let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        DkTimespec {
            sec: duration.as_secs() as i64,
            nsec: duration.subsec_nanos(),
        }
    }
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct FileMode: u16 {
        const FILE_TYPE_MASK   = 0b11110000_00000000;
        const SOCKET           = 0b11000000_00000000;
        const REGULAR_FILE     = 0b10000000_00000000;
        const DIRECTORY        = 0b01000000_00000000;
        const SYMBOLIC_LINK    = 0b10100000_00000000;
        const CHARACTER_DEVICE = 0b00100000_00000000;
        const BLOCK_DEVICE     = 0b01100000_00000000;
        const FIFO             = 0b00010000_00000000;

        const SET_USER_ID      = 0b00001000_00000000;
        const SET_GROUP_ID     = 0b00000100_00000000;
        const STICKY           = 0b00000010_00000000;

        const USER_READ        = 0b00000001_00000000;
        const USER_WRITE       = 0b00000000_10000000;
        const USER_EXECUTE     = 0b00000000_01000000;
        const GROUP_READ       = 0b00000000_00100000;
        const GROUP_WRITE      = 0b00000000_00010000;
        const GROUP_EXECUTE    = 0b00000000_00001000;
        const OTHERS_READ      = 0b00000000_00000100;
        const OTHERS_WRITE     = 0b00000000_00000010;
        const OTHERS_EXECUTE   = 0b00000000_00000001;
        const USER_RWX         = 0b00000001_11000000;
        const GROUP_RWX        = 0b00000000_00111000;
        const OTHERS_RWX       = 0b00000000_00000111;
    }
}

impl FileMode {
    pub fn is_regular_file(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::REGULAR_FILE
    }

    pub fn is_directory(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::DIRECTORY
    }

    pub fn is_symbolic_link(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::SYMBOLIC_LINK
    }

    pub fn is_managed(self) -> bool {
        self.is_regular_file() || self.is_directory() || self.is_symbolic_link()
    }

    pub fn is_block_device(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::BLOCK_DEVICE
    }

    pub fn is_character_device(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::CHARACTER_DEVICE
    }

    pub fn is_device(self) -> bool {
        self.is_block_device() || self.is_character_device()
    }

    pub fn is_fifo(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::FIFO
    }

    pub fn is_socket(self) -> bool {
        (self & FileMode::FILE_TYPE_MASK) == FileMode::SOCKET
    }
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct Flags: u32 {
        const ACCESS_MODE_MASK = 0b00000000_00000011;
        const READ_ONLY        = 0b00000000_00000000;
        const WRITE_ONLY       = 0b00000000_00000001;
        const READ_WRITE       = 0b00000000_00000010;

        const APPEND           = 0b00000100_00000000;
    }
}

pub mod block;
pub mod device;
pub mod file;
pub mod ops;
pub mod replies;

#[cfg(test)]
mod tests {}
