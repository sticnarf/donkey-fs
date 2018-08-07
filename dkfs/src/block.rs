use super::*;
use bincode::{deserialize_from, serialize};
use std::io::{self, Read};
use std::ops::Deref;

pub trait Block {
    /// Do necessary validation.
    /// Used in `from_bytes` after deserialization.
    fn validate(&self) -> DkResult<()> {
        Ok(())
    }

    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized;

    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>>;
}

pub const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;

#[derive(Debug, Serialize, Deserialize)]
pub struct SuperBlock {
    pub(crate) magic_number: u64,
    pub(crate) block_size: u64,
    pub(crate) inode_count: u64,
    pub(crate) used_inode_count: u64,
    pub(crate) data_count: u64,
    pub(crate) used_data_count: u64,
    pub(crate) free_inode_ptr: u64,
    pub(crate) free_data_ptr: u64,
}

/// Validates `SuperBlock`.
fn sbv(sb: &SuperBlock) -> DkResult<()> {
    if sb.magic_number != MAGIC_NUMBER {
        Err(format_err!(
            "Magic number validation failed! It is probably not using Donkey filesystem."
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FreeInode {
    pub(crate) next_free_ptr: u64,
    /// Number of continuous free inodes counting from this
    pub(crate) free_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Inode {
    pub(crate) ino: u64,
    pub(crate) mode: FileMode,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) nlink: u64,
    pub(crate) atime: DkTimespec,
    pub(crate) mtime: DkTimespec,
    pub(crate) ctime: DkTimespec,
    pub(crate) crtime: DkTimespec,
    /// valid for non-device files
    pub(crate) size: u64,
    /// valid for device special files
    pub(crate) device: u64,
    pub(crate) ptrs: InodePtrs,
}

fn inv(inode: &Inode) -> DkResult<()> {
    if inode.ino < ROOT_INODE {
        Err(format_err!(
            "Inode number {} is smaller than the root inode number {}",
            inode.ino,
            ROOT_INODE
        ))
    } else {
        Ok(())
    }
}

impl Inode {
    /// Converts ptr to inode number
    pub fn ino(ptr: u64) -> u64 {
        (ptr - FIRST_INODE_PTR) / INODE_SIZE + ROOT_INODE
    }

    pub fn ptr(&self) -> u64 {
        (self.ino - ROOT_INODE) * ::INODE_SIZE + FIRST_INODE_PTR
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct InodePtrs {
    pub(crate) direct_ptrs: [u64; 12],
    pub(crate) indirect_ptr: u64,
    pub(crate) double_indirect_ptr: u64,
    pub(crate) triple_indirect_ptr: u64,
    pub(crate) quadruple_indirect_ptr: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FreeData {
    pub(crate) next_free_ptr: u64,
    /// Number of continuous free data blocks counting from this
    pub(crate) free_count: u64,
}

#[derive(Debug)]
pub struct Data(Vec<u8>);

macro_rules! impl_block {
    ($b:ty$(; validation: $f:ident)*) => {
        impl Block for $b {
            fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
            where
                Self: Sized,
            {
                let b: Self = deserialize_from(bytes)?;
                b.validate()?;
                Ok(b)
            }

            fn as_bytes(&self) -> DkResult<Box<Deref<Target = [u8]>>> {
                Ok(Box::new(serialize(&self)?))
            }

            $(
            fn validate(&self) -> DkResult<()> {
                $f(self)
            }
            )*
        }
    };
}

impl_block!(SuperBlock; validation: sbv);
impl_block!(FreeInode);
impl_block!(Inode; validation: inv);
impl_block!(FreeData);

impl Block for Data {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized,
    {
        let v: Result<Vec<u8>, io::Error> = bytes.bytes().collect();
        Ok(Data(v?))
    }

    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        Ok(Box::new(&self.0[..]))
    }
}
