extern crate clap;
#[macro_use]
extern crate slog;
extern crate dkfs;
extern crate failure;
extern crate fuse;
extern crate libc;
extern crate slog_term;
extern crate time;

use dkfs::*;
use fuse::*;
use libc::*;
use slog::{Drain, Logger};
use std::collections::HashMap;
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
        "default_permissions",
        "-o",
        "auto_unmount",
    ]
        .iter()
        .map(|o| OsStr::new(o))
        .collect::<Vec<&OsStr>>();

    dkfs::open(dev_path).and_then(|dk| {
        let fuse = DonkeyFuse {
            dk: dk,
            log,
            dir_fh: HashMap::new(),
            file_fh: HashMap::new(),
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
    dir_fh: HashMap<u64, DkDirHandle>,
    file_fh: HashMap<u64, DkFileHandle>,
}

macro_rules! construct_fmt {
    () => {
        ""
    };
    ($i:ident, $($rem: ident),* $(,)*) => {
        concat!(
            stringify!($i),
            ": {:?}",
            $(
                ", ",
                stringify!($rem),
                ": {:?}",
            )*
        )
    };
}

macro_rules! debug_params {
    ($log:expr; $n:tt; $($i: ident),*) => {
        debug!($log, concat!(
            stringify!($n),
            "(",
            construct_fmt!($($i,)*),
            ")"
            ),
            $($i,)*
        );
    }
}

macro_rules! ino {
    ($($i:ident), *) => {
        $(
            let $i = if $i == FUSE_ROOT_ID {
                ROOT_INODE
            } else {
                $i
            };
        )*
    };
}

impl Filesystem for DonkeyFuse {
    fn init(&mut self, req: &Request) -> std::result::Result<(), c_int> {
        debug_params!(self.log; init; req);
        Ok(())
    }

    fn destroy(&mut self, req: &Request) {
        debug_params!(self.log; destroy; req);
    }

    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        ino![parent];
        debug_params!(self.log; lookup; req, parent, name);
        match self.dk.lookup(parent, name) {
            Ok(stat) => reply.entry(&TTL, &dk2fuse::file_attr(stat), req.unique()),
            Err(e) => {
                info!(self.log, "{}", e);
                reply.error(ENOENT);
            }
        }
    }

    fn forget(&mut self, req: &Request, ino: u64, nlookup: u64) {
        ino![ino];
        debug_params!(self.log; forget; req, ino, nlookup);
        unimplemented!()
    }

    fn getattr(&mut self, req: &Request, ino: u64, reply: ReplyAttr) {
        ino![ino];
        debug_params!(self.log; getattr; req, ino);
        match self.dk.getattr(ino) {
            Ok(stat) => {
                reply.attr(&TTL, &dk2fuse::file_attr(stat));
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(ENOENT);
            }
        }
    }

    fn setattr(
        &mut self,
        req: &Request,
        ino: u64,
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
        debug_params!(self.log; setattr;
            req, ino, mode, uid, gid, size, atime, mtime, fh, crtime, chgtime, bkuptime, flags);
        unimplemented!()
    }

    fn readlink(&mut self, req: &Request, ino: u64, reply: ReplyData) {
        ino![ino];
        debug_params!(self.log; readlink; req, ino);
        unimplemented!()
    }

    fn mknod(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        ino![parent];
        debug_params!(self.log; mknod; req, parent, name, mode, rdev);
        unimplemented!()
    }

    fn mkdir(&mut self, req: &Request, parent: u64, name: &OsStr, mode: u32, reply: ReplyEntry) {
        ino![parent];
        debug_params!(self.log; mkdir; req, parent, name, mode);
        unimplemented!()
    }

    fn unlink(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];
        debug_params!(self.log; unlink; req, parent, name);
        unimplemented!()
    }

    fn rmdir(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];
        debug_params!(self.log; rmdir; req, parent, name);
        unimplemented!()
    }

    fn symlink(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        link: &Path,
        reply: ReplyEntry,
    ) {
        ino![parent];
        debug_params!(self.log; symlink; req, parent, name, link);
        unimplemented!()
    }

    fn rename(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEmpty,
    ) {
        ino![parent, newparent];
        debug_params!(self.log; rename; req, parent, name, newparent, newname);
        unimplemented!()
    }

    fn open(&mut self, req: &Request, ino: u64, flags: u32, reply: ReplyOpen) {
        ino![ino];
        debug_params!(self.log; setattr; req, ino, flags);
        unimplemented!()
    }

    fn read(&mut self, req: &Request, ino: u64, fh: u64, offset: i64, size: u32, reply: ReplyData) {
        ino![ino];
        debug_params!(self.log; read; req, ino, fh, offset, size);
        unimplemented!()
    }

    fn write(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        flags: u32,
        reply: ReplyWrite,
    ) {
        ino![ino];
        debug_params!(self.log; write; req, ino, fh, offset, data, flags);
        unimplemented!()
    }

    fn flush(&mut self, req: &Request, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; flush; req, ino, fh, lock_owner);
        unimplemented!()
    }

    fn release(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        flags: u32,
        lock_owner: u64,
        flush: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];
        debug_params!(self.log; release; req, ino, fh, flags, lock_owner, flush);
        unimplemented!()
    }

    fn fsync(&mut self, req: &Request, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; fsync; req, ino, fh, datasync);
        unimplemented!()
    }

    fn opendir(&mut self, req: &Request, ino: u64, flags: u32, reply: ReplyOpen) {
        ino![ino];
        debug_params!(self.log; opendir; req, ino, flags);
        match self.dk.opendir(ino) {
            Ok(dh) => {
                self.dir_fh.insert(req.unique(), dh);
                reply.opened(req.unique(), flags);
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(ENOENT);
            }
        }
    }

    fn readdir(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        ino![ino];
        debug_params!(self.log; readdir; req, ino, fh, offset);
        let dh = match self.dir_fh.get(&fh) {
            Some(dh) => dh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        for (i, (name, ino)) in self.dk.readdir(dh.clone(), offset as usize).enumerate() {
            match self.dk.getattr(ino) {
                Ok(stat) => {
                    if reply.add(
                        ino,
                        offset + i as i64 + 1,
                        dk2fuse::file_type(stat.mode),
                        name,
                    ) {
                        // Full
                        return;
                    }
                }
                Err(e) => {
                    error!(self.log, "{}", e);
                    reply.error(EIO);
                    return;
                }
            }
        }
        reply.ok();
    }

    fn releasedir(&mut self, req: &Request, ino: u64, fh: u64, flags: u32, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; releasedir; req, ino, fh, flags);
        if let Some(_) = self.dir_fh.remove(&fh) {
            match self.dk.apply_releases() {
                Ok(_) => reply.ok(),
                Err(e) => {
                    error!(self.log, "{}", e);
                    reply.error(EIO);
                }
            }
        }
    }

    fn fsyncdir(&mut self, req: &Request, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; fsyncdir; req, ino, fh, datasync);
        unimplemented!()
    }

    fn statfs(&mut self, req: &Request, ino: u64, reply: ReplyStatfs) {
        ino![ino];
        debug_params!(self.log; statfs; req, ino);
        match self.dk.statfs() {
            Ok(stat) => {
                reply.statfs(
                    stat.blocks,
                    stat.bfree,
                    stat.bavail,
                    stat.files,
                    stat.ffree,
                    stat.bsize as u32,
                    stat.namelen,
                    stat.bsize as u32,
                );
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(EIO);
            }
        }
    }

    fn setxattr(
        &mut self,
        req: &Request,
        ino: u64,
        name: &OsStr,
        value: &[u8],
        flags: u32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        ino![ino];
        debug_params!(self.log; setxattr; req, ino, name, value, flags, position);
        unimplemented!()
    }

    fn getxattr(&mut self, req: &Request, ino: u64, name: &OsStr, size: u32, reply: ReplyXattr) {
        ino![ino];
        debug_params!(self.log; getxattr; req, ino, name, size);
        warn!(self.log, "Extra attributes are not supported.");
        reply.error(ENOTSUP);
    }

    fn listxattr(&mut self, req: &Request, ino: u64, size: u32, reply: ReplyXattr) {
        ino![ino];
        debug_params!(self.log; listxattr; req, ino, size);
        warn!(self.log, "Extra attributes are not supported.");
        reply.error(ENOTSUP);
    }

    fn removexattr(&mut self, req: &Request, ino: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; removexattr; req, ino, name);
        warn!(self.log, "Extra attributes are not supported.");
        reply.error(ENOTSUP);
    }

    fn access(&mut self, req: &Request, ino: u64, mask: u32, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; access; req, ino, mask);
        // This function should not be called with `default_permissions` mount option.
        unimplemented!()
    }

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        flags: u32,
        reply: ReplyCreate,
    ) {
        ino![parent];
        debug_params!(self.log; create; req, parent, name, mode, flags);
        unimplemented!()
    }

    fn getlk(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: u32,
        pid: u32,
        reply: ReplyLock,
    ) {
        ino![ino];
        debug_params!(self.log; getlk; req, ino, fh, lock_owner, start, end, typ, pid);
        unimplemented!()
    }

    fn setlk(
        &mut self,
        req: &Request,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: u32,
        pid: u32,
        sleep: bool,
        reply: ReplyEmpty,
    ) {
        ino![ino];
        debug_params!(self.log; setlk; req, ino, fh, lock_owner, start, end, typ, pid, sleep);
        unimplemented!()
    }

    fn bmap(&mut self, req: &Request, ino: u64, blocksize: u32, idx: u64, reply: ReplyBmap) {
        ino![ino];
        debug_params!(self.log; bmap; req, ino, blocksize, idx);
        unimplemented!()
    }
}

mod dk2fuse;
mod fuse2dk;
