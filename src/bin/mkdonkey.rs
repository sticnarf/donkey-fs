#[macro_use]
extern crate clap;
#[macro_use]
extern crate slog;
extern crate failure;
extern crate slog_term;
#[macro_use]
extern crate failure_derive;
#[macro_use]
extern crate serde_derive;
extern crate bincode;
#[macro_use]
extern crate nix;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate static_assertions;

use slog::{Drain, Logger};
use std::fs::*;
use std::io::{self, Seek, SeekFrom};
use std::mem::size_of;

#[derive(Debug, Fail)]
enum MakefsError {
    #[fail(display = "OS error: {}", e)]
    OsError {
        #[cause]
        e: io::Error,
    },
    #[fail(display = "Serializing error: {}", e)]
    SerializeBlockError {
        #[cause]
        e: bincode::Error,
    },
    #[fail(display = "Ioctl error: {}", e)]
    IoctlError {
        #[cause]
        e: nix::Error,
    },
    #[fail(display = "The device is not supported.")]
    UnsupportedDeviceError,
}

impl From<io::Error> for MakefsError {
    fn from(e: io::Error) -> MakefsError {
        MakefsError::OsError { e }
    }
}

impl From<bincode::Error> for MakefsError {
    fn from(e: bincode::Error) -> MakefsError {
        MakefsError::SerializeBlockError { e }
    }
}

impl From<nix::Error> for MakefsError {
    fn from(e: nix::Error) -> MakefsError {
        MakefsError::IoctlError { e }
    }
}

const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;
const BOOT_BLOCK_SIZE: u64 = 1024;
const SUPER_BLOCK_SIZE: u64 = 1024;
const INODE_SIZE: u64 = 256;
const BLOCK_SIZE: u64 = 4096;
const DEFAULT_BYTES_PER_NODE_STR: &'static str = "16384";

fn main() {
    use clap::*;

    let matches = App::new("mkdonkey")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Make a donkey filesystem")
        .arg(
            Arg::with_name("device")
                .help("Path to the device to be used")
                .required(true),
        )
        .arg(
            Arg::with_name("bytes-per-inode")
                .help("Specify the bytes/inode ratio")
                .short("i")
                .takes_value(true)
                .default_value(DEFAULT_BYTES_PER_NODE_STR),
        )
        .get_matches();

    let log = logger();
    let dev_path = matches.value_of("device").unwrap();
    let bytes_per_inode =
        value_t!(matches.value_of("bytes-per-inode"), u64).unwrap_or_else(|e| e.exit());

    let opt = FsOptions {
        dev_path,
        bytes_per_inode,
    };

    if let Err(e) = mkfs(opt, log.clone()) {
        error!(log, "{}", e);
    }
}

struct FsOptions<'a> {
    dev_path: &'a str,
    bytes_per_inode: u64,
}

fn mkfs(opt: FsOptions, log: Logger) -> Result<(), MakefsError> {
    info!(log, "Trying to open {}", opt.dev_path);
    let mut dev = OpenOptions::new().write(true).open(opt.dev_path)?;
    let dev_size = dev_size(&dev, log.clone())?;
    let inode_count = dev_size / opt.bytes_per_inode;
    let data_block_count =
        (dev_size - BOOT_BLOCK_SIZE - SUPER_BLOCK_SIZE - inode_count * INODE_SIZE) / BLOCK_SIZE;

    info!(log, "Device size: {} bytes", dev_size);
    info!(log, "Inode count: {}", inode_count);
    info!(log, "Data block count: {}", data_block_count);

    dev.seek(SeekFrom::Start(0))?; // Dummy
    make_boot_block(&mut dev, log.clone())?;

    dev.seek(SeekFrom::Start(BOOT_BLOCK_SIZE))?;
    make_super_block(&mut dev, inode_count, data_block_count, log.clone())?;

    dev.seek(SeekFrom::Start(BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE))?;
    make_inodes(&mut dev, inode_count, log.clone())?;

    dev.seek(SeekFrom::Start(
        BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE + inode_count * INODE_SIZE,
    ))?;
    make_data_blocks(&mut dev, data_block_count, log.clone())?;
    Ok(())
}

