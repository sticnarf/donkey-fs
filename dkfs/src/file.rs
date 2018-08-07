use super::*;
use bincode::{deserialize_from, serialize_into};
use im::hashmap::Entry;
use im::HashMap;
use std::ffi::OsString;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Drop;

#[derive(Debug)]
pub struct DkFile {
    pub(crate) handle: Handle,
    pub(crate) inode: Inode,
    pub(crate) pos: u64,
    pub(crate) flags: Flags,
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

impl DkFile {
    fn log(&self) -> Option<Logger> {
        self.handle.log.clone()
    }
}

#[derive(Debug)]
pub struct Directory {
    pub(crate) df: DkFile,
    pub(crate) entries: HashMap<OsString, u64>,
    pub(crate) dirty: bool,
}

impl Directory {
    pub fn from_file(df: DkFile) -> DkResult<Self> {
        let mut dir = Directory {
            df,
            entries: HashMap::new(),
            dirty: false,
        };
        dir.read_fully()?;
        Ok(dir)
    }

    fn read_fully(&mut self) -> DkResult<()> {
        let mut reader = BufReader::new(&mut self.df);
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

    pub fn flush(&mut self) -> DkResult<()> {
        if self.dirty {
            self.df.seek(SeekFrom::Start(0))?;
            let mut writer = BufWriter::new(&mut self.df);
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
            Entry::Vacant(e) => {
                e.insert(ino);
                self.dirty = true;
                Ok(())
            }
            Entry::Occupied(_) => Err(format_err!("Entry {:?} already exists.", name)),
        }
    }

    fn log(&self) -> Option<Logger> {
        self.df.log()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            try_error!(
                self.log(),
                "Failed to write directory of ino {}! {}",
                self.df.inode.ino,
                e
            );
        }
    }
}
