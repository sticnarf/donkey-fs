use file::*;
use replies::*;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::rc::Rc;
use *;

#[derive(Debug, Clone)]
pub struct Handle<'a> {
    pub(crate) inner: Rc<RefCell<Donkey<'a>>>,
}

impl<'a> Handle<'a> {
    pub(crate) fn new(dk: Donkey<'a>) -> Self {
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
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let dir = self.opendir(parent)?;
        match dir.entries.get(name) {
            Some(ino) => self.getattr(*ino),
            None => Err(NotFound),
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
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let parent = self.opendir(parent)?;
        let ino = self.inner.borrow_mut().mknod(mode, uid, gid, 0, None)?;
        self.inner.borrow_mut().link(ino, parent, name)?;
        self.getattr(ino)
    }

    pub fn link(&self, ino: u64, parent: u64, name: &OsStr) -> DkResult<Stat> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let parent = self.opendir(parent)?;
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
        mut ctime: Option<DkTimespec>,
        crtime: Option<DkTimespec>,
    ) -> DkResult<Stat> {
        let fh = match fh {
            Some(fh) => fh,
            None => self.open(ino, Flags::READ_ONLY)?,
        };
        let mut modified = false;
        fh.borrow_mut().dirty = true;
        macro_rules! setattrs {
            ($($i:ident),*) => {
                $(
                if let Some(v) = $i {
                    fh.borrow_mut().inode.$i = v;
                    modified = true;
                })*
            };
        }
        setattrs![mode, uid, gid, atime, mtime, crtime];
        if let Some(size) = size {
            let dk = &mut *self.inner.borrow_mut();
            fh.borrow_mut().update_size(dk, size)?;
            modified = true;
        }

        // Update ctime
        if modified && ctime.is_none() {
            ctime = Some(SystemTime::now().into());
        }
        if let Some(ctime) = ctime {
            fh.borrow_mut().inode.ctime = ctime;
        }

        self.getattr(ino)
    }

    pub fn read(&self, fh: DkFileHandle, offset: u64, size: u64) -> DkResult<Vec<u8>> {
        let dk = &mut *self.inner.borrow_mut();
        fh.inner.borrow_mut().seek(SeekFrom::Start(offset))?;
        let file = &mut *fh.inner.borrow_mut();
        let io = DkFileIO { dk, file };
        let mut v = Vec::new();
        let len = io.take(size).read_to_end(&mut v)?;
        v.truncate(len);
        Ok(v)
    }

    pub fn write(&self, fh: DkFileHandle, offset: u64, data: &[u8]) -> DkResult<usize> {
        let dk = &mut *self.inner.borrow_mut();
        fh.inner.borrow_mut().seek(SeekFrom::Start(offset))?;
        let file = &mut *fh.inner.borrow_mut();
        let mut io = DkFileIO { dk, file };
        io.write_all(data)?;
        Ok(data.len())
    }

    pub fn mkdir(
        &self,
        parent: u64,
        uid: u32,
        gid: u32,
        name: &OsStr,
        mode: FileMode,
    ) -> DkResult<Stat> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let ino = self.inner.borrow_mut().mkdir(parent, mode, uid, gid)?;
        let parent = self.opendir(parent)?;
        self.inner.borrow_mut().link(ino, parent, name)?;
        self.getattr(ino)
    }

    pub fn getxattr(&self, ino: u64, name: &OsStr) -> DkResult<Option<Vec<u8>>> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let fh = self.open(ino, Flags::READ_ONLY)?;
        let fh = fh.borrow();
        Ok(fh.xattr.get(name).map(|v| v.clone()))
    }

    pub fn listxattr(&self, ino: u64) -> DkResult<Vec<OsString>> {
        let fh = self.open(ino, Flags::READ_ONLY)?;
        let v = fh.borrow().xattr.keys().map(|key| key.to_owned()).collect();
        Ok(v)
    }

    pub fn setxattr(&self, ino: u64, name: &OsStr, value: &[u8]) -> DkResult<()> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let fh = self.open(ino, Flags::READ_ONLY)?;
        fh.borrow_mut().dirty = true;
        fh.borrow_mut()
            .xattr
            .insert(name.to_owned(), Vec::from(value));
        Ok(())
    }

    pub fn removexattr(&self, ino: u64, name: &OsStr) -> DkResult<()> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let fh = self.open(ino, Flags::READ_ONLY)?;
        fh.borrow_mut().dirty = true;
        fh.borrow_mut().xattr.remove(name);
        Ok(())
    }

    pub fn fsync(&self, fh: DkFileHandle, datasync: bool) -> DkResult<()> {
        // Now we do not support data cache, so only metadata needs synchronizing
        if !datasync {
            self.flush(fh)?;
        }
        Ok(())
    }

    pub fn fsyncdir(&self, dh: DkDirHandle, datasync: bool) -> DkResult<()> {
        let dk = &mut *self.inner.borrow_mut();
        dh.borrow_mut().flush(dk)?;
        self.fsync(dh.borrow().fh.clone(), datasync)
    }

    pub fn unlink(&self, parent: u64, name: &OsStr) -> DkResult<()> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let dh = self.opendir(parent)?;
        self.inner.borrow_mut().unlink(dh, name)
    }

    pub fn rename(
        &self,
        old_parent: u64,
        name: &OsStr,
        new_parent: u64,
        new_name: &OsStr,
    ) -> DkResult<()> {
        if name.len() > MAX_NAMELEN as usize || new_name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let ino = self.lookup(old_parent, name)?.ino;
        let new_parent = self.opendir(new_parent)?;
        self.inner.borrow_mut().link(ino, new_parent, new_name)?;
        self.unlink(old_parent, name)?;
        Ok(())
    }

    pub fn rmdir(&self, parent: u64, name: &OsStr) -> DkResult<()> {
        let dir = self.lookup(parent, name)?;
        let ino = dir.ino;
        let dir = self.opendir(ino)?;
        if dir.entries.len() == 2 {
            // dir only contains . and ..
            self.unlink(ino, OsStr::new("."))?;
            self.unlink(ino, OsStr::new(".."))?;
            self.unlink(parent, name)
        } else {
            Err(NotEmpty)
        }
    }

    pub fn symlink(
        &self,
        uid: u32,
        gid: u32,
        parent: u64,
        name: &OsStr,
        link: &Path,
    ) -> DkResult<Stat> {
        if name.len() > MAX_NAMELEN as usize {
            return Err(NameTooLong);
        }
        let stat = self.mknod(
            uid,
            gid,
            parent,
            name,
            FileMode::SYMBOLIC_LINK
                | FileMode::USER_RWX
                | FileMode::GROUP_RWX
                | FileMode::OTHERS_RWX,
        )?;
        let fh = self.open(stat.ino, Flags::WRITE_ONLY)?;
        let bytes = link.as_os_str().as_bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            offset += self.write(fh.clone(), offset as u64, &bytes[offset..])?;
        }
        self.getattr(stat.ino)
    }
}
