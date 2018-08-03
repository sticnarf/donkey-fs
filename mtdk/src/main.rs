extern crate clap;
#[macro_use]
extern crate slog;
extern crate dkfs;
#[macro_use]
extern crate failure;
extern crate fuse;
extern crate libc;
extern crate slog_term;
extern crate time;

mod dk2fuse;
mod fuse2dk;

use dkfs::*;
use failure::Error;
use fuse::*;
use libc::*;
use slog::{Drain, Logger};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

fn main() {
    use clap::*;

    let matches = App::new("mtdk")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Mount a donkey filesystem")
        .arg(
            Arg::with_name("device")
                .help("Path to the device to be used")
                .required(true),
        )
        .arg(
            Arg::with_name("dir")
                .help("Path of the mount point")
                .required(true),
        )
        .get_matches();

    let log = logger();
    let dev_path = matches.value_of("device").unwrap();
    let dir = matches.value_of("dir").unwrap();
    let options = [
        "-o",
        "fsname=donkey",
        "-o",
        "allow_other",
        "-o",
        "auto_unmount",
    ].iter()
        .map(|o| OsStr::new(o))
        .collect::<Vec<&OsStr>>();

    let res = DonkeyBuilder::new(dev_path)
        .and_then(|dk| dk.open())
        .and_then(|dk| {
            let fuse = DonkeyFuse {
                dk,
                opened_files: BTreeMap::new(),
                log: log.clone(),
            };
            fuse::mount(fuse, &dir, &options)?;
            Ok(())
        });

    if let Err(e) = res {
        error!(log, "{}", e);
    }
}

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}

type Result<T> = std::result::Result<T, Error>;

const TTL: time::Timespec = time::Timespec { sec: 1, nsec: 0 };

struct DonkeyFuse {
    dk: Donkey,
    opened_files: BTreeMap<u64, DonkeyFile>,
    log: Logger,
}

impl DonkeyFuse {
    fn new_fh(&self) -> u64 {
        loop {
            let fh = get_new_fh();
            if !self.opened_files.contains_key(&fh) {
                return fh;
            }
        }
    }

    fn dk_open(&self, _req: &Request, inode: u64, flags: OpenFlags) -> Result<DonkeyFile> {
        self.dk.open(inode, flags, Some(self.log.clone()))
    }

    // Returns file handle
    // Remember to close fh after calling this method!!!!
    fn dk_open_fh(
        &mut self,
        _req: &Request,
        inode: u64,
        flags: OpenFlags,
    ) -> Result<(u64, OpenFlags)> {
        let dkfile = self.dk_open(_req, inode, flags)?;
        let flags = dkfile.flags;
        let new_fh = self.new_fh();
        self.opened_files.insert(new_fh, dkfile);
        debug!(self.log, "open inode {}, fh: {}", inode, new_fh);
        Ok((new_fh, flags))
    }

    fn dk_find(&mut self, fh: u64) -> Result<&mut DonkeyFile> {
        self.opened_files
            .get_mut(&fh)
            .ok_or(format_err!("fh is not opened"))
    }

    fn dk_close(&mut self, fh: u64) {
        self.opened_files.remove(&fh);
    }

    fn dk_getattr(&self, _req: &Request, ino: u64) -> Result<fuse::FileAttr> {
        let dkfile = self
            .dk
            .open(ino, OpenFlags::READ_ONLY, Some(self.log.clone()))?;
        let attr = dkfile.get_attr()?;
        debug!(self.log, "inode {} attr: {:?}", ino, attr);
        Ok(dk2fuse::attr(attr, ino))
    }

