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
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{Seek, SeekFrom};

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

    fn dk_open(&self, inode: u64, flags: OpenFlags) -> Result<DonkeyFile> {
        self.dk.open(inode, flags, Some(self.log.clone()))
    }

    // Remember to close fh after calling this method!!!!
    fn dk_open_fh(&mut self, inode: u64, flags: OpenFlags) -> Result<u64> {
        let dkfile = self.dk_open(inode, flags)?;
        let new_fh = self.new_fh();
        self.opened_files.insert(new_fh, dkfile);
        debug!(self.log, "open inode {}, fh: {}", inode, new_fh);
        Ok(new_fh)
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
        Ok(dk2fuse::attr(dkfile.get_attr()?, ino))
    }

    fn dk_lookup(&self, _req: &Request, parent: u64, name: &OsStr) -> Result<fuse::FileAttr> {
        let mut dkfile = self.dk_open(parent, OpenFlags::READ_ONLY)?;

        loop {
            let result = dkfile.read_dir()?;
            if let Some(entry) = result {
                if entry.filename == name {
                    let attr = dkfile.get_attr()?;
                    return Ok(dk2fuse::attr(attr, entry.inode));
                }
            } else {
                return Err(format_err!("Cannot find file"));
            }
        }
    }

    // returns (fh, flags)
    fn dk_opendir(&mut self, _req: &Request, ino: u64, flags: u32) -> Result<(u64, u32)> {
        let fh = self.dk_open_fh(ino, fuse2dk::open_flags(flags))?;
        Ok((fh, flags))
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
            (dkfile.read_dir()?, dkfile.offset as i64)
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
            1,
            Some(rdev as u64),
            Some(self.log.clone()),
        )?;
        debug!(self.log, "Inode {} is created", inode);
        self.dk.link(inode, parent, name, Some(self.log.clone()))?;
        debug!(self.log, "Inode {} is linked to parent {}", inode, parent);
        self.dk_getattr(req, inode)
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
            Ok(attr) => reply.entry(&TTL, &attr, get_new_generation()),
            Err(e) => {
                error!(self.log, "{}", e);
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
                error!(self.log, "{}", e);
                reply.error(libc::ENOENT);
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

        match self.dk_opendir(_req, ino, _flags) {
            Ok((fh, flags)) => {
                debug!(self.log, "opened {}, fh: {}, flags: {}", ino, fh, flags);
                reply.opened(fh, flags)
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
                    let full = reply.add(entry.ino, new_offset, entry.kind, filename);
                    if full {
                        return;
                    }
                    offset = new_offset;
                }
                Ok(None) => {
                    reply.ok();
                    return;
                }
                Err(_) => {
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
                error!(self.log, "{}", e);
                reply.error(libc::EIO);
            }
        }
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
