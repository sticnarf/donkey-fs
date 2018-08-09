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
#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_try;
extern crate byteorder;
extern crate im;

use failure::Error;
use slog::Logger;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use block::*;
use device::Device;
use file::{DkDir, DkDirHandle, DkFile, DkFileHandle};
use std::cell::RefCell;
use std::collections::hash_map::{self, HashMap};
use std::ops::Deref;
use std::rc::{Rc, Weak};

const BOOT_BLOCK_SIZE: u64 = 1024;
const SUPER_BLOCK_SIZE: u64 = 1024;
const INODE_SIZE: u64 = 256;
const BOOT_BLOCK_PTR: u64 = 0;
const SUPER_BLOCK_PTR: u64 = BOOT_BLOCK_PTR + BOOT_BLOCK_SIZE;
const FIRST_INODE_PTR: u64 = SUPER_BLOCK_PTR + SUPER_BLOCK_SIZE;

pub const DEFAULT_BYTES_PER_INODE: u64 = 16384;
pub const DEFAULT_BYTES_PER_INODE_STR: &'static str = "16384";
/// This cannot be a very small integer. Inode numbers of
/// small integers are reserved for special use.
pub const ROOT_INODE: u64 = 114514;

pub type DkResult<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Donkey {
    dev: Box<Device>,
    sb: SuperBlock,
    opened_files: HashMap<u64, Weak<RefCell<DkFile>>>,
    opened_dirs: HashMap<u64, Weak<RefCell<DkDir>>>,
}

impl Donkey {
    pub fn open<P: AsRef<Path>>(dev_path: P) -> DkResult<Handle> {
        let mut dev = device::open(dev_path)?;
        let sb = SuperBlock::from_bytes(dev.read_at(SUPER_BLOCK_PTR)?)?;
        let dk = Donkey::new(dev, sb);
        Ok(Handle::new(dk))
    }

    pub fn format<P: AsRef<Path>>(dev_path: P, opts: FormatOptions) -> DkResult<Handle> {
        let mut dev = device::open(dev_path)?;

        let block_size = dev.block_size();
        let inode_count = dev.size() / opts.bytes_per_inode;
        let first_db_ptr = Donkey::first_db_ptr(inode_count, block_size);

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

        let dk = Donkey::new(dev, sb);
        let handle = Handle::new(dk);
        Donkey::create_root(handle.clone())?;
        Ok(handle)
    }

    fn new(dev: Box<Device>, sb: SuperBlock) -> Self {
        Donkey {
            dev,
            sb,
            opened_files: HashMap::new(),
            opened_dirs: HashMap::new(),
        }
    }

    /// We take care of block alignment here in case when
    /// the device itself is well aligned.
    fn first_db_ptr(inode_count: u64, block_size: u64) -> u64 {
        let used_blocks =
            (FIRST_INODE_PTR + INODE_SIZE * inode_count + block_size - 1) / block_size;
        used_blocks * block_size
    }

