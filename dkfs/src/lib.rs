//! Attention! This filesystem cannot run properly
//! in a multi-threaded environment!

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

use block::{Block, SuperBlock};
use device::Device;
use std::cell::Ref;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use block::FreeInode;
use block::FreeData;

const BOOT_BLOCK_SIZE: u64 = 1024;
const SUPER_BLOCK_SIZE: u64 = 1024;
const INODE_SIZE: u64 = 256;
const BOOT_BLOCK_PTR: u64 = 0;
const SUPER_BLOCK_PTR: u64 = BOOT_BLOCK_PTR + BOOT_BLOCK_SIZE;
const FIRST_INODE_PTR: u64 = SUPER_BLOCK_PTR + SUPER_BLOCK_SIZE;

pub const DEFAULT_BYTES_PER_INODE: u64 = 16384;
pub const DEFAULT_BYTES_PER_INODE_STR: &'static str = "16384";
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
        Ok(Handle(Rc::new(RefCell::new(dk))))
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
        Ok(Handle(Rc::new(RefCell::new(dk))))
    }

    /// We take care of block alignment here in case when
    /// the device itself is well aligned.
    fn first_data_ptr(inode_count: u64, block_size: u64) -> u64 {
        let used_blocks = (FIRST_INODE_PTR + INODE_SIZE * inode_count + block_size - 1) / block_size;
        used_blocks * block_size
    }
}

#[derive(Clone)]
pub struct Handle(Rc<RefCell<Donkey>>);

impl Handle {}

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    bytes_per_inode: u64,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE
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

#[cfg(test)]
mod tests {}
