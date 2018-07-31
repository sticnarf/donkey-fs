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
use std::ffi::{OsStr, OsString};
use std::fs::*;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::ops::Drop;
use std::path::Path;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

pub const MAGIC_NUMBER: u64 = 0x1BADFACEDEADC0DE;
pub const BOOT_BLOCK_SIZE: u64 = 1024;
pub const SUPER_BLOCK_SIZE: u64 = 1024;
pub const INODE_SIZE: u64 = 256;
pub const BLOCK_SIZE: u64 = 4096;
pub const DEFAULT_BYTES_PER_INODE: u64 = 16384;
pub const DEFAULT_BYTES_PER_INODE_STR: &'static str = "16384";
pub const INODE_START: u64 = 114514;

type Result<T> = std::result::Result<T, Error>;

pub struct DonkeyBuilder {
    dev: File,
}

type InnerDonkeyMutex = Arc<Mutex<InnerDonkey>>;

#[derive(Clone)]
pub struct Donkey {
    inner: InnerDonkeyMutex,
}

impl Donkey {
    fn new(inner: InnerDonkey) -> Donkey {
        Donkey {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

// TODO?
// If mutex is poisoned, the program panics.
impl Donkey {
    fn lock(&self) -> MutexGuard<InnerDonkey> {
        self.inner.lock().unwrap()
    }

    pub fn root_inode(&self) -> u64 {
        let inner = self.lock();
        inner.super_block.root_inode
    }

    // Returns the file handle
    pub fn open(
        &self,
        inode_number: u64,
        flags: OpenFlags,
        log: Option<Logger>,
    ) -> Result<DonkeyFile> {
        DonkeyFile::new(self.inner.clone(), inode_number, flags, log)
    }

    fn create_root(&self, log: Option<Logger>) -> Result<()> {
        try_info!(log, "Creating root directory...");
        let root_permission = FileMode::USER_RWX
            | FileMode::GROUP_READ
            | FileMode::GROUP_EXECUTE
            | FileMode::OTHERS_READ
            | FileMode::OTHERS_EXECUTE;
        // Here we assume INODE_START is the root inode number
        let root_inode = self.mkdir_raw(INODE_START, root_permission, 0, 0, log)?;
        let mut inner = self.lock();
        inner.super_block.root_inode = root_inode;
        inner.write_super_block()?;
        Ok(())
    }

    // Returns the inode number of the new node
    pub fn mknod_raw(
        &self,
        mode: FileMode,
        uid: u32,
        gid: u32,
        nlink: u64,
        rdev: Option<u64>,
        log: Option<Logger>,
    ) -> Result<u64> {
        let mut inner = self.lock();
        let inode_number = inner.allocate_inode(log.clone())?;
        let time = SystemTime::now().into();
        let size_or_device = rdev.unwrap_or(0);
        let inode = Inode::init_used(mode, uid, gid, nlink, time, size_or_device);
        inner.write_inode(inode_number, &inode, log)?;
        Ok(inode_number)
    }

    // Returns the inode number of the new directory
    // This method DOES NOT link the new directory to
    // the parent directory!!!!!!
    pub fn mkdir_raw(
        &self,
        parent_inode: u64,
        permission: FileMode,
        uid: u32,
        gid: u32,
        log: Option<Logger>,
    ) -> Result<u64> {
        let mode = FileMode::DIRECTORY | permission;
        let inode_number = self.mknod_raw(mode, uid, gid, 0, None, log.clone())?;

        // let entries = [
        //     DirectoryEntry::new(inode_number, "."),
        //     DirectoryEntry::new(parent_inode, ".."),
        // ];
        // let buf = bincode::serialize(&entries)?;
        // let mut dkfile = self.open(inode_number, OpenFlags::WRITE_ONLY, log)?;
        // dkfile.write_all(&buf)?;
        self.link(inode_number, parent_inode, OsStr::new("."), log.clone())?;
        self.link(inode_number, parent_inode, OsStr::new(".."), log.clone())?;
        Ok(inode_number)
    }

    // TODO? Cannot handle same filename!
    pub fn link(&self, inode: u64, parent: u64, name: &OsStr, log: Option<Logger>) -> Result<()> {
        {
            let mut dir = self.open(
                parent,
                OpenFlags::WRITE_ONLY | OpenFlags::APPEND,
                log.clone(),
            )?;
            let entry = DirectoryEntry::new(inode, name);
            let buf = bincode::serialize(&entry)?;
            dir.write_all(&buf)?;
        }

        let mut file = self.open(inode, OpenFlags::WRITE_ONLY, log)?;
        file.set_attr(SetFileAttr::new().nlink_inc(1))?;
        Ok(())
    }
}

pub struct InnerDonkey {
    dev: File,
    super_block: SuperBlock,
}

impl InnerDonkey {
    fn new(dev: File, super_block: SuperBlock) -> Self {
        InnerDonkey { dev, super_block }
    }

    fn read_block<B: DeserializableBlock>(&mut self, ptr: u64) -> Result<B> {
        read_block(&mut self.dev, ptr)
    }

    fn read_inode(&mut self, inode_number: u64) -> Result<Inode> {
        self.read_block(inode_ptr(inode_number)?)
    }

    fn write_block<B: SerializableBlock>(&mut self, ptr: u64, block: &B) -> Result<()> {
        write_block(&mut self.dev, ptr, block)
    }

    fn write_super_block(&mut self) -> Result<()> {
        let super_block = self.super_block.clone();
        self.write_block(BOOT_BLOCK_SIZE, &super_block)
    }

    fn write_inode(&mut self, inode_number: u64, inode: &Inode, log: Option<Logger>) -> Result<()> {
        try_debug!(
            log,
            "inode {} is written back, value: {:?}",
            inode_number,
            inode
        );
        self.write_block(inode_ptr(inode_number)?, inode)
    }

    fn allocate_inode(&mut self, log: Option<Logger>) -> Result<u64> {
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
                    self.write_inode(new_inode_number, &new_inode, log)?;
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

    fn allocate_block(&mut self) -> Result<u64> {
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
    ) -> Result<usize> {
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

    // If level is 0, then ptr is a direct pointer
    // offset is counted from the beginning of ptr
    // ptr must be the beginning of a block
    fn read_via_indirect_ptr(
        &mut self,
        ptr: u64,
        level: i32,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<u8>> {
        if ptr == 0 {
            return Err(format_err!("Read through an invalid pointer."));
        }
        let block_offset = offset % BLOCK_SIZE;
        let block_left = (BLOCK_SIZE - block_offset) as usize;
        let read_size = std::cmp::min(block_left, limit);

        if level == 0 {
            let mut data = vec![0; read_size];
            self.dev.seek(SeekFrom::Start(ptr + block_offset))?;
            self.dev.read_exact(&mut data[..read_size])?;
            Ok(data)
        } else {
            assert!(level > 0 && level <= 4);
            let indirect_block_size = 512u64.pow((level - 1) as u32);
            let block_index = offset / indirect_block_size;
            self.dev.seek(SeekFrom::Start(ptr + block_index * 8))?;
            let next_ptr = bincode::deserialize_from(&mut self.dev)?;
            self.read_via_indirect_ptr(next_ptr, level - 1, offset % indirect_block_size, limit)
        }
    }
}

impl DonkeyBuilder {
    pub fn new<P: AsRef<Path>>(dev_path: P) -> Result<DonkeyBuilder> {
        let dev = OpenOptions::new().read(true).write(true).open(dev_path)?;
        Ok(DonkeyBuilder { dev })
    }

    fn read_super_block(&mut self) -> Result<SuperBlock> {
        let super_block: SuperBlock = read_block(&mut self.dev, BOOT_BLOCK_SIZE)?;

        // validate magic number
        if super_block.magic_number != MAGIC_NUMBER {
            Err(format_err!("Maybe this device is not using Donkey?"))
        } else {
            Ok(super_block)
        }
    }

    pub fn open(mut self) -> Result<Donkey> {
        let super_block = self.read_super_block()?;
        Ok(Donkey::new(InnerDonkey::new(self.dev, super_block)))
    }

    pub fn format(mut self, opts: &FormatOptions, log: Option<Logger>) -> Result<Donkey> {
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

        let fs = self.open()?;
        fs.create_root(log.clone())?;
        Ok(fs)
    }
}

pub struct DonkeyFile {
    dk: InnerDonkeyMutex,
    inode: Inode,
    pub inode_number: u64,
    pub offset: u64,
    pub flags: OpenFlags,
    dirty: bool,
    log: Option<Logger>,
}

impl DonkeyFile {
    fn new(
        dk: InnerDonkeyMutex,
        inode_number: u64,
        flags: OpenFlags,
        log: Option<Logger>,
    ) -> Result<Self> {
        let dk2 = dk.clone();
        let mut dk2 = dk2.lock().unwrap();
        let dkfile = DonkeyFile {
            dk,
            inode: dk2.read_inode(inode_number)?,
            inode_number,
            offset: 0,
            flags,
            dirty: false,
            log,
        };
        Ok(dkfile)
    }

    fn seek_end(&mut self) -> io::Result<()> {
        let end = match &self.inode {
            Inode::FreeInode { .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Seek through an empty inode.",
                ))
            }
            Inode::UsedInode {
                mode,
                size_or_device,
                ..
            } => {
                if !is_managed(*mode) {
                    // This file is not managed by the filesystem
                    unreachable!()
                }
                *size_or_device
            }
        };
        self.offset = end;
        Ok(())
    }

    // This method does not modify the size in inode
    fn offset_write(&mut self, offset: u64, data: &[u8]) -> Result<usize> {
        let written = {
            let mut dk = self.dk.lock().unwrap();
            match &mut self.inode {
                Inode::FreeInode { .. } => return Err(format_err!("Write through an empty inode.")),
                Inode::UsedInode { mode, ptrs, .. } => {
                    if !is_managed(*mode) {
                        // This file is not managed by the filesystem
                        unreachable!()
                    }
                    let block_index = offset / BLOCK_SIZE;
                    if block_index < 12 {
                        // direct pointer
                        let block_index = block_index as usize;
                        if ptrs.direct_ptrs[block_index] == 0 {
                            // block is not allocated
                            ptrs.direct_ptrs[block_index] = dk.allocate_block()?;
                        }
                        dk.write_via_indirect_ptr(
                            ptrs.direct_ptrs[block_index],
                            0,
                            offset % BLOCK_SIZE,
                            data,
                        )?
                    } else if block_index < 12 + 512 {
                        // indirect pointer
                        if ptrs.indirect_ptr == 0 {
                            // indirect block is not allocated
                            ptrs.indirect_ptr = dk.allocate_block()?;
                        }
                        dk.write_via_indirect_ptr(
                            ptrs.indirect_ptr,
                            1,
                            offset - 12 * BLOCK_SIZE,
                            data,
                        )?
                    } else if block_index < 12 + 512 + 512 * 512 {
                        // double indirect pointer
                        if ptrs.double_indirect_ptr == 0 {
                            // double indirect block is not allocated
                            ptrs.double_indirect_ptr = dk.allocate_block()?;
                        }
                        dk.write_via_indirect_ptr(
                            ptrs.double_indirect_ptr,
                            2,
                            offset - (12 + 512) * BLOCK_SIZE,
                            data,
                        )?
                    } else if block_index < 12 + 512 + 512 * 512 + 512 * 512 * 512 {
                        // triple indirect pointer
                        if ptrs.triple_indirect_ptr == 0 {
                            // triple indirect block is not allocated
                            ptrs.triple_indirect_ptr = dk.allocate_block()?;
                        }
                        dk.write_via_indirect_ptr(
                            ptrs.triple_indirect_ptr,
                            3,
                            offset - (12 + 512 + 512 * 512) * BLOCK_SIZE,
                            data,
                        )?
                    } else {
                        // Assuming file size does not exceed 256 TB
                        // quadruple indirect pointer
                        if ptrs.quadruple_indirect_ptr == 0 {
                            // triple indirect block is not allocated
                            ptrs.quadruple_indirect_ptr = dk.allocate_block()?;
                        }
                        dk.write_via_indirect_ptr(
                            ptrs.quadruple_indirect_ptr,
                            4,
                            offset - (12 + 512 + 512 * 512 + 512 * 512 * 512) * BLOCK_SIZE,
                            data,
                        )?
                    }
                }
            }
        };
        if written == data.len() {
            // all data written
            Ok(written)
        } else {
            Ok(written + self.offset_write(offset + written as u64, &data[written..])?)
        }
    }

    fn offset_read(&mut self, offset: u64) -> Result<Vec<u8>> {
        let mut dk = self.dk.lock().unwrap();
        match &self.inode {
            Inode::FreeInode { .. } => Err(format_err!("Read through an empty inode.")),
            Inode::UsedInode {
                mode,
                size_or_device,
                ptrs,
                ..
            } => {
                if !is_managed(*mode) {
                    // This file is not managed by the filesystem
                    unreachable!()
                }
                if offset >= *size_or_device {
                    return Ok(Vec::new());
                }
                let limit = (size_or_device - offset) as usize;
                let block_index = offset / BLOCK_SIZE;
                if block_index < 12 {
                    // direct pointer
                    dk.read_via_indirect_ptr(
                        ptrs.direct_ptrs[block_index as usize],
                        0,
                        offset % BLOCK_SIZE,
                        limit,
                    )
                } else if block_index < 12 + 512 {
                    // indirect pointer
                    dk.read_via_indirect_ptr(ptrs.indirect_ptr, 1, offset - 12 * BLOCK_SIZE, limit)
                } else if block_index < 12 + 512 + 512 * 512 {
                    // double indirect pointer
                    dk.read_via_indirect_ptr(
                        ptrs.double_indirect_ptr,
                        2,
                        offset - (12 + 512) * BLOCK_SIZE,
                        limit,
                    )
                } else if block_index < 12 + 512 + 512 * 512 + 512 * 512 * 512 {
                    // triple indirect pointer
                    dk.read_via_indirect_ptr(
                        ptrs.triple_indirect_ptr,
                        3,
                        offset - (12 + 512 + 512 * 512) * BLOCK_SIZE,
                        limit,
                    )
                } else {
                    // Assuming file size does not exceed 256 TB
                    // quadruple indirect pointer
                    dk.read_via_indirect_ptr(
                        ptrs.quadruple_indirect_ptr,
                        4,
                        offset - (12 + 512 + 512 * 512 + 512 * 512 * 512) * BLOCK_SIZE,
                        limit,
                    )
                }
            }
        }
    }
}

impl DonkeyFile {
    // returns entry and the new offset
    pub fn read_dir(&mut self) -> Result<Option<DirectoryEntry>> {
        match self.inode {
            Inode::FreeInode { .. } => Err(format_err!("Bad inode.")),
            Inode::UsedInode { mode, .. } if !is_directory(mode) => {
                Err(format_err!("Not a directory."))
            }
            Inode::UsedInode { size_or_device, .. } if self.offset >= size_or_device => Ok(None),
            _ => {
                let entry = bincode::deserialize_from(self)?;
                Ok(Some(entry))
            }
        }
    }

    pub fn get_attr(&self) -> Result<FileAttr> {
        FileAttr::from_inode(&self.inode)
    }

    pub fn set_attr(&mut self, attr: SetFileAttr) -> Result<FileAttr> {
        macro_rules! modify {
            ($i:ident) => {
                if let Some(v) = attr.$i {
                    *$i = v;
                }
            };
        }

        match &mut self.inode {
            Inode::FreeInode { .. } => return Err(format_err!("Bad inode.")),
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
                modify!(mode);
                modify!(atime);
                modify!(mtime);
                modify!(ctime);
                modify!(crtime);
                modify!(uid);
                modify!(gid);
                if let Some(size) = attr.size {
                    *size_or_device = size;
                }
                if let Some(nlink_inc) = attr.nlink_inc {
                    let new_nlink = *nlink as i64 + nlink_inc;
                    *nlink = new_nlink as u64;
                }
            }
        }

        self.dirty = true;
        FileAttr::from_inode(&self.inode)
    }
}

impl Write for DonkeyFile {
    // This method modifies the size in the inode
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.dirty = true;

        if self.flags.contains(OpenFlags::APPEND) {
            try_debug!(self.log, "Open with APPEND flag, seek to end!");
            self.seek_end()?;
        }

        let offset = self.offset;
        let written = self
            .offset_write(offset, buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
        self.offset += written as u64;
        try_debug!(
            self.log,
            "{} bytes written, offset: {}",
            written,
            self.offset
        );
        if let Inode::UsedInode { size_or_device, .. } = &mut self.inode {
            if self.offset > *size_or_device {
                try_debug!(
                    self.log,
                    "Modify size from {} to {}",
                    *size_or_device,
                    self.offset
                );
                *size_or_device = self.offset;
            }
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // NOTICE: Not carefully considered!
        //         Possibly a bug!
        let mut dk = self.dk.lock().unwrap();
        if self.dirty {
            dk.write_inode(self.inode_number, &self.inode, self.log.clone())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
        }
        dk.dev.flush()
    }
}

impl Read for DonkeyFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let offset = self.offset;
        let read = self
            .offset_read(offset)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
        let len = std::cmp::min(buf.len(), read.len());
        buf[..len].copy_from_slice(&read[..len]);
        self.offset += len as u64;
        Ok(len)
    }
}

impl Seek for DonkeyFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Start(pos) => pos as i64,
            SeekFrom::Current(diff) => self.offset as i64 + diff,
            SeekFrom::End(diff) => {
                self.seek_end()?;
                self.offset as i64 + diff
            }
        };
        if new_offset >= 0 {
            self.offset = new_offset as u64
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seeking to a negative offset",
            ));
        }
        Ok(self.offset)
    }
}

