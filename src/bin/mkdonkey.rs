extern crate clap;
#[macro_use]
extern crate slog;
extern crate failure;
extern crate slog_term;
#[macro_use]
extern crate failure_derive;

use slog::{Drain, Logger};
use std::fs::*;
use std::io;

#[derive(Debug, Fail)]
enum MakefsError {
    #[fail(display = "{}", io_error)]
    OsError {
        #[cause]
        io_error: io::Error,
    },
    #[fail(display = "The device is not supported.")]
    UnsupportedDeviceError,
}

impl From<io::Error> for MakefsError {
    fn from(io_error: io::Error) -> MakefsError {
        MakefsError::OsError { io_error }
    }
}

fn main() {
    use clap::*;

    let matches = App::new("mkdonkey")
        .version("0.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Make a donkey filesystem")
        .arg(
            Arg::with_name("device")
                .help("path to the device to be used")
                .required(true),
        )
        .get_matches();

    let log = logger();
    let path = matches.value_of("device").unwrap();

    if let Err(e) = mkfs(path, log.clone()) {
        error!(log, "{}", e);
    }
}

fn mkfs(path: &str, log: Logger) -> Result<(), MakefsError> {
    let f = File::open(path)?;
    let metadata = f.metadata()?;
    let file_type = metadata.file_type();

    use std::os::unix::fs::FileTypeExt;
    if file_type.is_file() {
        mkfs_file(f, log)
    } else if file_type.is_block_device() {
        mkfs_block_device(f, log)
    } else {
        Err(MakefsError::UnsupportedDeviceError.into())
    }
}

fn mkfs_file(f: File, log: Logger) -> Result<(), MakefsError> {
    unimplemented!()
}

fn mkfs_block_device(f: File, log: Logger) -> Result<(), MakefsError> {
    unimplemented!()
}

fn logger() -> Logger {
    let plain = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(plain).build().fuse();
    let drain = std::sync::Mutex::new(drain).fuse();

    Logger::root(drain, o!())
}
