//! Attention! This filesystem cannot run properly
//! in a multi-threaded environment!
#![feature(nll)]
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate failure;
extern crate bincode;
#[macro_use]
extern crate nix;
#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_try;
extern crate im;

use failure::Error;
use slog::Logger;
use std::cmp::min;
use std::ffi::{OsStr, OsString};
use std::fs::*;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::ops::Drop;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

pub mod block;
pub mod device;
pub mod file;

use block::FreeData;
use block::FreeInode;
use block::{Block, Inode, SuperBlock};
use device::Device;
use file::DkFile;
use std::cell::Ref;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;

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
}

impl Donkey {
    pub fn open<P: AsRef<Path>>(dev_path: P) -> DkResult<Handle> {
        let mut dev = device::open(dev_path)?;
        let sb = SuperBlock::from_bytes(dev.read_at(SUPER_BLOCK_PTR)?)?;
        let dk = Donkey { dev, sb };
        Ok(Handle::new(dk))
    }

    pub fn format<P: AsRef<Path>>(dev_path: P, opts: FormatOptions) -> DkResult<Handle> {
        let mut dev = device::open(dev_path)?;

        let block_size = dev.block_size();
        let inode_count = dev.size() / opts.bytes_per_inode;
        let first_data_ptr = Donkey::first_data_ptr(inode_count, block_size);

        // No plan to implement a real boot block here.

        // Make the initial super block
        let sb = SuperBlock {
            magic_number: block::MAGIC_NUMBER,
            block_size,
            inode_count,
            used_inode_count: 0,
            data_count: dev.block_count() - first_data_ptr / block_size,
            used_data_count: 0,
            free_inode_ptr: FIRST_INODE_PTR,
            free_data_ptr: first_data_ptr,
        };
        dev.write_block_at(&sb, SUPER_BLOCK_PTR)?;

        // Make the initial free inode
        let fi = FreeInode {
            next_free_ptr: 0,
            free_count: inode_count,
        };
        dev.write_block_at(&fi, FIRST_INODE_PTR)?;

        // Make the initial free data block
        let fb = FreeData {
            next_free_ptr: 0,
            free_count: sb.data_count,
        };
        dev.write_block_at(&fb, first_data_ptr)?;

        let dk = Donkey { dev, sb };
        let handle = Handle::new(dk);
        Donkey::create_root(handle.clone())?;
        Ok(handle)
    }

    /// We take care of block alignment here in case when
    /// the device itself is well aligned.
    fn first_data_ptr(inode_count: u64, block_size: u64) -> u64 {
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

// Below are macros used in `impl Handle` to reduce characters.

// Read a block
macro_rules! rb {
    ($h:expr, $ptr:expr, $b:ty) => {
        <$b as Block>::from_bytes($h.borrow_mut().dev.read_at($ptr)?)
    };
}

// Write a block
macro_rules! wb {
    ($h:expr, $b:expr, $ptr:expr) => {
        $h.borrow_mut().dev.write_block_at(&$b, $ptr)
    };
}

// Access the super block
macro_rules! sb {
    ($h:expr) => {
        $h.borrow().sb
    };
    ($h:expr, mut) => {
        $h.borrow_mut().sb
    };
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

    fn flush_sb(&self) -> DkResult<()> {
        wb!(self, sb!(self), SUPER_BLOCK_PTR)
    }

    fn allocate_inode(&self) -> DkResult<u64> {
        let fi_ptr = sb!(self).free_inode_ptr;
        let fi = rb!(self, fi_ptr, FreeInode)?;
        if fi.free_count > 1 {
            // Split this free inode
            let new_fi = FreeInode {
                free_count: fi.free_count - 1,
                next_free_ptr: fi.next_free_ptr,
            };
            let new_fi_ptr = fi_ptr + INODE_SIZE;
            wb!(self, new_fi, new_fi_ptr)?;
        } else if fi.free_count == 1 {
            sb!(self, mut).free_inode_ptr = fi.next_free_ptr;
        } else {
            return Err(format_err!(
                "Bad free inode, free count = {}.",
                fi.free_count
            ));
        }
        sb!(self, mut).used_inode_count += 1;
        self.flush_sb()?;
        Ok(Inode::ino(fi_ptr))
    }

    fn read_inode(&self, ino: u64) -> DkResult<Inode> {
        rb!(self, ino, Inode)
    }

    fn write_inode(&self, inode: &Inode) -> DkResult<()> {
        wb!(self, *inode, inode.ptr())
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
        self.link(ino, ino, OsStr::new("."))?;
        self.link(parent_ino, ino, OsStr::new(".."))?;
        Ok(ino)
    }

    // TODO? Cannot handle same filename!
    pub fn link(&self, ino: u64, parent_ino: u64, name: &OsStr) -> DkResult<()> {
        // {
        //     let mut dir = self.open(parent_ino, Flags::WRITE_ONLY | Flags::APPEND)?;
        //     let entry = DirectoryEntry::new(ino, name);
        //     let buf = bincode::serialize(&entry)?;
        //     dir.write_all(&buf)?;
        // }

        // let mut file = self.open(ino, Flags::WRITE_ONLY)?;
        // file.set_attr(SetFileAttr::new().nlink_inc(1))?;
        // Ok(())
        unimplemented!()
    }

    pub fn open(&self, ino: u64, flags: Flags) -> DkResult<DkFile> {
        let f = DkFile {
            handle: self.clone(),
            inode: self.read_inode(ino)?,
            pos: 0,
            flags,
            dirty: false,
        };
        // TODO
        // Use data structures to record opens
        Ok(f)
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

#[cfg(test)]
mod tests {}
