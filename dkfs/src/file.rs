use bincode::{deserialize_from, serialize_into};
use block::*;
use im::ordmap::{self, OrdMap};
use std::cell::RefCell;
use std::cmp::min;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use std::ops::Drop;
use std::rc::Rc;
use *;

#[derive(Debug)]
pub struct DkFile {
    /// Only used in `drop`
    pub(crate) inode: Inode,
    pub(crate) pos: u64,
    pub(crate) dirty: bool,
    pub(crate) close_file_list: Rc<RefCell<Vec<u64>>>,
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
        if self.file.dirty {
            if let Err(e) = self.dk.write_inode(&self.file.inode) {
                return Err(io::Error::new(
                    ErrorKind::Other,
                    format!("Failed to flush file of ino {}! {}", self.file.inode.ino, e),
                ));
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

impl DkFile {
    pub(crate) fn new(inode: Inode, close_file_list: Rc<RefCell<Vec<u64>>>) -> Self {
        DkFile {
            inode: inode,
            pos: 0,
            dirty: false,
            close_file_list,
        }
    }

    /// Specify the position of the file.
    /// Returns the real pointer and how many bytes in maximum
    /// you can read or write from this pointer.
    /// This function allocates blocks if necessary.
    fn locate(&mut self, dk: &mut Donkey, pos: u64) -> DkResult<(u64, u64)> {
        fn locate_rec(
            dk: &mut Donkey,
            ptrs: &mut [u64],
            level: u32,
            off: u64,
            block_count: &mut u64,
            bs: u64,
        ) -> DkResult<(u64, u64)> {
            fn ptr_or_allocate(
                dk: &mut Donkey,
                ptr: &mut u64,
                block_count: &mut u64,
                bs: u64,
                zero: bool,
            ) -> DkResult<u64> {
                if *ptr == 0 {
                    *ptr = dk.allocate_db()?;
                    if zero {
                        dk.fill_zero(*ptr, bs)?;
                    }
                    *block_count += 1;
                }
                Ok(*ptr)
            }
            let pc = bs / 8; // pointer count in a single block
            let sz = bs * pc.pow(level); // size of all blocks through the direct or indirect pointer
            let i = off / sz; // index in the current level
            let block_off = off % sz;
            if level == 0 {
                let ptr = ptr_or_allocate(dk, &mut ptrs[i as usize], block_count, bs, false)?;
                Ok((ptr + block_off, bs - block_off))
            } else {
                let ptr = ptr_or_allocate(dk, &mut ptrs[i as usize], block_count, bs, true)?;
                // TODO Add cache, delay writing
                let mut ptrs: PtrBlock = dk.read_block(ptr)?;
                let res = locate_rec(dk, &mut ptrs[..], level - 1, block_off, block_count, bs);
                dk.write(ptr, &ptrs)?;
                res
            }
        }

        let bs = dk.block_size();
        let ptrs = &mut self.inode.ptrs;
        let (level, off) = ptrs.locate(pos, bs);
        locate_rec(dk, &mut ptrs[level], level, off, &mut self.inode.blocks, bs)
    }

    fn dk_read(&mut self, dk: &mut Donkey, buf: &mut [u8]) -> DkResult<usize> {
        if self.pos >= self.inode.size {
            return Ok(0);
        }
        let (ptr, len) = self.locate(dk, self.pos)?;
        let len = min(len, self.inode.size - self.pos); // Cannot read beyond EOF
        let len = min(len as usize, buf.len());
        let read_len = dk.read_into(ptr, &mut buf[..len])?;
        self.pos += read_len;
        Ok(read_len as usize)
    }

    fn dk_write(&mut self, dk: &mut Donkey, buf: &[u8]) -> DkResult<usize> {
        self.dirty = true;
        let (ptr, len) = self.locate(dk, self.pos)?;
        let len = min(len as usize, buf.len());
        dk.write(ptr, &RefData(&buf[..len]))?;
        self.pos += len as u64;
        if self.pos > self.inode.size {
            self.update_size(self.pos)?;
        }
        Ok(len)
    }

    pub(crate) fn flush(&mut self, dk: &mut Donkey) -> DkResult<()> {
        let mut io = DkFileIO { dk, file: self };
        Ok(io.flush()?)
    }

    pub(crate) fn update_size(&mut self, new_size: u64) -> DkResult<()> {
        self.inode.size = new_size;
        Ok(())
        // TODO Release blocks when shrinking
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
