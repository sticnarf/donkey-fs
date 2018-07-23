#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate failure_derive;
extern crate bincode;
#[macro_use]
extern crate nix;
#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_try;

use slog::Logger;
use std::fs::*;
use std::io::{self, Seek, SeekFrom};
use std::mem::size_of;
use std::path::Path;

pub const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;
pub const BOOT_BLOCK_SIZE: u64 = 1024;
pub const SUPER_BLOCK_SIZE: u64 = 1024;
pub const INODE_SIZE: u64 = 256;
pub const BLOCK_SIZE: u64 = 4096;
pub const DEFAULT_BYTES_PER_INODE: u64 = 16384;
pub const DEFAULT_BYTES_PER_INODE_STR: &'static str = "16384";

pub struct DonkeyBuilder {
    dev: File,
}

pub struct Donkey {
    dev: File,
    super_block: SuperBlock,
}

impl Donkey {
    pub fn create<P: AsRef<Path>>(dev_path: P) -> Result<DonkeyBuilder, DonkeyError> {
        let dev = OpenOptions::new().read(true).write(true).open(dev_path)?;
        Ok(DonkeyBuilder { dev })
    }
}

impl DonkeyBuilder {
    fn read_super_block(&mut self) -> Result<SuperBlock, ReadBlockError> {
        self.dev.seek(SeekFrom::Start(BOOT_BLOCK_SIZE))?;
        let super_block: SuperBlock = bincode::deserialize_from(&mut self.dev)?;

        // validate magic number
        if super_block.magic_number != MAGIC_NUMBER {
            Err(ReadBlockError::CorruptedBlockError(format_err!(
                "Maybe this device is not using Donkey?"
            )))
        } else {
            Ok(super_block)
        }
    }

    pub fn open(mut self) -> Result<Donkey, DonkeyError> {
        let super_block = self.read_super_block()?;
        Ok(Donkey {
            dev: self.dev,
            super_block,
        })
    }

    pub fn format(
        mut self,
        opts: &FormatOptions,
        log: Option<Logger>,
    ) -> Result<Donkey, DonkeyError> {
        let dev_size = dev_size(&self.dev, log.clone())?;
        let inode_count = dev_size / opts.bytes_per_inode;
        let data_block_count =
            (dev_size - BOOT_BLOCK_SIZE - SUPER_BLOCK_SIZE - inode_count * INODE_SIZE) / BLOCK_SIZE;

        try_info!(log, "Device size: {} bytes", dev_size);
        try_info!(log, "Inode count: {}", inode_count);
        try_info!(log, "Data block count: {}", data_block_count);

        self.dev.seek(SeekFrom::Start(0))?; // Dummy
        make_boot_block(&mut self.dev, log.clone())?;

        self.dev.seek(SeekFrom::Start(BOOT_BLOCK_SIZE))?;
        make_super_block(&mut self.dev, inode_count, data_block_count, log.clone())?;

        self.dev
            .seek(SeekFrom::Start(BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE))?;
        make_inodes(&mut self.dev, inode_count, log.clone())?;

        self.dev.seek(SeekFrom::Start(
            BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE + inode_count * INODE_SIZE,
        ))?;
        make_data_blocks(&mut self.dev, data_block_count, log.clone())?;

        self.open()
    }
}

fn make_boot_block(dev: &mut File, log: Option<Logger>) -> Result<(), FormatError> {
    try_info!(log, "Making the boot block...");
    let boot_block = BootBlock::init();
    bincode::serialize_into(dev, &boot_block)?;
    Ok(())
}

fn make_super_block(
    dev: &mut File,
    inode_count: u64,
    data_block_count: u64,
    log: Option<Logger>,
) -> Result<(), FormatError> {
    try_info!(log, "Making the super block...");
    let super_block = SuperBlock::init(inode_count, data_block_count);
    bincode::serialize_into(dev, &super_block)?;
    Ok(())
}

fn make_inodes(dev: &mut File, inode_count: u64, log: Option<Logger>) -> Result<(), FormatError> {
    try_info!(log, "Making inodes...");
    let init_inode = Inode::init(inode_count);
    bincode::serialize_into(dev, &init_inode)?;
    Ok(())
}

fn make_data_blocks(
    dev: &mut File,
    data_block_count: u64,
    log: Option<Logger>,
) -> Result<(), FormatError> {
    try_info!(log, "Making data blocks...");
    let init_data_block = DataBlock::init(data_block_count);
    let free_data_block = unsafe { init_data_block.free };
    bincode::serialize_into(dev, &free_data_block)?;
    Ok(())
}

fn dev_size(dev: &File, log: Option<Logger>) -> Result<u64, FormatError> {
    let metadata = dev.metadata()?;
    let file_type = metadata.file_type();

    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    if file_type.is_file() {
        try_info!(log, "Regular file detected. Treat it as an image file.");
        Ok(metadata.size())
    } else if file_type.is_block_device() {
        try_info!(log, "Block device detected.");
        block_dev_size(&dev, log)
    } else {
        Err(FormatError::UnsupportedDeviceError.into())
    }
}

