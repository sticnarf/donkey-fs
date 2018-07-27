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
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::*;
use std::io::{self, Read, Seek, SeekFrom, Write};
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
pub const INODE_START: u64 = 114514;

pub struct DonkeyBuilder {
    dev: File,
}

pub struct Donkey {
    dev: File,
    super_block: SuperBlock,
    file_handles: FileHandles,
}

impl Donkey {
    fn new(dev: File, super_block: SuperBlock) -> Self {
        Donkey {
            dev,
            super_block,
            file_handles: FileHandles::new(),
        }
    }

    fn read_block<B: DeserializableBlock>(&mut self, ptr: u64) -> Result<B, Error> {
        read_block(&mut self.dev, ptr)
    }

    fn read_inode(&mut self, inode_number: u64) -> Result<Inode, Error> {
        self.read_block(inode_ptr(inode_number)?)
    }

    fn write_block<B: SerializableBlock>(&mut self, ptr: u64, block: &B) -> Result<(), Error> {
        write_block(&mut self.dev, ptr, block)
    }

    fn write_super_block(&mut self) -> Result<(), Error> {
        let super_block = self.super_block.clone();
        self.write_block(BOOT_BLOCK_SIZE, &super_block)
    }

    fn write_inode(&mut self, inode_number: u64, inode: &Inode) -> Result<(), Error> {
        self.write_block(inode_ptr(inode_number)?, inode)
    }