impl Drop for DonkeyFile {
    fn drop(&mut self) {
        if self.dirty {
            let mut dk = self.dk.lock().unwrap();
            if let Err(e) = dk.write_inode(self.inode_number, &self.inode, self.log.clone()) {
                // If it fails, we can do nothing but print the error
                try_error!(self.log, "{}", e);
            }
        }
    }
}

fn read_block<B: DeserializableBlock>(dev: &mut File, ptr: u64) -> Result<B> {
    dev.seek(SeekFrom::Start(ptr))?;
    let block = bincode::deserialize_from(dev)?;
    Ok(block)
}

fn write_block<B: SerializableBlock>(dev: &mut File, ptr: u64, block: &B) -> Result<()> {
    dev.seek(SeekFrom::Start(ptr))?;
    bincode::serialize_into(dev, &block)?;
    Ok(())
}

fn inode_ptr(inode_number: u64) -> Result<u64> {
    let offset = inode_number
        .checked_sub(INODE_START)
        .ok_or(format_err!("Inode number {} underflow!", inode_number))?;
    Ok(INODE_SIZE * offset + BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE)
}

fn make_boot_block(dev: &mut File, log: Option<Logger>) -> Result<()> {
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
) -> Result<()> {
    try_info!(log, "Making the super block...");
    let super_block = SuperBlock::init(inode_count, data_block_count);
    write_block(dev, BOOT_BLOCK_SIZE, &super_block)?;
    Ok(())
}

