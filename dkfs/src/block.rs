use super::*;
use bincode::{deserialize_from, serialize};
use byteorder::{ByteOrder, LE};
use std::fmt::Debug;
use std::io::{self, BufReader, Read};
use std::ops::{Deref, DerefMut, Index, IndexMut};

pub trait Readable {
    /// Do necessary validation.
    /// Used in `from_bytes` after deserialization.
    fn validate(&self) -> DkResult<()> {
        Ok(())
    }

    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized;
}

pub trait Writable: Debug {
    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>>;
}

impl Writable for Box<Writable> {
    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        self.deref().as_bytes()
    }
}

pub(crate) const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct SuperBlock {
    pub(crate) magic_number: u64,
    pub(crate) block_size: u64,
    pub(crate) inode_count: u64,
    pub(crate) used_inode_count: u64,
    pub(crate) db_count: u64,
    pub(crate) used_db_count: u64,
    pub(crate) inode_fl_ptr: u64,
    pub(crate) db_fl_ptr: u64,
}

/// super block validation
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
pub struct FreeList {
    pub next_ptr: u64,
    /// Size of this free node
    pub size: u64,
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
    pub blocks: u64,
    /// valid for device special files
    pub device: u64,
    pub xattr_ptr: u64,
    pub ptrs: InodePtrs,
}

/// inode validation
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

    pub fn ptr(ino: u64) -> u64 {
        (ino - ROOT_INODE) * INODE_SIZE + FIRST_INODE_PTR
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct InodePtrs([u64; 12], [u64; 1], [u64; 1], [u64; 1], [u64; 1]);

// impl Index<u32> for InodePtrs {
//     type Output = [u64];

//     fn index(&self, index: u32) -> &[u64] {
//         match index {
//             0 => &self.0,
//             1 => &self.1,
//             2 => &self.2,
//             3 => &self.3,
//             4 => &self.4,
//             _ => unreachable!(),
//         }
//     }
// }

impl Index<usize> for InodePtrs {
    type Output = [u64];

    fn index(&self, index: usize) -> &[u64] {
        match index {
            0 => &self.0,
            1 => &self.1,
            2 => &self.2,
            3 => &self.3,
            4 => &self.4,
            _ => unreachable!(),
        }
    }
}

impl IndexMut<usize> for InodePtrs {
    fn index_mut(&mut self, index: usize) -> &mut [u64] {
        match index {
            0 => &mut self.0,
            1 => &mut self.1,
            2 => &mut self.2,
            3 => &mut self.3,
            4 => &mut self.4,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Data<T>(pub Vec<T>);

impl<T> Deref for Data<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T> DerefMut for Data<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.0
    }
}

pub type ByteData = Data<u8>;
pub type PtrBlock = Data<u64>;

#[derive(Debug)]
pub struct RefData<'a>(pub &'a [u8]);

macro_rules! impl_block {
    ($b:ty$(; validation: $f:ident)*) => {
        impl Readable for $b {
            fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
            where
                Self: Sized,
            {
                let b: Self = deserialize_from(bytes)?;
                b.validate()?;
                Ok(b)
            }

            $(
            fn validate(&self) -> DkResult<()> {
                $f(self)
            }
            )*
        }

        impl Writable for $b {
            fn as_bytes(&self) -> DkResult<Box<Deref<Target = [u8]>>> {
                Ok(Box::new(serialize(&self)?))
            }
        }
    };
}

impl_block!(SuperBlock; validation: sbv);
impl_block!(FreeList);
impl_block!(Inode; validation: inv);

impl Readable for ByteData {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized,
    {
        let v: Result<Vec<u8>, io::Error> = bytes.bytes().collect();
        Ok(Data(v?))
    }
}

impl Writable for ByteData {
    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        Ok(Box::new(&self[..]))
    }
}

impl Readable for PtrBlock {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized,
    {
        let read = BufReader::new(bytes);
        let bytes = read.bytes().collect::<Result<Vec<u8>, io::Error>>()?;
        let mut v = vec![0; bytes.len() / 8];
        LE::read_u64_into(&bytes[..], &mut v[..]);
        Ok(Data(v))
    }
}

impl Writable for PtrBlock {
    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        let mut v = vec![0; self.len() * 8];
        LE::write_u64_into(&self[..], &mut v[..]);
        Ok(Box::new(v))
    }
}

impl<'b> Writable for RefData<'b> {
    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        Ok(Box::new(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_ptr_block() -> DkResult<()> {
        let pb: PtrBlock = Data((512..1024).collect());
        let bytes: Vec<u8> = pb.as_bytes()?.iter().map(|&x| x).collect();
        let pb2 = PtrBlock::from_bytes(&bytes[..])?;
        assert_eq!(pb, pb2);
        Ok(())
    }
}