    fn allocate_inode(&mut self) -> Result<u64, Error> {
        let free_inode_number = self.super_block.free_inode;
        let inode = self.read_inode(free_inode_number)?;
        match inode {
            Inode::UsedInode { .. } => Err(format_err!(
                "Expect inode {} to be free, but used.",
                free_inode_number
            )),
            Inode::FreeInode {
                free_count,
                next_free,
            } => {
                if free_count > 1 {
                    let new_inode = Inode::FreeInode {
                        free_count: free_count - 1,
                        next_free,
                    };

                    let new_inode_number = free_inode_number + 1;
                    self.write_inode(new_inode_number, &new_inode)?;
                    self.super_block.free_inode = new_inode_number;
                } else {
                    self.super_block.free_inode = next_free;
                }
                self.super_block.used_inode_count += 1;
                self.write_super_block()?;
                Ok(free_inode_number)
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

    // If level is 0, then ptr is a direct pointer
    // offset is counted from the beginning of ptr
    // ptr must be the beginning of a block
    // This method returns how many bytes is written
    fn write_via_indirect_ptr(
        &mut self,
        ptr: u64,
        level: i32,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, Error> {
        assert!(ptr != 0); // Block must be already allocated
        let block_offset = offset % BLOCK_SIZE;
        let block_left = (BLOCK_SIZE - block_offset) as usize;
        let write_len = std::cmp::min(block_left, data.len());

        if level == 0 {
            self.dev.seek(SeekFrom::Start(ptr + block_offset))?;
            self.dev.write_all(&data[..write_len])?;
        } else {
            assert!(level > 0 && level <= 4);
            let indirect_block_size = 512u64.pow((level - 1) as u32);
            let block_index = offset / indirect_block_size;
            self.dev.seek(SeekFrom::Start(ptr + block_index * 8))?;
            let mut next_ptr = bincode::deserialize_from(&mut self.dev)?;
            if next_ptr == 0 {
                next_ptr = self.allocate_block()?;
                self.dev.seek(SeekFrom::Start(ptr + block_index * 8))?;
                bincode::serialize_into(&mut self.dev, &next_ptr)?;
            }
            self.write_via_indirect_ptr(
                next_ptr,
                level - 1,
                offset % indirect_block_size,
                &data[..],
            )?;
        }

        Ok(write_len)
    }

    // Returns the inode number of the new directory
    fn mkdir_raw(
        &mut self,
        parent_inode: u64,
        permission: FileMode,
        uid: u32,
        gid: u32,
        nlink: u64,
        log: Option<Logger>,
    ) -> Result<u64, Error> {
        let inode_number = self.allocate_inode()?;
        let time = SystemTime::now().into();
        let mode = FileMode::DIRECTORY | permission;
        let entries = [
            DirectoryEntry::new(inode_number, "."),
            DirectoryEntry::new(parent_inode, ".."),
        ];
        let buf = bincode::serialize(&entries)?;
        let mut inode = Inode::init_used(mode, uid, gid, nlink, time, buf.len() as u64);
        try_debug!(log, "directory: {:?}", buf);
        if let Inode::UsedInode { ptrs, .. } = &mut inode {
            let mut dkfile = DonkeyFile::new(self, ptrs).log(log);
            dkfile.write_all(&buf)?;
        }
        self.write_inode(inode_number, &inode)?;
        Ok(inode_number)
    }

    fn create_root(&mut self, log: Option<Logger>) -> Result<(), Error> {
        try_info!(log, "Creating root directory...");
        let root_permission = FileMode::USER_RWX
            | FileMode::GROUP_READ
            | FileMode::GROUP_EXECUTE
            | FileMode::ANY_READ
            | FileMode::ANY_EXECUTE;
        // Here we assume INODE_START is the root inode number
        let root_inode = self.mkdir_raw(INODE_START, root_permission, 0, 0, 1, log)?;
        self.super_block.root_inode = root_inode;
        self.write_super_block()?;
        Ok(())
    }

    pub fn root_inode(&self) -> u64 {
        self.super_block.root_inode
    }

    // Returns the file handle
    pub fn open(&mut self, inode_number: u64) -> Result<u64, Error> {
        let inode = self.read_inode(inode_number)?;
        Ok(self.file_handles.add(inode))
    }

    pub fn close(&mut self, file_handle: u64) {
        self.file_handles.remove(file_handle)
    }

    // returns entry and the new offset
    pub fn read_dir(
        &mut self,
        file_handle: u64,
        offset: u64,
    ) -> Result<Option<(DirectoryEntry, u64)>, Error> {
        let inode = self
            .file_handles
            .get(file_handle)
            .ok_or(format_err!("Bad file handle."))?;

        if offset < 12 * BLOCK_SIZE {
            let block = match inode {
                Inode::UsedInode {
                    size_or_device,
                    ptrs,
                    ..
                } => if offset >= *size_or_device {
                    return Ok(None);
                } else {
                    ptrs.direct_ptrs[(offset / BLOCK_SIZE) as usize]
                },
                _ => return Err(format_err!("Bad inode")),
            };
            self.dev.seek(SeekFrom::Start(block + offset % BLOCK_SIZE))?;
            let entry = bincode::deserialize_from(&mut self.dev)?;
            let new_offset = offset + bincode::serialized_size(&entry)?;
            Ok(Some((entry, new_offset)))
        } else {
            // Indirect pointer is not implemented
            unimplemented!()
        }
    }

    pub fn get_attr(&mut self, inode_number: u64) -> Result<FileAttr, Error> {
        let inode = self.read_inode(inode_number)?;
        FileAttr::from_inode(inode)
    }
}

impl DonkeyBuilder {
    pub fn new<P: AsRef<Path>>(dev_path: P) -> Result<DonkeyBuilder, Error> {
        let dev = OpenOptions::new().read(true).write(true).open(dev_path)?;
        Ok(DonkeyBuilder { dev })
    }

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
        Ok(Donkey::new(self.dev, super_block))
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

struct FileHandles {
    map: BTreeMap<u64, Inode>,
    top: u64,
}

impl FileHandles {
    fn new() -> Self {
        FileHandles {
            map: BTreeMap::new(),
            top: 1,
        }
    }

    fn add(&mut self, inode: Inode) -> u64 {
        let top = self.top;
        self.map.insert(top, inode);
        self.top += 1;
        top
    }

    fn get(&mut self, id: u64) -> Option<&mut Inode> {
        self.map.get_mut(&id)
    }

    fn remove(&mut self, id: u64) {
        self.map.remove(&id);
    }
}

struct DonkeyFile<'a> {
    dk: &'a mut Donkey,
    ptrs: &'a mut InodePtrs,
    offset: u64,
    log: Option<Logger>,
}

impl<'a> DonkeyFile<'a> {
    fn new(dk: &'a mut Donkey, ptrs: &'a mut InodePtrs) -> Self {
        DonkeyFile {
            dk,
            ptrs,
            offset: 0,
            log: None,
        }
    }

    fn log(mut self, log: Option<Logger>) -> Self {
        self.log = log;
        self
    }

    // This method does not modify the size in inode
    fn offset_write(&mut self, offset: u64, data: &[u8]) -> Result<usize, Error> {
        let block_index = offset / BLOCK_SIZE;
        let written = if block_index < 12 {
            // direct pointer
            let block_index = block_index as usize;
            if self.ptrs.direct_ptrs[block_index] == 0 {
                // block is not allocated
                self.ptrs.direct_ptrs[block_index] = self.dk.allocate_block()?;
            }
            self.dk.write_via_indirect_ptr(
                self.ptrs.direct_ptrs[block_index],
                0,
                offset % BLOCK_SIZE,
                data,
            )?
        } else if block_index < 12 + 512 {
            // indirect pointer
            if self.ptrs.indirect_ptr == 0 {
                // indirect block is not allocated
                self.ptrs.indirect_ptr = self.dk.allocate_block()?;
            }
            self.dk.write_via_indirect_ptr(
                self.ptrs.indirect_ptr,
                1,
                offset - 12 * BLOCK_SIZE,
                data,
            )?
        } else if block_index < 12 + 512 + 512 * 512 {
            // double indirect pointer
            if self.ptrs.double_indirect_ptr == 0 {
                // double indirect block is not allocated
                self.ptrs.double_indirect_ptr = self.dk.allocate_block()?;
            }
            self.dk.write_via_indirect_ptr(
                self.ptrs.double_indirect_ptr,
                2,
                offset - (12 + 512) * BLOCK_SIZE,
                data,
            )?
        } else if block_index < 12 + 512 + 512 * 512 + 512 * 512 * 512 {
            // triple indirect pointer
            if self.ptrs.triple_indirect_ptr == 0 {
                // triple indirect block is not allocated
                self.ptrs.triple_indirect_ptr = self.dk.allocate_block()?;
            }
            self.dk.write_via_indirect_ptr(
                self.ptrs.triple_indirect_ptr,
                3,
                offset - (12 + 512 + 512 * 512) * BLOCK_SIZE,
                data,
            )?
        } else {
            // Assuming file size does not exceed 256 TB
            // quadruple indirect pointer
            if self.ptrs.quadruple_indirect_ptr == 0 {
                // triple indirect block is not allocated
                self.ptrs.quadruple_indirect_ptr = self.dk.allocate_block()?;
            }
            self.dk.write_via_indirect_ptr(
                self.ptrs.quadruple_indirect_ptr,
                4,
                offset - (12 + 512 + 512 * 512 + 512 * 512 * 512) * BLOCK_SIZE,
                data,
            )?
        };
        if written == data.len() {
            // all data written
            Ok(written)
        } else {
            Ok(written + self.offset_write(offset + written as u64, &data[written..])?)
        }
    }
}

impl<'a> Write for DonkeyFile<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let offset = self.offset;
        let written = self
            .offset_write(offset, buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
        self.offset += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // NOTICE: Not carefully considered!
        //         Possibly a bug!
        self.dk.dev.flush()
    }
}

impl<'a> Read for DonkeyFile<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        unimplemented!()
    }
}

impl<'a> Seek for DonkeyFile<'a> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Start(pos) => self.offset = pos,
            SeekFrom::Current(diff) => {
                let new_offset = self.offset as i64 + diff;
                if new_offset >= 0 {
                    self.offset = new_offset as u64
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Seeking to a negative offset",
                    ));
                }
            }
            SeekFrom::End(_) => unimplemented!(), // Seek from end is not implemented yet
        }
        Ok(self.offset)
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

