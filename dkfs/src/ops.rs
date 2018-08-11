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

    pub fn mknod(
        &self,
        uid: u32,
        gid: u32,
        parent: u64,
        name: &OsStr,
        mode: FileMode,
    ) -> DkResult<Stat> {
        let parent = self.opendir(parent)?;
        let ino = self.inner.borrow_mut().mknod(mode, uid, gid, 0, None)?;
        self.inner.borrow_mut().link(ino, parent, name)?;
        self.getattr(ino)
    }

    pub fn open(&self, ino: u64, flags: Flags) -> DkResult<DkFileHandle> {
        self.inner.borrow_mut().open(ino, flags)
    }

    pub fn flush(&self, fh: DkFileHandle) -> DkResult<()> {
        let dk = &mut *self.inner.borrow_mut();
        fh.inner.borrow_mut().flush(dk)
    }

    pub fn setattr(
        &self,
        ino: u64,
        fh: Option<DkFileHandle>,
        mode: Option<FileMode>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<DkTimespec>,
        mtime: Option<DkTimespec>,
        ctime: Option<DkTimespec>,
        crtime: Option<DkTimespec>,
    ) -> DkResult<Stat> {
        let fh = match fh {
            Some(fh) => fh,
            None => self.open(ino, Flags::READ_ONLY)?,
        };
        fh.borrow_mut().dirty = true;
        macro_rules! setattrs {
            ($($i:ident),*) => {
                $(
                if let Some(v) = $i {
                    fh.borrow_mut().inode.$i = v;
                })*
            };
        }
        setattrs![mode, uid, gid, atime, mtime, ctime, crtime];
        if let Some(size) = size {
            fh.borrow_mut().update_size(size)?;
        }
        self.getattr(ino)
    }
}