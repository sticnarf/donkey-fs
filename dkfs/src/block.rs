use bincode::{deserialize_from, serialize};
use std::io::{self, Read};
use std::ops::Deref;
use {DkResult, DkTimespec, FileMode};
use device::Device;

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
    pub magic_number: u64,
    pub block_size: u64,
    pub inode_count: u64,
    pub used_inode_count: u64,
    pub data_count: u64,
    pub used_data_count: u64,
    pub free_inode_ptr: u64,
    pub free_data_ptr: u64,
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
    pub next_free_ptr: u64,
    /// Number of continuous free inodes counting from this
    pub free_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Inode {
    pub ino: u64,
    pub mode: FileMode,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub atime: DkTimespec,
    pub mtime: DkTimespec,
    pub ctime: DkTimespec,
    pub crtime: DkTimespec,
    /// valid for non-device files
    pub size: u64,
    /// valid for device special files
    pub device: u64,
    pub ptrs: InodePtrs,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InodePtrs {
    pub direct_ptrs: [u64; 12],
    pub indirect_ptr: u64,
    pub double_indirect_ptr: u64,
    pub triple_indirect_ptr: u64,
    pub quadruple_indirect_ptr: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FreeData {
    pub next_free_ptr: u64,
    /// Number of continuous free data blocks counting from this
    pub free_count: u64,
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
impl_block!(Inode);
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