fn make_inodes(dev: &mut File, inode_count: u64, log: Option<Logger>) -> Result<()> {
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
) -> Result<()> {
    try_info!(log, "Making data blocks...");
    let free_data_block = FreeDataBlock::init(data_block_count);
    write_block(
        dev,
        BOOT_BLOCK_SIZE + SUPER_BLOCK_SIZE + inode_count * INODE_SIZE,
        &free_data_block,
    )?;
    Ok(())
}

fn dev_size(dev: &File, log: Option<Logger>) -> Result<u64> {
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
fn block_dev_size(dev: &File, _log: Option<Logger>) -> Result<u64> {
    use std::os::unix::io::{AsRawFd, RawFd};
    let fd = dev.as_raw_fd();

    #[cfg(target_os = "linux")]
    fn getsize(fd: RawFd) -> Result<u64> {
        // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
        ioctl_read!(getsize64, 0x12, 114, u64);
        let mut size: u64 = 0;
        unsafe {
            getsize64(fd, &mut size)?;
        }
        Ok(size)
    }

    #[cfg(target_os = "macos")]
    fn getsize(fd: RawFd) -> Result<u64> {
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
    fn getsize(fd: RawFd) -> Result<u64> {
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
        const FILE_TYPE_MASK   = 0b11110000_00000000;
        const SOCKET           = 0b11000000_00000000;
        const REGULAR_FILE     = 0b10000000_00000000;
        const DIRECTORY        = 0b01000000_00000000;
        const SYMBOLIC_LINK    = 0b10100000_00000000;
        const CHARACTER_DEVICE = 0b00100000_00000000;
        const BLOCK_DEVICE     = 0b01100000_00000000;
        const FIFO             = 0b00010000_00000000;

        const SET_USER_ID      = 0b00001000_00000000;
        const SET_GROUP_ID     = 0b00000100_00000000;
        const STICKY           = 0b00000010_00000000;

        const USER_READ        = 0b00000001_00000000;
        const USER_WRITE       = 0b00000000_10000000;
        const USER_EXECUTE     = 0b00000000_01000000;
        const GROUP_READ       = 0b00000000_00100000;
        const GROUP_WRITE      = 0b00000000_00010000;
        const GROUP_EXECUTE    = 0b00000000_00001000;
        const OTHERS_READ      = 0b00000000_00000100;
        const OTHERS_WRITE     = 0b00000000_00000010;
        const OTHERS_EXECUTE   = 0b00000000_00000001;
        const USER_RWX         = 0b00000001_11000000;
        const GROUP_RWX        = 0b00000000_00111000;
        const OTHERS_RWX       = 0b00000000_00000111;
    }
}

impl From<u64> for FileMode {
    fn from(mode: u64) -> FileMode {
        FileMode::from_bits_truncate(mode)
    }
}

pub fn is_regular_file<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::REGULAR_FILE
}

pub fn is_directory<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::DIRECTORY
}

