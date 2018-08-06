use bincode::{deserialize_from, serialize};
use std::io::{Read, self};
use {DkResult, DkTimespec, FileMode};
use std::ops::Deref;

pub trait Block {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
        where
            Self: Sized;

    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target=[u8]> + 'a>>;
}

#[derive(Debug, Serialize, Deserialize)]
struct SuperBlock {
    magic_number: u64,
    block_size: u64,
    inode_count: u64,
    used_inode_count: u64,
    data_count: u64,
    used_data_count: u64,
    free_inode_ptr: u64,
    free_data_ptr: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct FreeInode {
    ptr: u64,
    next_free_ptr: u64,
    /// Number of continuous free inodes counting from `ptr`
    free_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Inode {
    mode: FileMode,
    uid: u32,
    gid: u32,
    nlink: u64,
    atime: DkTimespec,
    mtime: DkTimespec,
    ctime: DkTimespec,
    crtime: DkTimespec,
    /// valid for non-device files
    size: u64,
    /// valid for device special files
    device: u64,
    ptrs: InodePtrs,
}

#[derive(Debug, Serialize, Deserialize)]
struct InodePtrs {
    direct_ptrs: [u64; 12],
    indirect_ptr: u64,
    double_indirect_ptr: u64,
    triple_indirect_ptr: u64,
    quadruple_indirect_ptr: u64,
}

#[derive(Debug)]
struct Data(Vec<u8>);

macro_rules! impl_block {
    ($b:ty) => {
        impl Block for $b {
            fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
            where
                Self: Sized,
            {
                Ok(deserialize_from(bytes)?)
            }

            fn as_bytes(&self) -> DkResult<Box<Deref<Target=[u8]>>> {
                Ok(Box::new(serialize(&self)?))
            }
        }
    };
}

impl_block!(SuperBlock);
impl_block!(FreeInode);
impl_block!(Inode);

impl Block for Data {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self> where
        Self: Sized {
        let v: Result<Vec<u8>, io::Error> = bytes.bytes().collect();
        Ok(Data(v?))
    }

    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target=[u8]> + 'a>> {
        Ok(Box::new(&self.0[..]))
    }
}