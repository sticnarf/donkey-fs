extern crate clap;
#[macro_use]
extern crate slog;
extern crate dkfs;
extern crate fuse;
extern crate libc;
extern crate slog_term;
extern crate time;

use dkfs::*;
use fuse::*;
use slog::{Drain, Logger};
use std::ffi::OsStr;

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
    let options = ["-o", "fsname=donkey"]
        .iter()
        .map(|o| OsStr::new(o))
        .collect::<Vec<&OsStr>>();

    let res = DonkeyBuilder::new(dev_path)
        .and_then(|dk| dk.open())
        .and_then(|dk| {
            let fuse = DonkeyFuse {
                dk,
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

struct DonkeyFuse {
    dk: Donkey,
    log: Logger,
}

const TTL: time::Timespec = time::Timespec { sec: 1, nsec: 0 };

impl Filesystem for DonkeyFuse {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        info!(
            self.log,
            "lookup, parent: {}, name: {}",
            parent,
            name.to_str().unwrap_or("not valid string")
        );
    }

    fn getattr(&mut self, _req: &Request, mut ino: u64, reply: ReplyAttr) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        info!(self.log, "getattr, inode: {}", ino);
        match self.dk.get_attr(ino) {
            Ok(attr) => {
                let fuse_attr = fuse::FileAttr {
                    ino,
                    size: attr.size,
                    blocks: (attr.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
                    atime: convert_timespec(attr.atime),
                    mtime: convert_timespec(attr.mtime),
                    ctime: convert_timespec(attr.ctime),
                    crtime: convert_timespec(attr.crtime),
                    kind: mode_to_filetype(attr.mode),
                    perm: mode_to_permission(attr.mode),
                    nlink: attr.nlink as u32,
                    uid: attr.uid,
                    gid: attr.gid,
                    rdev: attr.rdev as u32,
                    flags: 0,
                };
                info!(self.log, "file attr: {:?}", fuse_attr);
                reply.attr(&TTL, &fuse_attr)
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(libc::ENOENT)
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        mut ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        reply: ReplyData,
    ) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        info!(self.log, "read, ino: {}, fh: {}, size: {}", ino, _fh, _size);
    }

    fn opendir(&mut self, _req: &Request, mut ino: u64, _flags: u32, reply: ReplyOpen) {
        info!(self.log, "opendir, ino: {}", ino);

        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        match self.dk.open(ino) {
            Ok(fh) => {
                info!(self.log, "opened, fh: {}", fh);
                reply.opened(fh, 0)
            }
            Err(_) => {
                error!(self.log, "cannot open inode {}", ino);
                reply.error(libc::ENOENT)
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        info!(
            self.log,
            "readdir, ino: {}, fh: {}, offset: {}", _ino, fh, offset
        );
        let mut offset = offset as u64;
        while let Ok(Some((entry, new_offset))) =
            self.dk.read_dir(fh, offset, Some(self.log.clone()))
        {
            offset = new_offset;
            match self.dk.get_attr(entry.inode) {
                Ok(attr) => {
                    let full = reply.add(
                        entry.inode,
                        new_offset as i64,
                        mode_to_filetype(attr.mode),
                        entry.filename,
                    );
                    if full {
                        return;
                    }
                }
                Err(_) => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        }
        reply.ok()
    }
}

fn mode_to_filetype(mode: FileMode) -> FileType {
    if is_directory(mode) {
        FileType::Directory
    } else if is_regular_file(mode) {
        FileType::RegularFile
    } else {
        unimplemented!()
    }
}

fn mode_to_permission(mode: FileMode) -> u16 {
    0o777 & (mode.bits() >> 2) as u16
}

fn convert_timespec(t: dkfs::Timespec) -> time::Timespec {
    time::Timespec {
        sec: t.sec,
        nsec: t.nsec as i32,
    }
}
