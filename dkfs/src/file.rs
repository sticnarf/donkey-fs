use bincode::{deserialize_from, serialize_into};
use block::*;
use im::hashmap::{self as im_hashmap, HashMap as ImHashMap};
use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::{DerefMut, Drop};
use std::rc::Rc;
use *;

#[derive(Debug)]
pub struct DkFile {
    pub(crate) handle: Handle,
    pub(crate) inode: Inode,
    ind_ptr_cm: IndirectPtrCacheManager,
    pub(crate) pos: u64,
    pub(crate) dirty: bool,
}

impl Read for DkFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        unimplemented!()
    }
}

impl Write for DkFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        unimplemented!()
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.dirty {
            if let Err(e) = self.handle.write_inode(&self.inode) {
                try_error!(
                    self.handle.log,
                    "Failed to write directory of ino {}! {}",
                    self.inode.ino,
                    e
                );
            }
        }
        Ok(())
    }
}

impl Seek for DkFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(pos) => pos as i64,
            SeekFrom::Current(diff) => self.pos as i64 + diff,
            SeekFrom::End(diff) => self.inode.size as i64 + diff,
        };
        if new_pos >= 0 {
            self.pos = new_pos as u64;
            Ok(self.pos)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seeking to a negative offset",
            ))
        }
    }
}

impl Drop for DkFile {
    fn drop(&mut self) {
        // Remove from opened files
        self.handle.borrow_mut().opened_dirs.remove(&self.inode.ino);

        if let Err(e) = self.flush() {
            try_error!(self.log(), "Failed to write ino {}! {}", self.inode.ino, e);
        }
    }
}

impl DkFile {
    pub fn new(handle: Handle, inode: Inode) -> Self {
        DkFile {
            handle,
            inode,
            ind_ptr_cm: Default::default(),
            pos: 0,
            dirty: false,
        }
    }

    pub fn log(&self) -> Option<Logger> {
        self.handle.log.clone()
    }

    /// Specify the position of the file.
    /// Returns the real pointer and how many bytes in maximum
    /// you can read or write from this pointer.
    /// This function allocates blocks if necessary.
    fn locate(&mut self, pos: u64) -> DkResult<(u64, u64)> {
        fn locate_rec(
            handle: Handle,
            cache: &mut IndirectPtrCacheManager,
            ptrs: &mut [u64],
            level: u32,
            off: u64,
            bs: u64,
        ) -> DkResult<(u64, u64)> {
            fn ptr_or_allocate(handle: Handle, ptr: &mut u64, bs: u64) -> DkResult<u64> {
                if *ptr == 0 {
                    *ptr = handle.allocate_db()?;
                    handle.fill_zero(*ptr, bs)?;
                }
                Ok(*ptr)
            }

            let pc = bs / 8; // Pointer count in a single block
            let sz = bs * pc.pow(level); // Size of all blocks through the direct or indirect pointer
            let ptr = ptr_or_allocate(handle.clone(), &mut ptrs[(off / sz) as usize], bs)?;
            if level == 0 {
                Ok((ptr + off, bs - off))
            } else {
                let ind = cache.get(handle, ptr, level)?;
                unimplemented!()
            }
        }

        let bs = self.handle.borrow().sb.block_size;
        let ptrs = &mut self.inode.ptrs;
        let (level, off) = ptrs.locate(pos, bs);
        locate_rec(
            self.handle.clone(),
            &mut self.ind_ptr_cm,
            &mut self.inode.ptrs[level],
            level,
            off,
            bs,
        )
    }
}

#[derive(Debug, Default)]
struct IndirectPtrCacheManager([Option<IndirectPtrCache>; 4]);

impl IndirectPtrCacheManager {
    fn get(&mut self, handle: Handle, ptr: u64, level: u32) -> DkResult<&mut DataBlock> {
        let level = level as usize;
        match &mut self.0[level] {
            Some(c) if c.ptr == ptr => Ok(&mut c.data),
            r => {
                let data = rb!(handle, ptr, DataBlock)?;
                *r = Some(IndirectPtrCache {
                    handle,
                    ptr,
                    data,
                    dirty: false,
                });
                Ok(&mut r.as_mut().unwrap().data)
            }
        }
    }
}