pub fn is_symbolic_link<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::SYMBOLIC_LINK
}

fn is_managed<T: Into<FileMode>>(mode: T) -> bool {
    let mode = mode.into();
    is_regular_file(mode) || is_directory(mode) || is_symbolic_link(mode)
}

pub fn is_block_device<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::BLOCK_DEVICE
}

pub fn is_character_device<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::CHARACTER_DEVICE
}

pub fn is_device<T: Into<FileMode>>(mode: T) -> bool {
    let mode = mode.into();
    is_block_device(mode) || is_character_device(mode)
}

pub fn is_fifo<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::FIFO
}

pub fn is_socket<T: Into<FileMode>>(mode: T) -> bool {
    (mode.into() & FileMode::FILE_TYPE_MASK) == FileMode::SOCKET
}

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct OpenFlags: u64 {
        const ACCESS_MODE_MASK = 0b00000000_00000011;
        const READ_ONLY        = 0b00000000_00000000;
        const WRITE_ONLY       = 0b00000000_00000001;
        const READ_WRITE       = 0b00000000_00000010;

        const APPEND           = 0b00000100_00000000;
    }
}

pub fn is_read_only<T: Into<OpenFlags>>(flags: T) -> bool {
    (flags.into() & OpenFlags::ACCESS_MODE_MASK) == OpenFlags::READ_ONLY
}

