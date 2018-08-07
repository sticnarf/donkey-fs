#[macro_use]
extern crate clap;
extern crate dkfs;

use dkfs::*;

fn main() -> DkResult<()> {
    use clap::*;

    let matches = App::new("mkdk")
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

    let dev_path = matches.value_of("device").unwrap();
    let bytes_per_inode =
        value_t!(matches.value_of("bytes-per-inode"), u64).unwrap_or_else(|e| e.exit());

    let opt = FormatOptions::default().bytes_per_inode(bytes_per_inode);
    let _ = Donkey::format(dev_path, opt)?;
    Ok(())
}