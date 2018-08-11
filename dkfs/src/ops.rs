use replies::*;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::rc::Rc;
use *;

#[derive(Debug, Clone)]
pub struct Handle {
    inner: Rc<RefCell<Donkey>>,
}

impl Handle {
    pub(crate) fn new(dk: Donkey) -> Self {
        Handle {
            inner: Rc::new(RefCell::new(dk)),
        }
    }

    pub fn statfs(&self) -> DkResult<Statvfs> {
        let ref sb = self.inner.borrow().sb;
        let stat = Statvfs {
            blocks: sb.db_count,
            bfree: sb.db_count - sb.used_db_count,
            bavail: sb.db_count - sb.used_db_count,
            files: sb.inode_count,
            ffree: sb.inode_count - sb.used_inode_count,
            bsize: sb.block_size,
            namelen: MAX_NAMELEN,
        };
        Ok(stat)
    }

    pub fn getattr(&self, ino: u64) -> DkResult<Stat> {
        let f = self.inner.borrow_mut().open(ino, Flags::READ_ONLY)?;
        let statfs = self.statfs()?;
        let ref inode = f.inner.borrow_mut().inode;
        let stat = Stat {
            ino,
            mode: inode.mode,
            size: inode.size,
            blksize: statfs.bsize as u32,
            blocks: inode.blocks * (statfs.bsize / 512),
            atime: inode.atime,
            mtime: inode.mtime,
            ctime: inode.ctime,
            crtime: inode.crtime,
            nlink: inode.nlink,
            uid: inode.uid,
            gid: inode.gid,
            rdev: inode.device,
        };
        Ok(stat)
    }

    pub fn opendir(&self, ino: u64) -> DkResult<DkDirHandle> {
        self.inner.borrow_mut().open_dir(ino)
    }

    pub fn apply_releases(&self) -> DkResult<()> {
        self.inner.borrow_mut().close_dirs_in_list()?;
        self.inner.borrow_mut().close_files_in_list()?;
        Ok(())
    }

    pub fn lookup(&self, parent: u64, name: &OsStr) -> DkResult<Stat> {
        let dir = self.opendir(parent)?;
        match dir.entries.get(name) {
            Some(ino) => self.getattr(*ino),
            None => Err(format_err!("No such directory entry.")),
        }
    }

    pub fn readdir(
        &self,
        dir: DkDirHandle,
        offset: usize,
    ) -> impl Iterator<Item = (OsString, u64)> {
        dir.entries.skip(offset).skip(offset).into_iter()
    }
}
