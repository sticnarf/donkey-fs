#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate static_assertions;

use std::fs::*;
use std::mem::size_of;

pub const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;
pub const BOOT_BLOCK_SIZE: u64 = 1024;
pub const SUPER_BLOCK_SIZE: u64 = 1024;
pub const INODE_SIZE: u64 = 256;
pub const BLOCK_SIZE: u64 = 4096;

pub struct DonkeyFS {
    pub dev: File,
    pub super_block: SuperBlock,
}

// A boot block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
pub struct BootBlock {}

const_assert!(boot_block; (size_of::<BootBlock>() as u64) <= BOOT_BLOCK_SIZE);

impl BootBlock {
    pub fn new() -> Self {
        BootBlock {}
    }
}

// A super block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
pub struct SuperBlock {
    pub magic_number: u64,
    pub inode_count: u64,
    pub used_inode_count: u64,
    pub data_block_count: u64,
    pub used_data_block_count: u64,
    pub root_inode_ptr: u64,
    pub free_inode_ptr: u64,
    pub free_block_ptr: u64,
}

const_assert!(super_block; (size_of::<SuperBlock>() as u64) <= SUPER_BLOCK_SIZE);

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct FileMode: u32 {
        const REGULAR_FILE = 0b00000001;
    }
}

#[derive(Serialize, Deserialize)]
pub struct TimeSpec {
    pub sec: i64,
    pub nsec: i64,
}

#[derive(Serialize, Deserialize)]
pub enum Inode {
    FreeInode {
        free_count: u64,
        next_free: u64,
    },
    UsedInode {
        mode: FileMode,
        uid: u32,
        gid: u32,
        link_count: u64,
        atime: TimeSpec,
        mtime: TimeSpec,
        ctime: TimeSpec,
        // file size for regular file, device number for device
        size_or_device: u64,
        direct_ptrs: [u64; 12],
        indirect_ptr: u64,
        double_indirect_ptr: u64,
        triple_indirect_ptr: u64,
        quadruple_indirect_ptr: u64,
    },
}

const_assert!(inode; (size_of::<Inode>() as u64) <= INODE_SIZE);

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct FreeDataBlock {
    pub free_count: u64,
    pub next_free: u64,
}

pub union DataBlock {
    _data: [u8; 4096],
    _ptrs: [u64; 512],
    _free: FreeDataBlock,
}

const_assert!(data_block; (size_of::<DataBlock>() as u64) <= BLOCK_SIZE);

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