    fn dk_setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<time::Timespec>,
        mtime: Option<time::Timespec>,
        _fh: Option<u64>,
        crtime: Option<time::Timespec>,
        chgtime: Option<time::Timespec>,
        _bkuptime: Option<time::Timespec>,
        _flags: Option<u32>,
    ) -> Result<fuse::FileAttr> {
        let mode = mode.map(|mode| fuse2dk::file_mode(mode));
        let atime = atime.map(|atime| fuse2dk::timespec(atime));
        let mtime = mtime.map(|mtime| fuse2dk::timespec(mtime));
        let ctime = chgtime.map(|ctime| fuse2dk::timespec(ctime));
        let crtime = crtime.map(|crtime| fuse2dk::timespec(crtime));
        let set_attr = SetFileAttr {
            mode,
            uid,
            gid,
            size,
            atime,
            mtime,
            ctime,
            crtime,
            ..Default::default()
        };
        let mut dkfile = self.dk_open(_req, ino, OpenFlags::WRITE_ONLY)?;
        dkfile
            .set_attr(set_attr)
            .map(|attr| dk2fuse::attr(attr, ino))
    }

    fn dk_lookup(&self, _req: &Request, parent: u64, name: &OsStr) -> Result<fuse::FileAttr> {
        let mut dkfile = self.dk_open(_req, parent, OpenFlags::READ_ONLY)?;

        loop {
            let result = dkfile.read_dir()?;
            if let Some(entry) = result {
                if entry.filename == name {
                    let entry_file = self.dk_open(_req, entry.inode, OpenFlags::READ_ONLY)?;
                    let attr = entry_file.get_attr()?;
                    return Ok(dk2fuse::attr(attr, entry.inode));
                }
            } else {
                return Err(format_err!("Cannot find file"));
            }
        }
    }

    // returns (entry, new_offset)
    fn dk_readdir(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
    ) -> Result<Option<(fuse::FileAttr, OsString, i64)>> {
        let (entry, new_offset) = {
            let dkfile = self.dk_find(fh)?;
            dkfile.seek(SeekFrom::Start(offset as u64))?;
            let entry = dkfile.read_dir()?;
            (entry, dkfile.offset as i64)
        };
        match entry {
            Some(entry) => {
                let attr = self.dk_getattr(_req, entry.inode)?;
                Ok(Some((attr, entry.filename, new_offset)))
            }
            None => Ok(None),
        }
    }

    fn dk_mknod(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        rdev: u32,
    ) -> Result<fuse::FileAttr> {
        let inode = self.dk.mknod_raw(
            fuse2dk::file_mode(mode),
            req.uid(),
            req.gid(),
            0,
            Some(rdev as u64),
            Some(self.log.clone()),
        )?;
        debug!(self.log, "Inode {} is created", inode);
        self.dk.link(inode, parent, name, Some(self.log.clone()))?;
        debug!(self.log, "Inode {} is linked to parent {}", inode, parent);
        self.dk_getattr(req, inode)
    }

    fn dk_mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
    ) -> Result<fuse::FileAttr> {
        let inode = self.dk.mkdir_raw(
            parent,
            fuse2dk::file_mode(mode),
            req.uid(),
            req.gid(),
            Some(self.log.clone()),
        )?;
        debug!(self.log, "Inode {} is created", inode);
        self.dk.link(inode, parent, name, Some(self.log.clone()))?;
        debug!(self.log, "Inode {} is linked to parent {}", inode, parent);
        self.dk_getattr(req, inode)
    }

    fn dk_read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
    ) -> Result<Vec<u8>> {
        let dkfile = self.dk_find(fh)?;
        let size = size as usize;
        dkfile.seek(SeekFrom::Start(offset as u64))?;
        let mut buf = vec![0; size];
        match dkfile.read_exact(&mut buf[..]) {
            Ok(()) => Ok(buf),
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                buf.truncate(size);
                Ok(buf)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn dk_write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
    ) -> Result<u32> {
        let dkfile = self.dk_find(fh)?;
        dkfile.seek(SeekFrom::Start(offset as u64))?;
        let written = dkfile.write(data)?;
        Ok(written as u32)
    }
}

impl Filesystem for DonkeyFuse {
    fn lookup(&mut self, _req: &Request, mut parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == FUSE_ROOT_ID {
            parent = self.dk.root_inode();
        }

        debug!(
            self.log,
            "lookup, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        match self.dk_lookup(_req, parent, name) {
            Ok(attr) => {
                debug!(self.log, "lookup attr: {:?}", attr);
                reply.entry(&TTL, &attr, get_new_generation());
            }
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, mut ino: u64, reply: ReplyAttr) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "getattr, inode: {}", ino);

