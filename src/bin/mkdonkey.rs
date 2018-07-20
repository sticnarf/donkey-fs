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

use slog::{Drain, Logger};
use std::fs::*;
use std::io::{self, Write};
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
    let block_count =
        (dev_size - BOOT_BLOCK_SIZE - SUPER_BLOCK_SIZE - inode_count * INODE_SIZE) / BLOCK_SIZE;

    info!(log, "Inode count: {}", inode_count);
    info!(log, "Block count: {}", block_count);

    make_boot_block(&mut dev, log.clone())?;
    make_super_block(&mut dev, inode_count, block_count, log.clone())?;
    Ok(())
}

fn make_boot_block(dev: &mut File, log: Logger) -> Result<(), MakefsError> {
    info!(log, "Making the boot block...");
    let boot_block = BootBlock {
        ..Default::default()
    };
    assert_eq!(size_of::<BootBlock>(), 1024);
    let mut block_slice = [0u8; size_of::<BootBlock>()];
    bincode::serialize_into(&mut block_slice[..], &boot_block)?;
    dev.write_all(&block_slice)?;
    Ok(())
}

fn make_super_block(
    dev: &mut File,
    inode_count: u64,
    block_count: u64,
    log: Logger,
) -> Result<(), MakefsError> {
    info!(log, "Making the super block...");
    let super_block = SuperBlock {
        magic_number: MAGIC_NUMBER,
        inode_count,
        block_count,
        used_inode_count: 1,
        used_block_count: 1,
        root_inode_ptr: 0,
        free_inode_ptr: 1,
        free_block_ptr: 1,
        ..Default::default()
    };
    assert_eq!(size_of::<SuperBlock>(), 1024);
    let mut block_slice = [0u8; size_of::<SuperBlock>()];
    bincode::serialize_into(&mut block_slice[..], &super_block)?;
    dev.write_all(&block_slice)?;
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

fn block_dev_size(dev: &File, log: Logger) -> Result<u64, MakefsError> {
    unimplemented!()
}

#[repr(C)]
#[derive(Serialize, Deserialize, Default)]
struct BootBlock {
    _padding: [Padding256B; 4],
}

#[repr(C)]
#[derive(Serialize, Deserialize, Default)]
struct SuperBlock {
    magic_number: u64,
    inode_count: u64,
    used_inode_count: u64,
    block_count: u64,
    used_block_count: u64,
    root_inode_ptr: u64,
    free_inode_ptr: u64,
    free_block_ptr: u64,
    _padding: ([Padding256B; 3], [u64; 24]),
}

#[repr(C)]
#[derive(Serialize, Deserialize, Copy, Clone, Default)]
struct Padding256B([u64; 32]);

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}