pub fn is_write_only<T: Into<OpenFlags>>(flags: T) -> bool {
    (flags.into() & OpenFlags::ACCESS_MODE_MASK) == OpenFlags::WRITE_ONLY
}

pub fn is_read_write<T: Into<OpenFlags>>(flags: T) -> bool {
    (flags.into() & OpenFlags::ACCESS_MODE_MASK) == OpenFlags::READ_WRITE
}

pub fn can_read<T: Into<OpenFlags>>(flags: T) -> bool {
    let flags = flags.into();
    is_read_only(flags) | is_read_write(flags)
}

pub fn can_write<T: Into<OpenFlags>>(flags: T) -> bool {
    let flags = flags.into();
    is_write_only(flags) | is_read_write(flags)
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
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

#[derive(Serialize, Deserialize, Clone, Debug)]
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

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
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

#[derive(Debug)]
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
    fn from_inode(inode: &Inode) -> Result<FileAttr> {
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
                    mode: *mode,
                    size: 0,
                    atime: *atime,
                    mtime: *mtime,
                    ctime: *ctime,
                    crtime: *crtime,
                    nlink: *nlink,
                    uid: *uid,
                    gid: *gid,
                    rdev: 0,
                };
                if is_block_device(*mode) || is_character_device(*mode) {
                    attr.rdev = *size_or_device;
                } else {
                    attr.size = *size_or_device;
                }
                Ok(attr)
            }
            _ => Err(format_err!("Bad inode.")),
        }
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub struct SetFileAttr {
    pub mode: Option<FileMode>,
    pub size: Option<u64>,
    pub atime: Option<Timespec>,
    pub mtime: Option<Timespec>,
    pub ctime: Option<Timespec>,
    pub crtime: Option<Timespec>,
    pub nlink_inc: Option<i64>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

impl SetFileAttr {
    pub fn new() -> Self {
        SetFileAttr::default()
    }

    pub fn mode(mut self, mode: FileMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    pub fn atime(mut self, atime: Timespec) -> Self {
        self.atime = Some(atime);
        self
    }

    pub fn ctime(mut self, ctime: Timespec) -> Self {
        self.ctime = Some(ctime);
        self
    }

    pub fn crtime(mut self, crtime: Timespec) -> Self {
        self.crtime = Some(crtime);
        self
    }

    pub fn mtime(mut self, mtime: Timespec) -> Self {
        self.mtime = Some(mtime);
        self
    }

    pub fn nlink_inc(mut self, nlink_inc: i64) -> Self {
        self.nlink_inc = Some(nlink_inc);
        self
    }

    pub fn uid(mut self, uid: u32) -> Self {
        self.uid = Some(uid);
        self
    }

    pub fn gid(mut self, gid: u32) -> Self {
        self.gid = Some(gid);
        self
    }
}

#[cfg(test)]
mod tests {}