        match self.dk_getattr(_req, ino) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::ENOENT);
            }
        }
    }

    fn setattr(
        &mut self,
        req: &Request,
        mut ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<time::Timespec>,
        mtime: Option<time::Timespec>,
        fh: Option<u64>,
        crtime: Option<time::Timespec>,
        chgtime: Option<time::Timespec>,
        bkuptime: Option<time::Timespec>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "setattr, inode: {}, fh: {:?}", ino, fh);

        match self.dk_setattr(
            req, ino, mode, uid, gid, size, atime, mtime, fh, crtime, chgtime, bkuptime, flags,
        ) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn open(&mut self, _req: &Request, mut ino: u64, flags: u32, reply: ReplyOpen) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "open, inode: {}", ino);

        match self.dk_open_fh(_req, ino, fuse2dk::open_flags(flags)) {
            Ok((fh, flags)) => {
                debug!(self.log, "opened {}, fh: {}, flags: {:?}", ino, fh, flags);
                reply.opened(fh, dk2fuse::open_flags(flags))
            }
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::ENOENT)
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        mut ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }
        info!(self.log, "read, ino: {}, fh: {}, size: {}", ino, fh, size);

        match self.dk_read(_req, ino, fh, offset, size) {
            Ok(data) => reply.data(&data),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        debug!(self.log, "release, fh: {}, ", fh);

        self.dk_close(fh);
        reply.ok();
    }

    fn opendir(&mut self, _req: &Request, mut ino: u64, flags: u32, reply: ReplyOpen) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "opendir, ino: {}", ino);

        match self.dk_open_fh(_req, ino, fuse2dk::open_flags(flags)) {
            Ok((fh, flags)) => {
                debug!(self.log, "opened {}, fh: {}, flags: {:?}", ino, fh, flags);
                reply.opened(fh, dk2fuse::open_flags(flags))
            }
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::ENOENT)
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        mut offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!(
            self.log,
            "readdir, ino: {}, fh: {}, offset: {}", _ino, fh, offset
        );

        loop {
            match self.dk_readdir(_req, _ino, fh, offset) {
                Ok(Some((entry, filename, new_offset))) => {
                    debug!(
                        self.log,
                        "inode {}, new_offset: {}, kind: {:?}, filename: {}",
                        entry.ino,
                        new_offset,
                        entry.kind,
                        filename.to_str().unwrap_or("not valid utf-8")
                    );
                    let full = reply.add(entry.ino, new_offset, entry.kind, filename);
                    if full {
                        debug!(self.log, "buffer full!");
                        return;
                    }
                    offset = new_offset;
                }
                Ok(None) => {
                    debug!(self.log, "readdir reply ok!");
                    reply.ok();
                    return;
                }
                Err(e) => {
                    warn!(self.log, "{:?}", e);
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        }
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, fh: u64, _flags: u32, reply: ReplyEmpty) {
        debug!(self.log, "releasedir, fh: {}", fh);

        self.dk_close(fh);
        reply.ok();
    }

    fn mknod(
        &mut self,
        req: &Request,
        mut parent: u64,
        name: &OsStr,
        mode: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        if parent == FUSE_ROOT_ID {
            parent = self.dk.root_inode();
        }

        debug!(
            self.log,
            "mknod, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        match self.dk_mknod(req, parent, name, mode, rdev) {
            Ok(attr) => reply.entry(&TTL, &attr, get_new_generation()),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn mkdir(
        &mut self,
        req: &Request,
        mut parent: u64,
        name: &OsStr,
        mode: u32,
        reply: ReplyEntry,
    ) {
        if parent == FUSE_ROOT_ID {
            parent = self.dk.root_inode();
        }

        debug!(
            self.log,
            "mkdir, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        match self.dk_mkdir(req, parent, name, mode) {
            Ok(attr) => reply.entry(&TTL, &attr, get_new_generation()),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
        reply: ReplyWrite,
    ) {
        debug!(self.log, "write, fh: {}, {} bytes", fh, data.len());

        match self.dk_write(_req, _ino, fh, offset, data, _flags) {
            Ok(written) => reply.written(written),
            Err(e) => {
                warn!(self.log, "{:?}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        unimplemented!()
    }

    fn fsync(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        debug!(
            self.log,
            "fsync, ino: {}, fh: {}, datasync: {}", _ino, _fh, _datasync
        );
    }

    fn fsyncdir(
        &mut self,
        _req: &Request,
        _ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        debug!(
            self.log,
            "fsync, ino: {}, fh: {}, datasync: {}", _ino, _fh, _datasync
        );
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!(
            self.log,
            "unlink parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid utf-8")
        );
        unimplemented!()
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!(
            self.log,
            "unlink parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid utf-8")
        );
        unimplemented!()
    }

    fn rename(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _newparent: u64,
        _newname: &OsStr,
        reply: ReplyEmpty,
    ) {
        unimplemented!()
    }

    fn init(&mut self, _req: &Request) -> std::result::Result<(), c_int> {
        unimplemented!()
    }

    fn destroy(&mut self, _req: &Request) {
        unimplemented!()
    }

    fn forget(&mut self, _req: &Request, _ino: u64, _nlookup: u64) {
        unimplemented!()
    }

    fn readlink(&mut self, _req: &Request, _ino: u64, reply: ReplyData) {
        unimplemented!()
    }

    fn symlink(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _link: &Path,
        reply: ReplyEntry,
    ) {
        unimplemented!()
    }

    fn statfs(&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) {
        unimplemented!()
    }

    fn setxattr(
        &mut self,
        _req: &Request,
        _ino: u64,
        _name: &OsStr,
        _value: &[u8],
        _flags: u32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        unimplemented!()
    }

    fn getxattr(
        &mut self,
        _req: &Request,
        _ino: u64,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        unimplemented!()
    }

    fn listxattr(&mut self, _req: &Request, _ino: u64, _size: u32, reply: ReplyXattr) {
        unimplemented!()
    }

    fn removexattr(&mut self, _req: &Request, _ino: u64, _name: &OsStr, reply: ReplyEmpty) {
        unimplemented!()
    }

    fn access(&mut self, _req: &Request, _ino: u64, _mask: u32, reply: ReplyEmpty) {
        unimplemented!()
    }

    fn create(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _flags: u32,
        reply: ReplyCreate,
    ) {
        unimplemented!()
    }

    fn getlk(
        &mut self,
        _req: &Request,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: u32,
        _pid: u32,
        reply: ReplyLock,
    ) {
        unimplemented!()
    }

    fn setlk(
        &mut self,
        _req: &Request,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: u32,
        _pid: u32,
        _sleep: bool,
        reply: ReplyEmpty,
    ) {
        unimplemented!()
    }

    fn bmap(&mut self, _req: &Request, _ino: u64, _blocksize: u32, _idx: u64, reply: ReplyBmap) {
        unimplemented!()
    }
}

// TODO?
// Only works in a single thread environment

thread_local!(static GENERATION: Cell<(i64, u64)> = Cell::new((0,0)));

// Generate a unique value for NFS generation
// https://libfuse.github.io/doxygen/structfuse__entry__param.html
fn get_new_generation() -> u64 {
    GENERATION.with(|cell| {
        let (t, g) = cell.get();
        let new_t = time::now().to_timespec().sec;
        let new_g = if t == new_t { g + 1 } else { 1 };
        cell.set((new_t, new_g));
        (new_t as u64) << 26 + new_g
    })
}

thread_local!(static FH: Cell<u64> = Cell::new(1));

// generate a new file handle number
fn get_new_fh() -> u64 {
    FH.with(|cell| {
        let new_fh = cell.get() + 1;
        cell.set(new_fh);
        new_fh
    })
}
