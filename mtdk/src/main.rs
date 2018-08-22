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
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

fn main() -> DkResult<()> {
    use clap::*;

    let matches = App::new("mtdk")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Mount a donkey file system")
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

    let dk = dkfs::open(dev(dev_path)?)?;
    let fuse = DonkeyFuse {
        dk: dk,
        log,
        dir_fh: HashMap::new(),
        file_fh: HashMap::new(),
    };
    fuse::mount(fuse, &dir, &options)?;
    Ok(())
}

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}

const TTL: time::Timespec = time::Timespec { sec: 1, nsec: 0 };

struct DonkeyFuse<'a> {
    dk: Handle<'a>,
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

impl<'a> Filesystem for DonkeyFuse<'a> {
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
                match &e {
                    DkError::NotFound => {}
                    _ => error!(self.log, "{}", e),
                }
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn forget(&mut self, req: &Request, ino: u64, nlookup: u64) {
        ino![ino];
        debug_params!(self.log; forget; req, ino, nlookup);
        // Nothing to do
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
                reply.error(dk2fuse::errno(&e));
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
        let fh = fh.and_then(|fh| self.file_fh.get(&fh)).map(|fh| fh.clone());
        match self.dk.setattr(
            ino,
            fh,
            mode.map(fuse2dk::file_mode),
            uid,
            gid,
            size,
            atime.map(fuse2dk::timespec),
            mtime.map(fuse2dk::timespec),
            chgtime.map(fuse2dk::timespec),
            crtime.map(fuse2dk::timespec),
        ) {
            Ok(stat) => reply.attr(&TTL, &dk2fuse::file_attr(stat)),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn readlink(&mut self, req: &Request, ino: u64, reply: ReplyData) {
        ino![ino];
        debug_params!(self.log; readlink; req, ino);
        let res = self
            .dk
            .getattr(ino)
            .map(|stat| stat.size)
            .and_then(|size| self.dk.open(ino, Flags::READ_ONLY).map(|fh| (fh, size)))
            .and_then(|(fh, size)| self.dk.read(fh, 0, size));
        match res {
            Ok(v) => reply.data(&v[..]),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
        match self
            .dk
            .mknod(req.uid(), req.gid(), parent, name, fuse2dk::file_mode(mode))
        {
            Ok(stat) => reply.entry(&TTL, &dk2fuse::file_attr(stat), req.unique()),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn mkdir(&mut self, req: &Request, parent: u64, name: &OsStr, mode: u32, reply: ReplyEntry) {
        ino![parent];
        debug_params!(self.log; mkdir; req, parent, name, mode);
        match self
            .dk
            .mkdir(parent, req.uid(), req.gid(), name, fuse2dk::file_mode(mode))
        {
            Ok(stat) => reply.entry(&TTL, &dk2fuse::file_attr(stat), req.unique()),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn unlink(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];
        debug_params!(self.log; unlink; req, parent, name);
        match self.dk.unlink(parent, name) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn rmdir(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![parent];
        debug_params!(self.log; rmdir; req, parent, name);
        match self.dk.rmdir(parent, name) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
        match self.dk.symlink(req.uid(), req.gid(), parent, name, link) {
            Ok(stat) => reply.entry(&TTL, &dk2fuse::file_attr(stat), req.unique()),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
        match self.dk.rename(parent, name, newparent, newname) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn open(&mut self, req: &Request, ino: u64, flags: u32, reply: ReplyOpen) {
        ino![ino];
        debug_params!(self.log; open; req, ino, flags);
        match self.dk.open(ino, fuse2dk::flags(flags)) {
            Ok(fh) => {
                reply.opened(req.unique(), dk2fuse::flags(fh.flags));
                self.file_fh.insert(req.unique(), fh);
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn read(&mut self, req: &Request, ino: u64, fh: u64, offset: i64, size: u32, reply: ReplyData) {
        ino![ino];
        debug_params!(self.log; read; req, ino, fh, offset, size);
        let fh = match self.file_fh.get(&fh) {
            Some(fh) => fh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.dk.read(fh.clone(), offset as u64, size as u64) {
            Ok(v) => reply.data(&v[..]),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
        let fh = match self.file_fh.get(&fh) {
            Some(fh) => fh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.dk.write(fh.clone(), offset as u64, data) {
            Ok(size) => reply.written(size as u32),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn flush(&mut self, req: &Request, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; flush; req, ino, fh, lock_owner);
        let fh = match self.file_fh.get(&fh) {
            Some(fh) => fh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.dk.flush(fh.clone()) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
        if let Some(_) = self.file_fh.remove(&fh) {
            match self.dk.apply_releases() {
                Ok(_) => reply.ok(),
                Err(e) => {
                    error!(self.log, "{}", e);
                    reply.error(dk2fuse::errno(&e));
                }
            }
        }
    }

    fn fsync(&mut self, req: &Request, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; fsync; req, ino, fh, datasync);
        let fh = match self.file_fh.get(&fh) {
            Some(fh) => fh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.dk.fsync(fh.clone(), datasync) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
                reply.error(dk2fuse::errno(&e));
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
                    reply.error(dk2fuse::errno(&e));
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
                    reply.error(dk2fuse::errno(&e));
                }
            }
        }
    }

    fn fsyncdir(&mut self, req: &Request, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; fsyncdir; req, ino, fh, datasync);
        let dh = match self.dir_fh.get(&fh) {
            Some(dh) => dh,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        match self.dk.fsyncdir(dh.clone(), datasync) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
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
                reply.error(dk2fuse::errno(&e));
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
        match self.dk.setxattr(ino, name, value) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn getxattr(&mut self, req: &Request, ino: u64, name: &OsStr, size: u32, reply: ReplyXattr) {
        ino![ino];
        debug_params!(self.log; getxattr; req, ino, name, size);
        match self.dk.getxattr(ino, name) {
            Ok(Some(v)) => {
                if size == 0 {
                    reply.size(v.len() as u32);
                } else {
                    if size > v.len() as u32 {
                        reply.error(ERANGE);
                    } else {
                        reply.data(&v);
                    }
                }
            }
            Ok(None) => reply.error(ENOATTR),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn listxattr(&mut self, req: &Request, ino: u64, size: u32, reply: ReplyXattr) {
        ino![ino];
        debug_params!(self.log; listxattr; req, ino, size);
        match self.dk.listxattr(ino) {
            Ok(v) => {
                let mut b = Vec::new();
                for name in &v {
                    b.extend_from_slice(name.as_bytes());
                    b.push(0);
                }
                if size == 0 {
                    reply.size(b.len() as u32);
                } else {
                    if size > b.len() as u32 {
                        reply.error(ERANGE);
                    } else {
                        reply.data(&b);
                    }
                }
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn removexattr(&mut self, req: &Request, ino: u64, name: &OsStr, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; removexattr; req, ino, name);
        match self.dk.removexattr(ino, name) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(dk2fuse::errno(&e));
            }
        }
    }

    fn access(&mut self, req: &Request, ino: u64, mask: u32, reply: ReplyEmpty) {
        ino![ino];
        debug_params!(self.log; access; req, ino, mask);
        // This function should not be called with `default_permissions` mount option.
        reply.error(ENOSYS);
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
        // Returns ENOSYS to let fuse use mknod and open instead.
        reply.error(ENOSYS);
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
        reply.error(ENOSYS);
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
        reply.error(ENOSYS);
    }

    fn bmap(&mut self, req: &Request, ino: u64, blocksize: u32, idx: u64, reply: ReplyBmap) {
        ino![ino];
        debug_params!(self.log; bmap; req, ino, blocksize, idx);
        reply.error(ENOSYS);
    }
}

mod dk2fuse;
mod fuse2dk;