fn inode_ptr(inode_number: u64) -> Result<u64, Error> {
    let offset = inode_number
        .checked_sub(INODE_START)
        .ok_or(format_err!("Inode number underflow!"))?;
    Ok(INODE_SIZE * offset + BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE)
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
            free_inode: INODE_START,
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

impl From<u64> for FileMode {
    fn from(mode: u64) -> FileMode {
        FileMode::from_bits_truncate(mode)
    }
}

pub fn is_regular_file<T: Into<FileMode>>(mode: T) -> bool {
    !(mode.into() & FileMode::REGULAR_FILE).is_empty()
}

pub fn is_directory<T: Into<FileMode>>(mode: T) -> bool {
    !(mode.into() & FileMode::DIRECTORY).is_empty()
}

pub fn is_symbolic_link<T: Into<FileMode>>(mode: T) -> bool {
    !(mode.into() & FileMode::SYMBOLIC_LINK).is_empty()
}

pub fn is_block_device<T: Into<FileMode>>(mode: T) -> bool {
    !(mode.into() & FileMode::BLOCK_DEVICE).is_empty()
}

pub fn is_character_device<T: Into<FileMode>>(mode: T) -> bool {
    !(mode.into() & FileMode::CHARACTER_DEVICE).is_empty()
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: i64,
}

impl From<SystemTime> for Timespec {
    fn from(t: SystemTime) -> Self {
        let duration = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        Timespec {
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
        nlink: u64,
        atime: Timespec,
        mtime: Timespec,
        ctime: Timespec,
        crtime: Timespec,
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
        nlink: u64,
        time: Timespec,
        size_or_device: u64,
    ) -> Self {
        Inode::UsedInode {
            mode,
            uid,
            gid,
            nlink,
            atime: time,
            mtime: time,
            ctime: time,
            crtime: time,
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
pub struct DirectoryEntry {
    pub inode: u64,
    pub filename: OsString,
}

impl DirectoryEntry {
    fn new<T>(inode: u64, filename: T) -> Self
    where
        T: Into<OsString>,
    {
        DirectoryEntry {
            inode,
            filename: filename.into(),
        }
    }
}

pub struct FileAttr {
    pub mode: FileMode,
    pub size: u64,
    pub atime: Timespec,
    pub mtime: Timespec,
    pub ctime: Timespec,
    pub crtime: Timespec,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
}

impl FileAttr {
    fn from_inode(inode: Inode) -> Result<FileAttr, Error> {
        match inode {
            Inode::UsedInode {
                mode,
                uid,
                gid,
                nlink,
                atime,
                mtime,
                ctime,
                crtime,
                size_or_device,
                ..
            } => {
                let mut attr = FileAttr {
                    mode,
                    size: 0,
                    atime,
                    mtime,
                    ctime,
                    crtime,
                    nlink,
                    uid,
                    gid,
                    rdev: 0,
                };
                if is_block_device(mode) || is_character_device(mode) {
                    attr.rdev = size_or_device;
                } else {
                    attr.size = size_or_device;
                }
                Ok(attr)
            }
            _ => Err(format_err!("Bad inode.")),
        }
    }
}

#[cfg(test)]
mod tests {}
