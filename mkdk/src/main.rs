#[macro_use]
extern crate clap;
extern crate dkfs;

use dkfs::*;

fn main() -> DkResult<()> {
    use clap::*;

    let bpi = format!("{}", DEFAULT_BYTES_PER_INODE);
    let matches = App::new("mkdk")
        .version("0.1.1")
        .author("Yilin Chen <sticnarf@gmail.com>")
        .about("Make a donkey file system")
        .arg(
            Arg::with_name("device")
                .help("Path to the device to be used")
                .required(true),
        ).arg(
            Arg::with_name("bytes-per-inode")
                .help("Specify the bytes/inode ratio")
                .short("i")
                .takes_value(true)
                .default_value(&bpi),
        ).get_matches();

    let dev_path = matches.value_of("device").unwrap();
    let bytes_per_inode =
        value_t!(matches.value_of("bytes-per-inode"), u64).unwrap_or_else(|e| e.exit());

    let opt = FormatOptions::default().bytes_per_inode(bytes_per_inode);
    let _ = format(dev(dev_path)?, opt)?;
    Ok(())
}