// #[cfg(target_os = "linux")]
fn block_dev_size(dev: &File, _log: Option<Logger>) -> Result<u64, FormatError> {
    use std::os::unix::io::{AsRawFd, RawFd};
    let fd = dev.as_raw_fd();

    #[cfg(target_os = "linux")]
    fn getsize(fd: RawFd) -> Result<u64, FormatError> {
        // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
        ioctl_read!(getsize64, 0x12, 114, u64);
        let mut size: u64 = 0;
        unsafe {
            getsize64(fd, &mut size)?;
        }
        Ok(size)
    }

    #[cfg(target_os = "macos")]
    fn getsize(fd: RawFd) -> Result<u64, FormatError> {
        // https://github.com/apple/darwin-xnu/blob/xnu-4570.1.46/bsd/sys/disk.h#L203
        ioctl_read!(getblksize, b'd', 24, u32);
        ioctl_read!(getblkcount, b'd', 25, u64);
        let mut blksize: u32 = 0;
        let mut blkcount: u64 = 0;
        unsafe {
            getblksize(fd, &mut blksize)?;
            getblkcount(fd, &mut blkcount)?;
        }
        Ok(blksize as u64 * blkcount)
    }

    #[cfg(target_os = "freebsd")]
    fn getsize(fd: RawFd) -> Result<u64, FormatError> {
        // https://github.com/freebsd/freebsd/blob/stable/11/sys/sys/disk.h#L37
        ioctl_read!(getmediasize, b'd', 129, u64);
        let mut size: u64 = 0;
        unsafe {
            getmediasize(fd, &mut size)?;
        }
        Ok(size)
    }

    getsize(fd)
}

pub struct FormatOptions {
    bytes_per_inode: u64,
}

impl FormatOptions {
    pub fn new() -> Self {
        FormatOptions {
            bytes_per_inode: DEFAULT_BYTES_PER_INODE,
        }
    }

    pub fn bytes_per_inode(mut self, bytes_per_inode: u64) -> Self {
        self.bytes_per_inode = bytes_per_inode;
        self
    }
}

// A boot block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
pub struct BootBlock {}

const_assert!(boot_block; (size_of::<BootBlock>() as u64) <= BOOT_BLOCK_SIZE);

impl BootBlock {
    pub fn init() -> Self {
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

impl SuperBlock {
    pub fn init(inode_count: u64, data_block_count: u64) -> Self {
        SuperBlock {
            magic_number: MAGIC_NUMBER,
            inode_count,
            data_block_count,
            ..Default::default()
        }
    }
}

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

impl Inode {
    pub fn init(inode_count: u64) -> Self {
        Inode::FreeInode {
            free_count: inode_count,
            next_free: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct FreeDataBlock {
    pub free_count: u64,
    pub next_free: u64,
}

pub union DataBlock {
    pub data: [u8; 4096],
    pub ptrs: [u64; 512],
    pub free: FreeDataBlock,
}

const_assert!(data_block; (size_of::<DataBlock>() as u64) <= BLOCK_SIZE);

impl DataBlock {
    pub fn init(data_block_count: u64) -> Self {
        DataBlock {
            free: FreeDataBlock {
                free_count: data_block_count,
                next_free: 0,
            },
        }
    }
}

#[derive(Debug, Fail)]
pub enum DonkeyError {
    #[fail(display = "OS error: {}", e)]
    OsError {
        #[cause]
        e: io::Error,
    },
    #[fail(display = "Read block error: {}", e)]
    ReadBlockError {
        #[cause]
        e: ReadBlockError,
    },
    #[fail(display = "Error occurred when formatting: {}", e)]
    FormatError {
        #[cause]
        e: FormatError,
    },
}

impl From<io::Error> for DonkeyError {
    fn from(e: io::Error) -> DonkeyError {
        DonkeyError::OsError { e }
    }
}

impl From<ReadBlockError> for DonkeyError {
    fn from(e: ReadBlockError) -> DonkeyError {
        DonkeyError::ReadBlockError { e }
    }
}

impl From<FormatError> for DonkeyError {
    fn from(e: FormatError) -> DonkeyError {
        DonkeyError::FormatError { e }
    }
}

#[derive(Debug, Fail)]
pub enum ReadBlockError {
    #[fail(display = "OS error, {}", e)]
    OsError {
        #[cause]
        e: io::Error,
    },
    #[fail(display = "Deserialization error, {}", e)]
    DeserializeBlockError {
        #[cause]
        e: bincode::Error,
    },
    #[fail(display = "Block is corrupted, {}", _0)]
    CorruptedBlockError(failure::Error),
}

impl From<io::Error> for ReadBlockError {
    fn from(e: io::Error) -> ReadBlockError {
        ReadBlockError::OsError { e }
    }
}

impl From<bincode::Error> for ReadBlockError {
    fn from(e: bincode::Error) -> ReadBlockError {
        ReadBlockError::DeserializeBlockError { e }
    }
}

#[derive(Debug, Fail)]
pub enum FormatError {
    #[fail(display = "OS error, {}", e)]
    OsError {
        #[cause]
        e: io::Error,
    },
    #[fail(display = "Serialization error, {}", e)]
    SerializeBlockError {
        #[cause]
        e: bincode::Error,
    },
    #[fail(display = "Ioctl error, {}", e)]
    IoctlError {
        #[cause]
        e: nix::Error,
    },
    #[fail(display = "The device is not supported.")]
    UnsupportedDeviceError,
}

impl From<io::Error> for FormatError {
    fn from(e: io::Error) -> FormatError {
        FormatError::OsError { e }
    }
}

impl From<bincode::Error> for FormatError {
    fn from(e: bincode::Error) -> FormatError {
        FormatError::SerializeBlockError { e }
    }
}

impl From<nix::Error> for FormatError {
    fn from(e: nix::Error) -> FormatError {
        FormatError::IoctlError { e }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