fn make_boot_block(dev: &mut File, log: Logger) -> Result<(), MakefsError> {
    info!(log, "Making the boot block...");
    let boot_block = BootBlock {
        ..Default::default()
    };
    bincode::serialize_into(dev, &boot_block)?;
    Ok(())
}

fn make_super_block(
    dev: &mut File,
    inode_count: u64,
    data_block_count: u64,
    log: Logger,
) -> Result<(), MakefsError> {
    info!(log, "Making the super block...");
    let super_block = SuperBlock {
        magic_number: MAGIC_NUMBER,
        inode_count,
        data_block_count,
        ..Default::default()
    };
    bincode::serialize_into(dev, &super_block)?;
    Ok(())
}

fn make_inodes(dev: &mut File, inode_count: u64, log: Logger) -> Result<(), MakefsError> {
    info!(log, "Making inodes...");
    let init_inode = Inode::FreeInode {
        free_count: inode_count,
        next_free: 0,
    };
    bincode::serialize_into(dev, &init_inode)?;
    Ok(())
}

fn make_data_blocks(dev: &mut File, data_block_count: u64, log: Logger) -> Result<(), MakefsError> {
    info!(log, "Making data blocks...");
    let init_data_block = FreeDataBlock {
        free_count: data_block_count,
        next_free: 0,
    };
    bincode::serialize_into(dev, &init_data_block)?;
    Ok(())
}

fn dev_size(dev: &File, log: Logger) -> Result<u64, MakefsError> {
    let metadata = dev.metadata()?;
    let file_type = metadata.file_type();

    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    if file_type.is_file() {
        info!(log, "Regular file detected. Treat it as an image file.");
        Ok(metadata.size())
    } else if file_type.is_block_device() {
        info!(log, "Block device detected.");
        block_dev_size(&dev, log)
    } else {
        Err(MakefsError::UnsupportedDeviceError.into())
    }
}

// #[cfg(target_os = "linux")]
fn block_dev_size(dev: &File, _log: Logger) -> Result<u64, MakefsError> {
    use std::os::unix::io::{AsRawFd, RawFd};
    let fd = dev.as_raw_fd();

    #[cfg(target_os = "linux")]
    fn getsize(fd: RawFd) -> Result<u64, MakefsError> {
        // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
        ioctl_read!(getsize64, 0x12, 114, u64);
        let mut size: u64 = 0;
        unsafe {
            getsize64(fd, &mut size)?;
        }
        Ok(size)
    }

    #[cfg(target_os = "macos")]
    fn getsize(fd: RawFd) -> Result<u64, MakefsError> {
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
    fn getsize(fd: RawFd) -> Result<u64, MakefsError> {
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

// A boot block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
struct BootBlock {
    // _padding: [Padding256B; 4],
}

const_assert!(boot_block; (size_of::<BootBlock>() as u64) <= BOOT_BLOCK_SIZE);

// A super block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
struct SuperBlock {
    magic_number: u64,
    inode_count: u64,
    used_inode_count: u64,
    data_block_count: u64,
    used_data_block_count: u64,
    root_inode_ptr: u64,
    free_inode_ptr: u64,
    free_block_ptr: u64,
    // _padding: ([Padding256B; 3], [u64; 24]),
}

const_assert!(super_block; (size_of::<SuperBlock>() as u64) <= SUPER_BLOCK_SIZE);

bitflags! {
    #[derive(Serialize, Deserialize)]
    struct FileMode: u32 {
        const REGULAR_FILE = 0b00000001;
    }
}

#[derive(Serialize, Deserialize)]
struct TimeSpec {
    sec: i64,
    nsec: i64,
}

#[derive(Serialize, Deserialize)]
enum Inode {
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
struct FreeDataBlock {
    free_count: u64,
    next_free: u64,
}

union DataBlock {
    _data: [u8; 4096],
    _ptrs: [u64; 512],
    _free: FreeDataBlock,
}

const_assert!(data_block; (size_of::<DataBlock>() as u64) <= BLOCK_SIZE);

// #[derive(Serialize, Deserialize, Copy, Clone, Default)]
// struct Padding256B([u64; 32]);

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}