#[derive(Debug)]
struct IndirectPtrCache {
    handle: Handle,
    ptr: u64,
    data: DataBlock,
    dirty: bool,
}

impl Drop for IndirectPtrCache {
    fn drop(&mut self) {
        if self.dirty {
            if let Err(e) = wb!(self.handle, self.data, self.ptr) {
                try_error!(
                    self.handle.log,
                    "Failed to write data block at {}: {}",
                    self.ptr,
                    e
                );
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DkFileHandle {
    pub(crate) inner: Rc<RefCell<DkFile>>,
    pub(crate) flags: Flags,
}

impl Deref for DkFileHandle {
    type Target = RefCell<DkFile>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug)]
pub struct DkDir {
    pub(crate) fh: DkFileHandle,
    pub(crate) entries: ImHashMap<OsString, u64>,
    pub(crate) dirty: bool,
}

impl DkDir {
    pub fn from_file(fh: DkFileHandle) -> DkResult<Self> {
        if !fh.borrow().inode.mode.is_directory() {
            Err(format_err!("Not a directory."))
        } else {
            let mut dir = DkDir {
                fh,
                entries: ImHashMap::new(),
                dirty: false,
            };
            dir.read_fully()?;
            Ok(dir)
        }
    }

    fn read_fully(&mut self) -> DkResult<()> {
        let mut f = self.fh.borrow_mut();
        let mut reader = BufReader::new(f.deref_mut());
        loop {
            // `name` and `ino` are deserialized separately so that
            // redundant copies are avoided when serializing
            let ino = deserialize_from(&mut reader)?;

            if ino >= ROOT_INODE {
                let name: OsString = deserialize_from(&mut reader)?;
                self.entries.insert(name, ino);
            } else if ino == ROOT_INODE - 1 {
                // `ino == ROOT_INODE - 1` indicates the end
                // of the directory.
                return Ok(());
            } else {
                return Err(format_err!("Invalid directory entry ino: {}", ino));
            }
        }
    }

    fn fs_handle(&self) -> Handle {
        self.fh.borrow().handle.clone()
    }

    fn ino(&self) -> u64 {
        self.fh.borrow().inode.ino
    }

    pub fn flush(&mut self) -> DkResult<()> {
        if self.dirty {
            let mut f = self.fh.borrow_mut();
            f.seek(SeekFrom::Start(0))?;
            let mut writer = BufWriter::new(f.deref_mut());
            for (name, ino) in &self.entries {
                serialize_into(&mut writer, ino)?;
                serialize_into(&mut writer, name)?;
            }
            // Indicates the end of the directory
            serialize_into(&mut writer, &(ROOT_INODE - 1))?;

            self.dirty = false;
        }
        Ok(())
    }

    pub fn add_entry(&mut self, name: &OsStr, ino: u64) -> DkResult<()> {
        match self.entries.entry(name.to_os_string()) {
            im_hashmap::Entry::Vacant(e) => {
                e.insert(ino);
                self.dirty = true;
                Ok(())
            }
            im_hashmap::Entry::Occupied(_) => Err(format_err!("Entry {:?} already exists.", name)),
        }
    }

    fn log(&self) -> Option<Logger> {
        self.fh.borrow().log()
    }
}

impl Drop for DkDir {
    fn drop(&mut self) {
        // Remove from opened directories
        self.fs_handle()
            .borrow_mut()
            .opened_dirs
            .remove(&self.ino());

        if let Err(e) = self.flush() {
            try_error!(
                self.log(),
                "Failed to write directory of ino {}! {}",
                self.fh.borrow().inode.ino,
                e
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct DkDirHandle {
    pub(crate) inner: Rc<RefCell<DkDir>>,
}

impl Deref for DkDirHandle {
    type Target = RefCell<DkDir>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
