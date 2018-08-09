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

use dkfs::*;
use fuse::*;
use libc::*;
use slog::{Drain, Logger};
use std::ffi::OsStr;
use std::path::Path;

fn main() -> DkResult<()> {
    use clap::*;

    let matches = App::new("mtdk")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Mount a donkey filesystem")
        .arg(
            Arg::with_name("device")
                .help("Path to the device to be used")
                .required(true),
        ).arg(
            Arg::with_name("dir")
                .help("Path of the mount point")
                .required(true),
        ).get_matches();

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
    ]
        .iter()
        .map(|o| OsStr::new(o))
        .collect::<Vec<&OsStr>>();

    Donkey::open(dev_path).and_then(|dk| {
        let fuse = DonkeyFuse {
            dk: dk.log(log.clone()),
            log: log.clone(),
        };
        fuse::mount(fuse, &dir, &options)?;
        Ok(())
    })
}

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}

const TTL: time::Timespec = time::Timespec { sec: 1, nsec: 0 };

struct DonkeyFuse {
    dk: Handle,
    log: Logger,
}

macro_rules! ino {
    ($($i:expr), *) => {
        $(
            if $i == FUSE_ROOT_ID {
                $i = ROOT_INODE
            }
        )*
    };
}

impl Filesystem for DonkeyFuse {
    fn lookup(&mut self, _req: &Request, mut parent: u64, name: &OsStr, reply: ReplyEntry) {
        ino![parent];

        debug!(
            self.log,
            "lookup, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        unimplemented!()
    }

    fn getattr(&mut self, _req: &Request, mut ino: u64, reply: ReplyAttr) {
        ino![ino];

        debug!(self.log, "getattr, inode: {}", ino);

        unimplemented!()
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
        ino![ino];

        debug!(self.log, "setattr, inode: {}, fh: {:?}", ino, fh);

        unimplemented!()
    }

    fn open(&mut self, _req: &Request, mut ino: u64, flags: u32, reply: ReplyOpen) {
        ino![ino];

        debug!(self.log, "open, inode: {}", ino);

        unimplemented!()
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
        ino![ino];

        debug!(self.log, "read, ino: {}, fh: {}, size: {}", ino, fh, size);

        unimplemented!()
    }

    fn release(
        &mut self,
        _req: &Request,
        mut ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];

        debug!(self.log, "release, fh: {}, ", fh);

        unimplemented!()
    }

    fn opendir(&mut self, _req: &Request, mut ino: u64, flags: u32, reply: ReplyOpen) {
        ino![ino];

        debug!(self.log, "opendir, ino: {}", ino);

        unimplemented!()
    }

    fn readdir(
        &mut self,
        _req: &Request,
        mut ino: u64,
        fh: u64,
        mut offset: i64,
        mut reply: ReplyDirectory,
    ) {
        ino![ino];

        debug!(
            self.log,
            "readdir, ino: {}, fh: {}, offset: {}", ino, fh, offset
        );

        unimplemented!()
    }

    fn releasedir(
        &mut self,
        _req: &Request,
        mut ino: u64,
        fh: u64,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        ino![ino];

        debug!(self.log, "releasedir, fh: {}", fh);

        unimplemented!()
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
        ino![parent];

        debug!(
            self.log,
            "mknod, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        unimplemented!()
    }

    fn mkdir(
        &mut self,
        req: &Request,
        mut parent: u64,
        name: &OsStr,
        mode: u32,
        reply: ReplyEntry,
    ) {
        ino![parent];

        debug!(
            self.log,
            "mkdir, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );

        unimplemented!()
    }

    fn write(
        &mut self,
        _req: &Request,
        mut ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
        reply: ReplyWrite,
    ) {
        ino![ino];

        debug!(self.log, "write, fh: {}, {} bytes", fh, data.len());

        unimplemented!()
    }

    fn flush(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        ino![ino];

        unimplemented!()
    }

    fn fsync(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];

        debug!(
            self.log,
            "fsync, ino: {}, fh: {}, datasync: {}", ino, _fh, _datasync
        );
    }

    fn fsyncdir(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];

        debug!(
            self.log,
            "fsync, ino: {}, fh: {}, datasync: {}", ino, _fh, _datasync
        );
    }

    fn unlink(&mut self, _req: &Request, mut parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];

        debug!(
            self.log,
            "unlink parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid utf-8")
        );

        unimplemented!()
    }

    fn rmdir(&mut self, _req: &Request, mut parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];

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
        mut parent: u64,
        _name: &OsStr,
        mut newparent: u64,
        _newname: &OsStr,
        reply: ReplyEmpty,
    ) {
        ino![parent, newparent];

        unimplemented!()
    }

    fn init(&mut self, _req: &Request) -> std::result::Result<(), c_int> {
        unimplemented!()
    }

    fn destroy(&mut self, _req: &Request) {
        unimplemented!()
    }

    fn forget(&mut self, _req: &Request, mut ino: u64, _nlookup: u64) {
        ino![ino];

        unimplemented!()
    }

    fn readlink(&mut self, _req: &Request, mut ino: u64, reply: ReplyData) {
        ino![ino];

        unimplemented!()
    }

    fn symlink(
        &mut self,
        _req: &Request,
        mut parent: u64,
        _name: &OsStr,
        _link: &Path,
        reply: ReplyEntry,
    ) {
        ino![parent];
        unimplemented!()
    }

    fn statfs(&mut self, _req: &Request, mut ino: u64, reply: ReplyStatfs) {
        ino![ino];
        unimplemented!()
    }

    fn setxattr(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _name: &OsStr,
        _value: &[u8],
        _flags: u32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        ino![ino];
        unimplemented!()
    }

    fn getxattr(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        ino![ino];
        unimplemented!()
    }

    fn listxattr(&mut self, _req: &Request, mut ino: u64, _size: u32, reply: ReplyXattr) {
        ino![ino];
        unimplemented!()
    }

    fn removexattr(&mut self, _req: &Request, mut ino: u64, _name: &OsStr, reply: ReplyEmpty) {
        ino![ino];
        unimplemented!()
    }

    fn access(&mut self, _req: &Request, mut ino: u64, _mask: u32, reply: ReplyEmpty) {
        ino![ino];
        unimplemented!()
    }

    fn create(
        &mut self,
        _req: &Request,
        mut parent: u64,
        _name: &OsStr,
        _mode: u32,
        _flags: u32,
        reply: ReplyCreate,
    ) {
        ino![parent];
        unimplemented!()
    }

    fn getlk(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: u32,
        _pid: u32,
        reply: ReplyLock,
    ) {
        ino![ino];
        unimplemented!()
    }

    fn setlk(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: u32,
        _pid: u32,
        _sleep: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];
        unimplemented!()
    }

    fn bmap(&mut self, _req: &Request, mut ino: u64, _blocksize: u32, _idx: u64, reply: ReplyBmap) {
        ino![ino];
        unimplemented!()
    }
}

mod dk2fuse;
mod fuse2dk;
mod ops;
