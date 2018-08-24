# donkey-fs

[![Build Status](https://travis-ci.org/sticnarf/donkey-fs.svg?branch=master)](https://travis-ci.org/sticnarf/donkey-fs)

Donkey is a simple and naive file system for purposes of learning.
**Performance or reliability is never taken into consideration.**

It has passed [pjdfstest](https://github.com/pjd/pjdfstest/) for correctness.

## Build

Rust 1.28 or above is required in order to build this project.

`pkg-config` and libfuse 2.x headers are needed to build `mtdk`.

Prebuilt binaries are available in the [releases section](https://github.com/sticnarf/donkey-fs/releases).

## Format

`mkdk` is the format tool. 

The `device` argument accepts regular files or block special files.

The size of a block device is automatically detected.

You can specify your own bytes/inode ratio for the file system.
Pay attention that this ratio cannot be modified after formatting.

```
USAGE:
    mkdk [OPTIONS] <device>

OPTIONS:
    -i <bytes-per-inode>        Specify the bytes/inode ratio [default: 16384]

ARGS:
    <device>    Path to the device to be used
```

## Mount

Although this file system is not designed to depend on Linux FUSE, 
the only way to mount a donkey file system now is to use `mtdk` with libfuse 2.x.

So you must install libfuse 2.x (`libfuse2` on Debian/Ubuntu) before running `mtdk`.

Note that `allow_other` option is enabled, so non-root users cannot mount using `mtdk` 
unless you uncomment the `user_allow_other` line in `/etc/fuse.conf`. 

```
USAGE:
    mtdk [FLAGS] <device> <dir>

FLAGS:
    -d               Run as a daemon

ARGS:
    <device>    Path to the device to be used
    <dir>       Path of the mount point
```

## Limitations

The max file size is about 256 TB. There is no practical limit on the file system size.

Linux is the only supported platform.