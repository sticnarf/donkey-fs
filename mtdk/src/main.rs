extern crate clap;
#[macro_use]
extern crate slog;
extern crate dkfs;
extern crate fuse;
extern crate slog_term;

use dkfs::*;
use fuse::Filesystem;
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

    let res = Donkey::new(dev_path)
        .and_then(|dk| dk.open())
        .and_then(|dk| {
            let fuse = DonkeyFuse { dk };
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
}

impl Filesystem for DonkeyFuse {}
