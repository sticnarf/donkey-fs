use super::*;
use bincode::{deserialize_from, serialize};
use std::io::{self, Read};
use std::ops::{Deref, Index, IndexMut};

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
    pub(crate) db_count: u64,
    pub(crate) used_db_count: u64,
    pub(crate) inode_fl_ptr: u64,
    pub(crate) db_fl_ptr: u64,
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
pub struct FreeList {
    pub(crate) next_ptr: u64,
    /// Size of this free node
    pub(crate) size: u64,
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
pub struct InodePtrs([u64; 12], [u64; 1], [u64; 1], [u64; 1], [u64; 1]);

impl Index<u32> for InodePtrs {
    type Output = [u64];

    fn index(&self, index: u32) -> &[u64] {
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

impl IndexMut<u32> for InodePtrs {
    fn index_mut(&mut self, index: u32) -> &mut [u64] {
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

impl InodePtrs {
    /// Given the position of the file and the block size,
    /// returns the level and the offset
    pub fn locate(&self, mut pos: u64, bs: u64) -> (u32, u64) {
        let pc = bs / 8; // Pointer count in a single block
        let mut b = pos / bs; // index of block at pos
        let mut sz = bs; // Size of all blocks through the direct or indirect pointer
        for i in 0..5 {
            let len = self[i].len() as u64;
            if b < len {
                return (i, pos);
            }
            b = (b - len) / pc;
            pos -= len * sz;
            sz *= pc;
        }
        unreachable!()
    }
}

#[derive(Debug)]
pub struct DataBlock(pub Vec<u8>);

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
impl_block!(FreeList);
impl_block!(Inode; validation: inv);

impl Block for DataBlock {
    fn from_bytes<R: Read>(bytes: R) -> DkResult<Self>
    where
        Self: Sized,
    {
        let v: Result<Vec<u8>, io::Error> = bytes.bytes().collect();
        Ok(DataBlock(v?))
    }

    fn as_bytes<'a>(&'a self) -> DkResult<Box<Deref<Target = [u8]> + 'a>> {
        Ok(Box::new(&self.0[..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inode_ptrs_locate() {
        let p = InodePtrs::default();
        assert_eq!(p.locate(0, 4096), (0, 0));
        assert_eq!(p.locate(29906, 4096), (0, 29906));
        assert_eq!(p.locate(49152, 4096), (1, 0));
        assert_eq!(p.locate(60554, 4096), (1, 11402));
        assert_eq!(p.locate(2146304, 4096), (2, 0));
        assert_eq!(p.locate(1075888127, 4096), (2, 1073741823));
        assert_eq!(p.locate(1075888128, 4096), (3, 0));
        assert_eq!(p.locate(550831702015, 4096), (3, 549755813887));
        assert_eq!(p.locate(550831702016, 4096), (4, 0));
        assert_eq!(p.locate(282025808412671, 4096), (4, 281474976710655));
    }
}
