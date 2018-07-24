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
use std::fs::*;
use std::io::{Seek, SeekFrom, Write};
use std::mem::size_of;
use std::path::Path;
use std::time::SystemTime;

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
    pub fn create<P: AsRef<Path>>(dev_path: P) -> Result<DonkeyBuilder, Error> {
        let dev = OpenOptions::new().read(true).write(true).open(dev_path)?;
        Ok(DonkeyBuilder { dev })
    }

    fn read_block<B: DeserializableBlock>(&mut self, ptr: u64) -> Result<B, Error> {
        read_block(&mut self.dev, ptr)
    }

    fn read_inode(&mut self, inode_number: u64) -> Result<Inode, Error> {
        let ptr = INODE_SIZE * inode_number + BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE;
        self.read_block(ptr)
    }

    fn write_block<B: SerializableBlock>(&mut self, ptr: u64, block: &B) -> Result<(), Error> {
        write_block(&mut self.dev, ptr, block)
    }

    fn write_super_block(&mut self) -> Result<(), Error> {
        let super_block = self.super_block.clone();
        self.write_block(BOOT_BLOCK_SIZE, &super_block)
    }

    fn write_inode(&mut self, inode_number: u64, inode: &Inode) -> Result<(), Error> {
        let ptr = INODE_SIZE * inode_number + BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE;
        self.write_block(ptr, inode)
    }

    fn allocate_inode(&mut self) -> Result<u64, Error> {
        let free = self.super_block.free_inode;
        let inode = self.read_inode(free)?;
        match inode {
            Inode::UsedInode { .. } => {
                Err(format_err!("Expect inode {} to be free, but used.", free))
            }
            Inode::FreeInode {
                free_count,
                next_free,
            } => {
                if free_count > 1 {
                    let new_inode = Inode::FreeInode {
                        free_count: free_count - 1,
                        next_free,
                    };

                    let new_inode_number = free + 1;
                    self.write_inode(new_inode_number, &new_inode)?;
                    self.super_block.free_inode = new_inode_number;
                } else {
                    self.super_block.free_inode = next_free;
                }
                self.super_block.used_inode_count += 1;
                self.write_super_block()?;
                Ok(free)
            }
        }
    }

    fn allocate_block(&mut self) -> Result<u64, Error> {
        let free = self.super_block.free_block_ptr;
        let data: FreeDataBlock = self.read_block(free)?;
        if data.free_count > 1 {
            let new_data = FreeDataBlock {
                free_count: data.free_count - 1,
                ..data
            };

            let new_data_ptr = free + BLOCK_SIZE;
            self.write_block(new_data_ptr, &new_data)?;
            self.super_block.free_block_ptr = new_data_ptr;
        } else {
            self.super_block.free_block_ptr = data.next_free;
        }
        self.super_block.used_block_count += 1;
        self.write_super_block()?;
        Ok(free)
    }

    fn write_data(&mut self, buf: &[u8]) -> Result<Vec<u64>, Error> {
        buf.chunks(BLOCK_SIZE as usize)
            .map(|chunk| {
                let block = self.allocate_block()?;
                self.dev.seek(SeekFrom::Start(block))?;
                self.dev.write_all(chunk)?;
                Ok(block)
            })
            .collect()
    }

    // Returns the inode number of the new directory
    fn mkdir_raw(
        &mut self,
        parent_inode: u64,
        permission: FileMode,
        uid: u32,
        gid: u32,
        link_count: u64,
    ) -> Result<u64, Error> {
        let inode_ptr = self.allocate_inode()?;
        let time = SystemTime::now().into();
        let mode = FileMode::DIRECTORY | permission;
        let entries = [
            DirectoryEntry::new(inode_ptr, "."),
            DirectoryEntry::new(parent_inode, ".."),
        ];
        let buf = bincode::serialize(&entries)?;
        let mut inode = Inode::init_used(mode, uid, gid, link_count, time, buf.len() as u64);
        let data_ptrs = self.write_data(&buf)?;
        if let Inode::UsedInode { ref mut ptrs, .. } = inode {
            if data_ptrs.len() <= 12 {
                ptrs.direct_ptrs[..data_ptrs.len()].copy_from_slice(&data_ptrs);
            } else {
                // Indirect pointers is not implemented yet
                unimplemented!()
            }
        }
        self.write_block(inode_ptr, &inode)?;
        Ok(inode_ptr)
    }

    fn create_root(&mut self, log: Option<Logger>) -> Result<(), Error> {
        try_info!(log, "Creating root directory...");
        let root_permission = FileMode::USER_RWX
            | FileMode::GROUP_READ
            | FileMode::GROUP_EXECUTE
            | FileMode::ANY_READ
            | FileMode::ANY_EXECUTE;
        let root_inode = self.mkdir_raw(0, root_permission, 0, 0, 1)?;
        self.super_block.root_inode = root_inode;
        self.write_super_block()?;
        Ok(())
    }
}