    /// This function is only called in `format`
    /// because we assume root inode is not allocated yet.
    fn create_root(handle: Handle) -> DkResult<()> {
        let perm = FileMode::USER_RWX
            | FileMode::GROUP_READ
            | FileMode::GROUP_EXECUTE
            | FileMode::OTHERS_READ
            | FileMode::OTHERS_EXECUTE;
        let root_inode = handle.mkdir_raw(ROOT_INODE, perm, 0, 0)?;

        if root_inode == ROOT_INODE {
            Ok(())
        } else {
            Err(format_err!(
                "Expected root inode number to be {}, but got {}.",
                ROOT_INODE,
                root_inode
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub struct Handle {
    inner: Rc<RefCell<Donkey>>,
    log: Option<Logger>,
}

impl Deref for Handle {
    type Target = RefCell<Donkey>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Handle {
    fn new(dk: Donkey) -> Self {
        Handle {
            inner: Rc::new(RefCell::new(dk)),
            log: None,
        }
    }

    pub fn log(&self, log: Logger) -> Self {
        Handle {
            inner: self.inner.clone(),
            log: Some(log),
        }
    }

    fn read_into(&self, ptr: u64, mut dst: &mut [u8]) -> DkResult<u64> {
        Ok(io::copy(
            &mut self.borrow_mut().dev.read_len_at(ptr, dst.len() as u64)?,
            &mut dst,
        )?)
    }

    fn read<T: Readable>(&self, ptr: u64) -> DkResult<T> {
        <T as Readable>::from_bytes(self.borrow_mut().dev.read_at(ptr)?)
    }

    fn write(&self, ptr: u64, writable: &Writable) -> DkResult<()> {
        self.borrow_mut().dev.write_at(writable, ptr)
    }

    fn block_size(&self) -> u64 {
        self.borrow().sb.block_size
    }

    fn flush_sb(&self) -> DkResult<()> {
        self.write(SUPER_BLOCK_PTR, &self.borrow().sb)
    }

    /// Allocated a block of size `size` from `FreeList` at `ptr`.
    /// Returns the pointer of the allocated block and the pointer
    /// of the new `FreeList`.
    fn allocate_from_free(&self, ptr: u64, size: u64) -> DkResult<(u64, u64)> {
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
    fn allocate_inode(&self) -> DkResult<u64> {
        let sb = &self.borrow().sb;
        if sb.used_inode_count < sb.inode_count {
            let fl_ptr = self.borrow().sb.inode_fl_ptr;
            let (fi_ptr, new_fl_ptr) = self.allocate_from_free(fl_ptr, INODE_SIZE)?;
            self.borrow_mut().sb.inode_fl_ptr = new_fl_ptr;
            self.borrow_mut().sb.used_inode_count += 1;
            self.flush_sb()?;
            Ok(Inode::ino(fi_ptr))
        } else {
            Err(format_err!("Inodes are used up!"))
        }
    }

    fn read_inode(&self, ino: u64) -> DkResult<Inode> {
        self.read(ino)
    }

    fn write_inode(&self, inode: &Inode) -> DkResult<()> {
        self.write(inode.ptr(), inode)
    }

    /// Returns the pointer of the allocated data block
    fn allocate_db(&self) -> DkResult<u64> {
        let sb = &self.borrow().sb;
        if sb.used_db_count < sb.db_count {
            let fl_ptr = self.borrow().sb.db_fl_ptr;
            let (fd_ptr, new_fl_ptr) = self.allocate_from_free(fl_ptr, self.block_size())?;
            self.borrow_mut().sb.db_fl_ptr = new_fl_ptr;
            self.borrow_mut().sb.used_db_count += 1;
            self.flush_sb()?;
            Ok(fd_ptr)
        } else {
            Err(format_err!("Data blocks are used up!"))
        }
    }

    fn fill_zero(&self, ptr: u64, size: u64) -> DkResult<()> {
        let v = vec![0u8; size as usize];
        let b = RefData(v.as_slice());
        self.write(ptr, &b)
    }

    /// Returns the inode number of the new node.
    pub fn mknod(
        &self,
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
            device: rdev.unwrap_or(0),
            ptrs: Default::default(),
        };
        self.write_inode(&inode)?;
        Ok(ino)
    }

    /// Returns the inode number of the new directory.
    /// This method **DOES NOT** link the new directory to
    /// the parent directory!
    fn mkdir_raw(&self, parent_ino: u64, perm: FileMode, uid: u32, gid: u32) -> DkResult<u64> {
        let mode = FileMode::DIRECTORY | perm;
        let ino = self.mknod(mode, uid, gid, 0, None)?;

        // Create `.` and `..` entry
        let dir = self.open_dir(ino)?;
        dir.borrow_mut().add_entry(OsStr::new("."), ino)?;
        dir.borrow_mut().add_entry(OsStr::new(".."), parent_ino)?;

        Ok(ino)
    }

    pub fn open_file(&self, ino: u64, flags: Flags) -> DkResult<DkFileHandle> {
        let inner = match self.borrow_mut().opened_files.entry(ino) {
            hash_map::Entry::Occupied(e) => {
                // We ensure that all `Weak`s in the map is valid,
                // so we simply unwrap here.
                e.get().upgrade().unwrap()
            }
            hash_map::Entry::Vacant(e) => {
                let inode = self.read_inode(ino)?;
                let f = DkFile::new(self.clone(), inode);
                let rc = Rc::new(RefCell::new(f));
                e.insert(Rc::downgrade(&rc));
                rc
            }
        };
        let handle = DkFileHandle { inner, flags };
        Ok(handle)
    }

    pub fn open_dir(&self, ino: u64) -> DkResult<DkDirHandle> {
        let inner = match self.borrow_mut().opened_dirs.entry(ino) {
            hash_map::Entry::Occupied(e) => {
                // We ensure that all `Weak`s in the map is valid,
                // so we simply unwrap here.
                e.get().upgrade().unwrap()
            }
            hash_map::Entry::Vacant(e) => {
                let fh = self.open_file(ino, Flags::READ_WRITE)?;
                let dir = DkDir::from_file(fh)?;
                let rc = Rc::new(RefCell::new(dir));
                e.insert(Rc::downgrade(&rc));
                rc
            }
        };
        let handle = DkDirHandle { inner };
        Ok(handle)
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

#[cfg(test)]
mod tests {}
