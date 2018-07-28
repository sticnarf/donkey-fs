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
use slog::{Drain, Logger};
use std::cell::Cell;
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
    let options = ["-o", "fsname=donkey", "-o", "allow_other"]
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

        fn real_lookup(
            dk: &mut Donkey,
            parent: u64,
            name: &OsStr,
            log: Logger,
        ) -> Result<fuse::FileAttr, Error> {
            let fh = dk.open(parent)?;
            debug!(log, "opened {}, fh: {}", parent, fh);

            let mut offset = 0;
            loop {
                let result = dk.read_dir(fh, offset, Some(log.clone()))?;
                if let Some((entry, new_offset)) = result {
                    if entry.filename == name {
                        let attr = dk.get_attr(entry.inode)?;
                        return Ok(dk2fuse::attr(attr, entry.inode));
                    }
                    offset = new_offset;
                } else {
                    return Err(format_err!("Cannot find file"));
                }
            }
        }

        if let Ok(attr) = real_lookup(&mut self.dk, parent, name, self.log.clone()) {
            reply.entry(&TTL, &attr, get_generation());
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, mut ino: u64, reply: ReplyAttr) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "getattr, inode: {}", ino);
        match self.dk.get_attr(ino) {
            Ok(attr) => {
                let fuse_attr = dk2fuse::attr(attr, ino);
                reply.attr(&TTL, &fuse_attr)
            }
            Err(e) => {
                error!(self.log, "{}", e);
                reply.error(libc::ENOENT)
            }
        }
    }

    fn open(&mut self, _req: &Request, mut ino: u64, _flags: u32, reply: ReplyOpen) {
        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        debug!(self.log, "open, inode: {}", ino);
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
        debug!(self.log, "opendir, ino: {}", ino);

        if ino == FUSE_ROOT_ID {
            ino = self.dk.root_inode();
        }

        match self.dk.open(ino) {
            Ok(fh) => {
                debug!(self.log, "opened {}, fh: {}", ino, fh);
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
        debug!(
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
                        dk2fuse::filetype(attr.mode),
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

    fn releasedir(&mut self, _req: &Request, _ino: u64, fh: u64, _flags: u32, reply: ReplyEmpty) {
        debug!(self.log, "releasedir, fh: {}", fh);

        self.dk.close(fh);
        reply.ok();
    }

    fn mknod(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        unimplemented!()
    }
}

thread_local!(static GENERATION: Cell<(i64, u64)> = Cell::new((0,0)));

// Generate a unique value for NFS generation
// https://libfuse.github.io/doxygen/structfuse__entry__param.html
fn get_generation() -> u64 {
    GENERATION.with(|cell| {
        let (t, g) = cell.get();
        let new_t = time::now().to_timespec().sec;
        let new_g = if t == new_t { g + 1 } else { 1 };
        cell.set((new_t, new_g));
        (new_t as u64) << 26 + new_g
    })
}
