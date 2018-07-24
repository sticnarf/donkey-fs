#[macro_use]
extern crate clap;
#[macro_use]
extern crate slog;
extern crate dkfs;
extern crate slog_term;

use dkfs::*;
use slog::{Drain, Logger};

fn main() {
    use clap::*;

    let matches = App::new("mkdonkey")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Make a donkey filesystem")
        .arg(
            Arg::with_name("device")
                .help("Path to the device to be used")
                .required(true),
        )
        .arg(
            Arg::with_name("bytes-per-inode")
                .help("Specify the bytes/inode ratio")
                .short("i")
                .takes_value(true)
                .default_value(DEFAULT_BYTES_PER_INODE_STR),
        )
        .get_matches();

    let log = logger();
    let dev_path = matches.value_of("device").unwrap();
    let bytes_per_inode =
        value_t!(matches.value_of("bytes-per-inode"), u64).unwrap_or_else(|e| e.exit());

    let opt = FormatOptions::new().bytes_per_inode(bytes_per_inode);
    let donkey = Donkey::create(dev_path).and_then(|dk| dk.format(&opt, Some(log.clone())));

    if let Err(e) = donkey {
        error!(log, "{}", e);
    } else {
        info!(log, "Done.");
    }
}

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}