impl DonkeyBuilder {
    fn read_super_block(&mut self) -> Result<SuperBlock, Error> {
        let super_block: SuperBlock = read_block(&mut self.dev, BOOT_BLOCK_SIZE)?;

        // validate magic number
        if super_block.magic_number != MAGIC_NUMBER {
            Err(format_err!("Maybe this device is not using Donkey?"))
        } else {
            Ok(super_block)
        }
    }

    pub fn open(mut self) -> Result<Donkey, Error> {
        let super_block = self.read_super_block()?;
        Ok(Donkey {
            dev: self.dev,
            super_block,
        })
    }

    pub fn format(mut self, opts: &FormatOptions, log: Option<Logger>) -> Result<Donkey, Error> {
        let dev_size = dev_size(&self.dev, log.clone())?;
        let inode_count = dev_size / opts.bytes_per_inode;
        let data_block_count =
            (dev_size - BOOT_BLOCK_SIZE - SUPER_BLOCK_SIZE - inode_count * INODE_SIZE) / BLOCK_SIZE;

        try_info!(log, "Device size: {} bytes", dev_size);
        try_info!(log, "Inode count: {}", inode_count);
        try_info!(log, "Data block count: {}", data_block_count);

        make_boot_block(&mut self.dev, log.clone())?;
        make_super_block(&mut self.dev, inode_count, data_block_count, log.clone())?;
        make_inodes(&mut self.dev, inode_count, log.clone())?;
        make_data_blocks(&mut self.dev, inode_count, data_block_count, log.clone())?;

        let mut fs = self.open()?;
        fs.create_root(log.clone())?;
        Ok(fs)
    }
}

fn read_block<B: DeserializableBlock>(dev: &mut File, ptr: u64) -> Result<B, Error> {
    dev.seek(SeekFrom::Start(ptr))?;
    let block = bincode::deserialize_from(dev)?;
    Ok(block)
}

fn write_block<B: SerializableBlock>(dev: &mut File, ptr: u64, block: &B) -> Result<(), Error> {
    dev.seek(SeekFrom::Start(ptr))?;
    bincode::serialize_into(dev, &block)?;
    Ok(())
}

fn make_boot_block(dev: &mut File, log: Option<Logger>) -> Result<(), Error> {
    try_info!(log, "Making the boot block...");
    let boot_block = BootBlock::init();
    write_block(dev, 0, &boot_block)?;
    Ok(())
}

fn make_super_block(
    dev: &mut File,
    inode_count: u64,
    data_block_count: u64,
    log: Option<Logger>,
) -> Result<(), Error> {
    try_info!(log, "Making the super block...");
    let super_block = SuperBlock::init(inode_count, data_block_count);
    write_block(dev, BOOT_BLOCK_SIZE, &super_block)?;
    Ok(())
}

fn make_inodes(dev: &mut File, inode_count: u64, log: Option<Logger>) -> Result<(), Error> {
    try_info!(log, "Making inodes...");
    let init_inode = Inode::init_free(inode_count);
    write_block(dev, BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE, &init_inode)?;
    Ok(())
}

fn make_data_blocks(
    dev: &mut File,
    inode_count: u64,
    data_block_count: u64,
    log: Option<Logger>,
) -> Result<(), Error> {
    try_info!(log, "Making data blocks...");
    let free_data_block = FreeDataBlock::init(data_block_count);
    write_block(
        dev,
        BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE + inode_count * INODE_SIZE,
        &free_data_block,
    )?;
    Ok(())
}

fn dev_size(dev: &File, log: Option<Logger>) -> Result<u64, Error> {
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
        Err(format_err!("This device is not supported."))
    }
}

