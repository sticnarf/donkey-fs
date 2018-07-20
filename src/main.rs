extern crate fuse;

use fuse::*;

struct DonkeyFS;

impl Filesystem for DonkeyFS {}

fn main() {
    let mountpoint = "";
    fuse::mount(DonkeyFS, &mountpoint, &[]).unwrap();
}
