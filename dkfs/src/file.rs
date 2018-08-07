use super::*;
use bincode::{deserialize_from, serialize_into};
use im::hashmap::{self as im_hashmap, HashMap as ImHashMap};
use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::{DerefMut, Drop};
use std::rc::{Rc, Weak};

#[derive(Debug)]
pub struct DkFile {
    pub(crate) handle: Handle,
    pub(crate) inode: Inode,
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
        unimplemented!()
    }
}

impl Seek for DkFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        unimplemented!()
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
    fn log(&self) -> Option<Logger> {
        self.handle.log.clone()
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