// #[cfg(target_os = "linux")]
fn block_dev_size(dev: &File, _log: Option<Logger>) -> Result<u64, Error> {
    use std::os::unix::io::{AsRawFd, RawFd};
    let fd = dev.as_raw_fd();

    #[cfg(target_os = "linux")]
    fn getsize(fd: RawFd) -> Result<u64, Error> {
        // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
        ioctl_read!(getsize64, 0x12, 114, u64);
        let mut size: u64 = 0;
        unsafe {
            getsize64(fd, &mut size)?;
        }
        Ok(size)
    }

    #[cfg(target_os = "macos")]
    fn getsize(fd: RawFd) -> Result<u64, Error> {
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
    fn getsize(fd: RawFd) -> Result<u64, Error> {
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
trait SerializableBlock: serde::ser::Serialize {}
trait DeserializableBlock: serde::de::DeserializeOwned {}

impl SerializableBlock for BootBlock {}
impl SerializableBlock for SuperBlock {}
impl SerializableBlock for Inode {}
impl SerializableBlock for FreeDataBlock {}
impl DeserializableBlock for BootBlock {}
impl DeserializableBlock for SuperBlock {}
impl DeserializableBlock for Inode {}
impl DeserializableBlock for FreeDataBlock {}

// A boot block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Default)]
struct BootBlock {}

const_assert!(boot_block; (size_of::<BootBlock>() as u64) <= BOOT_BLOCK_SIZE);

impl BootBlock {
    fn init() -> Self {
        BootBlock {}
    }
}

// A super block occupies 1024 bytes.
#[derive(Serialize, Deserialize, Clone, Default)]
struct SuperBlock {
    magic_number: u64,
    inode_count: u64,
    used_inode_count: u64,
    data_block_count: u64,
    used_block_count: u64,
    root_inode: u64,
    free_inode: u64,
    free_block_ptr: u64,
}

const_assert!(super_block; (size_of::<SuperBlock>() as u64) <= SUPER_BLOCK_SIZE);

impl SuperBlock {
    fn init(inode_count: u64, data_block_count: u64) -> Self {
        SuperBlock {
            magic_number: MAGIC_NUMBER,
            inode_count,
            data_block_count,
            free_block_ptr: BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE + INODE_SIZE * inode_count,
            ..Default::default()
        }
    }
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct FileMode: u64 {
        const REGULAR_FILE     = 0b10000000_00000000;
        const DIRECTORY        = 0b01000000_00000000;
        const SYMBOLIC_LINK    = 0b00100000_00000000;
        const BLOCK_DEVICE     = 0b00010000_00000000;
        const CHARACTER_DEVICE = 0b00001000_00000000;
        const USER_READ        = 0b00000100_00000000;
        const USER_WRITE       = 0b00000010_00000000;
        const USER_EXECUTE     = 0b00000001_00000000;
        const GROUP_READ       = 0b00000000_10000000;
        const GROUP_WRIT       = 0b00000000_01000000;
        const GROUP_EXECUTE    = 0b00000000_00100000;
        const ANY_READ         = 0b00000000_00010000;
        const ANY_WRITE        = 0b00000000_00001000;
        const ANY_EXECUTE      = 0b00000000_00000100;
        const USER_RWX         = 0b00000111_00000000;
        const GROUP_RWX        = 0b00000000_11100000;
        const ANY_RWX          = 0b00000000_00011100;
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct TimeSpec {
    pub sec: i64,
    pub nsec: i64,
}

impl From<SystemTime> for TimeSpec {
    fn from(t: SystemTime) -> Self {
        let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        TimeSpec {
            sec: duration.as_secs() as i64,
            nsec: duration.subsec_nanos() as i64,
        }
    }
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
        ptrs: InodePtrs,
    },
}

#[derive(Serialize, Deserialize, Default)]
struct InodePtrs {
    direct_ptrs: [u64; 12],
    indirect_ptr: u64,
    double_indirect_ptr: u64,
    triple_indirect_ptr: u64,
    quadruple_indirect_ptr: u64,
}

const_assert!(inode; (size_of::<Inode>() as u64) <= INODE_SIZE);

impl Inode {
    fn init_free(inode_count: u64) -> Self {
        Inode::FreeInode {
            free_count: inode_count,
            next_free: 0,
        }
    }

    fn init_used(
        mode: FileMode,
        uid: u32,
        gid: u32,
        link_count: u64,
        time: TimeSpec,
        size_or_device: u64,
    ) -> Self {
        Inode::UsedInode {
            mode,
            uid,
            gid,
            link_count,
            atime: time,
            mtime: time,
            ctime: time,
            size_or_device,
            ptrs: Default::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct FreeDataBlock {
    free_count: u64,
    next_free: u64,
}

impl FreeDataBlock {
    fn init(data_block_count: u64) -> Self {
        FreeDataBlock {
            free_count: data_block_count,
            next_free: 0,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DirectoryEntry<'a> {
    inode: u64,
    filename: &'a str,
}

impl<'a> DirectoryEntry<'a> {
    fn new(inode: u64, filename: &'a str) -> Self {
        DirectoryEntry { inode, filename }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
