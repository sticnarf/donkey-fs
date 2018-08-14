use bincode::{deserialize_from, serialize_into};
use block::*;
use im::ordmap::{self, OrdMap};
use std::cell::RefCell;
use std::cmp::min;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Read, Seek, SeekFrom, Write};
use std::ops::Drop;
use std::rc::Rc;
use *;

#[derive(Debug)]
pub struct DkFile {
    pub(crate) inode: Inode,
    pub(crate) pos: u64,
    pub(crate) xattr: OrdMap<OsString, Vec<u8>>,
    pub(crate) dirty: bool,
    pub(crate) close_file_list: Rc<RefCell<Vec<u64>>>,
    pub(crate) ptr_cache: [Option<(u64, PtrBlock)>; 4],
}

#[derive(Debug)]
pub struct DkFileIO<'a, 'b: 'a> {
    pub(crate) dk: &'a mut Donkey<'b>,
    pub(crate) file: &'a mut DkFile,
}

impl<'a, 'b: 'a> Read for DkFileIO<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.file.dk_read(self.dk, buf) {
            Ok(len) => Ok(len),
            Err(e) => Err(io::Error::new(ErrorKind::Other, format!("{}", e))),
        }
    }
}

impl<'a, 'b: 'a> Write for DkFileIO<'a, 'b> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.file.dk_write(self.dk, buf) {
            Ok(len) => Ok(len),
            Err(e) => Err(io::Error::new(ErrorKind::Other, format!("{}", e))),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.file.dirty && self.file.inode.nlink > 0 {
            let res = self
                .file
                .write_ptr_cache(self.dk)
                .and_then(|_| self.file.write_xattr(self.dk))
                .and_then(|_| self.dk.write_inode(&self.file.inode));
            if let Err(e) = res {
                return Err(io::Error::new(
                    ErrorKind::Other,
                    format!("Failed to flush file of ino {}! {}", self.file.inode.ino, e),
                ));
            }
            self.file.dirty = false;
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

impl DkFile {
    pub(crate) fn new(inode: Inode, close_file_list: Rc<RefCell<Vec<u64>>>) -> Self {
        DkFile {
            inode: inode,
            pos: 0,
            xattr: OrdMap::new(),
            dirty: false,
            close_file_list,
            ptr_cache: Default::default(),
        }
    }

    pub(crate) fn read_xattr(&mut self, dk: &mut Donkey) -> DkResult<()> {
        if self.inode.xattr_ptr != 0 {
            let data: ByteData = dk.read_block(self.inode.xattr_ptr)?;
            let mut reader = Cursor::new(data.0.as_slice());
            loop {
                let key: OsString = deserialize_from(&mut reader)?;
                if key.len() == 0 {
                    break;
                }
                let value: Vec<u8> = deserialize_from(&mut reader)?;
                self.xattr.insert(key, value);
            }
        }
        Ok(())
    }

    pub(crate) fn write_xattr(&mut self, dk: &mut Donkey) -> DkResult<()> {
        if self.xattr.len() == 0 {
            if self.inode.xattr_ptr != 0 {
                dk.free_db(self.inode.xattr_ptr)?;
                self.inode.xattr_ptr = 0;
                self.dirty = true;
            }
        } else {
            if self.inode.xattr_ptr == 0 {
                self.inode.xattr_ptr = dk.allocate_db()?;
                self.inode.blocks += 1;
            }
            let mut data = Vec::new();
            for (key, value) in &self.xattr {
                serialize_into(&mut data, key)?;
                serialize_into(&mut data, value)?;
            }
            serialize_into(&mut data, &OsString::new())?;
            dk.write(self.inode.xattr_ptr, &RefData(data.as_slice()))?;
        }
        Ok(())
    }

    /// Get the block index and offset at `pos`
    fn pos_to_block(pos: u64, bs: u64) -> (u64, u64) {
        (pos / bs, pos % bs)
    }

    /// Get the next block index after pos
    fn pos_to_next_block(pos: u64, bs: u64) -> u64 {
        (pos + bs - 1) / pos
    }

    fn level_off(&self, dk: &mut Donkey, bi: u64) -> (usize, usize) {
        let mut bi = bi as usize;
        let pc = dk.block_size() as usize / 8;
        let mut multi = 1;
        for level in 0..=4 {
            let len = self.inode.ptrs[level].len() * multi;
            if bi < len {
                return (level, bi);
            }
            bi -= len;
            multi *= pc;
        }
        unreachable!()
    }

    fn write_ptr_cache(&mut self, dk: &mut Donkey) -> DkResult<()> {
        for cache in &self.ptr_cache {
            if let Some((ptr, cache)) = cache {
                dk.write(*ptr, cache)?;
            }
        }
        Ok(())
    }

    /// Returns cache ptr
    fn load_ptrs_alloc(&mut self, dk: &mut Donkey, level: usize, ptr: u64) -> DkResult<(u64)> {
        assert!(level > 0);
        if let Some((p, pb)) = &self.ptr_cache[level - 1] {
            if *p == ptr {
                return Ok(ptr);
            } else {
                dk.write(*p, pb)?;
            }
        }
        if ptr == 0 {
            let ptr = dk.allocate_db()?;
            self.ptr_cache[level - 1] = Some((ptr, Self::empty_ptr_block(dk)));
            Ok(ptr)
        } else {
            self.ptr_cache[level - 1] = Some((ptr, dk.read_block(ptr)?));
            Ok(ptr)
        }
    }

    fn load_ptrs_in_cache_alloc(
        &mut self,
        dk: &mut Donkey,
        level: usize,
        index: usize,
    ) -> DkResult<()> {
        assert!(level > 1);
        let (_, cache) = self.ptr_cache[level - 1].as_mut().unwrap();
        let empty = cache.0[index] == 0;
        if empty {
            cache.0[index] = dk.allocate_db()?;
        }
        let new_ptr = cache.0[index];
        if let Some((p, pb)) = &self.ptr_cache[level - 2] {
            if *p == new_ptr {
                return Ok(());
            } else {
                dk.write(*p, pb)?;
            }
        }
        let cache = if empty {
            Self::empty_ptr_block(dk)
        } else {
            dk.read_block(new_ptr)?
        };
        self.ptr_cache[level - 2] = Some((new_ptr, cache));
        Ok(())
    }

    fn empty_ptr_block(dk: &Donkey) -> PtrBlock {
        Data(vec![0; dk.block_size() as usize / 8])
    }

    fn locate_alloc(&mut self, dk: &mut Donkey, bi: u64) -> DkResult<u64> {
        let (mut level, mut off) = self.level_off(dk, bi);
        if level == 0 {
            if self.inode.ptrs[level][off] == 0 {
                self.inode.ptrs[level][off] = dk.allocate_db()?;
            }
            Ok(self.inode.ptrs[level][off])
        } else {
            self.inode.ptrs[level][0] =
                self.load_ptrs_alloc(dk, level, self.inode.ptrs[level][0])?;
            let pc = dk.block_size() as usize / 8;
            let mut ipc = pc.pow(level as u32);
            while level > 1 {
                ipc /= pc;
                off %= ipc;
                self.load_ptrs_in_cache_alloc(dk, level, off / ipc)?;
                level -= 1;
            }
            let ref mut cache = self.ptr_cache[0].as_mut().unwrap().1;
            if cache.0[off] == 0 {
                cache.0[off] = dk.allocate_db()?;
            }
            Ok(cache.0[off])
        }
    }

    fn dk_read(&mut self, dk: &mut Donkey, buf: &mut [u8]) -> DkResult<usize> {
        if self.pos >= self.inode.size {
            return Ok(0);
        }
        let bs = dk.block_size();
        let (bi, bo) = Self::pos_to_block(self.pos, bs);
        let ptr = self.locate_alloc(dk, bi)? + bo;
        let len = min(bs - bo, self.inode.size - self.pos); // Cannot read beyond EOF
        let len = min(len as usize, buf.len());
        let read_len = dk.read_into(ptr, &mut buf[..len])?;
        self.pos += read_len;
        Ok(read_len as usize)
    }

    fn dk_write(&mut self, dk: &mut Donkey, buf: &[u8]) -> DkResult<usize> {
        self.dirty = true;
        let bs = dk.block_size();
        let (bi, bo) = Self::pos_to_block(self.pos, bs);
        let ptr = self.locate_alloc(dk, bi)? + bo;
        let len = min((bs - bo) as usize, buf.len());
        dk.write(ptr, &RefData(&buf[..len]))?;
        self.pos += len as u64;
        if self.pos > self.inode.size {
            self.update_size(dk, self.pos)?;
        }
        Ok(len)
    }

    pub(crate) fn flush(&mut self, dk: &mut Donkey) -> DkResult<()> {
        let mut io = DkFileIO { dk, file: self };
        Ok(io.flush()?)
    }

    pub(crate) fn update_size(&mut self, dk: &mut Donkey, new_size: u64) -> DkResult<()> {
        let old_size = self.inode.size;
        self.inode.size = new_size;
        self.dirty = true;
        if old_size > new_size {
            let bs = dk.block_size();
            let free_from = (old_size + bs - 1) / bs;
            let free_to = (new_size + bs - 1) / bs;
            self.free_file_db(dk, free_from, free_to)?;
        }
        Ok(())
        // TODO Release blocks when shrinking
    }

    /// `from` is inclusive, `to` is exclusive
    fn free_file_db(&mut self, dk: &mut Donkey, from: u64, to: u64) -> DkResult<()> {
        unimplemented!()
    }

    pub(crate) fn destroy(&mut self, dk: &mut Donkey) -> DkResult<()> {
        assert_eq!(self.inode.nlink, 0);
        self.update_size(dk, 0)?; // Release used blocks
        if self.inode.xattr_ptr != 0 {
            dk.free_db(self.inode.xattr_ptr)?;
        }
        dk.free_inode(self.inode.ino)
    }
}

#[derive(Debug, Clone)]
pub struct DkFileHandle {
    pub(crate) inner: Rc<RefCell<DkFile>>,
    pub flags: Flags,
}

impl Deref for DkFileHandle {
    type Target = RefCell<DkFile>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Drop for DkFileHandle {
    fn drop(&mut self) {
        let ino = self.borrow().inode.ino;
        self.borrow().close_file_list.borrow_mut().push(ino);
    }
}

#[derive(Debug)]
pub struct DkDir {
    /// Only used in `drop`
    pub(crate) fh: DkFileHandle,
    pub(crate) entries: OrdMap<OsString, u64>,
    pub(crate) dirty: bool,
    pub(crate) close_dir_list: Rc<RefCell<Vec<u64>>>,
}

impl DkDir {
    pub(crate) fn from_file(
        fh: DkFileHandle,
        close_dir_list: Rc<RefCell<Vec<u64>>>,
    ) -> DkResult<Self> {
        if !fh.borrow().inode.mode.is_directory() {
            Err(format_err!("Not a directory."))
        } else {
            let dir = DkDir {
                fh,
                entries: OrdMap::new(),
                dirty: false,
                close_dir_list,
            };
            Ok(dir)
        }
    }

    pub(crate) fn flush(&mut self, dk: &mut Donkey) -> DkResult<()> {
        if self.dirty {
            self.fh.borrow_mut().seek(SeekFrom::Start(0))?;
            let mut io = DkFileIO {
                dk,
                file: &mut *self.fh.borrow_mut(),
            };
            let mut writer = BufWriter::new(&mut io);
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

    pub(crate) fn read_fully(&mut self, dk: &mut Donkey) -> DkResult<()> {
        if self.fh.borrow().inode.size == 0 {
            // Directory is just created
            return Ok(());
        }

        let file = &mut *self.fh.borrow_mut();
        let io = DkFileIO { file, dk };
        let mut reader = BufReader::new(io);
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
}

#[derive(Debug, Clone)]
pub struct DkDirHandle {
    pub(crate) inner: Rc<RefCell<DkDir>>,
    pub(crate) entries: OrdMap<OsString, u64>,
}

impl Deref for DkDirHandle {
    type Target = RefCell<DkDir>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DkDirHandle {
    pub(crate) fn add_entry(&self, name: &OsStr, ino: u64) -> DkResult<()> {
        let res = match self.borrow_mut().entries.entry(name.to_os_string()) {
            ordmap::Entry::Vacant(e) => {
                e.insert(ino);
                Ok(())
            }
            ordmap::Entry::Occupied(_) => Err(format_err!("Entry {:?} already exists.", name)),
        };
        self.borrow_mut().dirty = true;
        res
    }
}

impl Drop for DkDirHandle {
    fn drop(&mut self) {
        let ino = self.inner.borrow().fh.borrow().inode.ino;
        self.borrow().close_dir_list.borrow_mut().push(ino);
    }
}
