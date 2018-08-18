# donkey-fs

[![Build Status](https://travis-ci.org/sticnarf/donkey-fs.svg?branch=master)](https://travis-ci.org/sticnarf/donkey-fs)

Donkey is a simple and naive file system for purposes of learning.
**Performance or reliability is never taken into consideration.**

This project contains:

* `dkfs`: Library that supports `mkdk` and `mtdk`.
* `mkdk`: Binary to make a donkey file system.
* `mtdk`: Binary to mount a donkey file ystem.

## Build

Rust 1.28 or above is required in order to build this project.

`pkg-config` and `libfuse-dev` are needed to build `mtdk`.

This project should compile on Linux, macOS and FreeBSD,
but only **Linux** is assured on which `mtdk` performs properly.
See [#4](https://github.com/sticnarf/donkey-fs/issues/4).

Prebuilt binaries are available in the [releases section](https://github.com/sticnarf/donkey-fs/releases).

## Usage

Run `mkdk --help` or `mtdk --help` for usage.

Fuse 2.x library should be installed if you use `mtdk`.

## Limitations

The max file size is about 256 TB. There is no practical limit on the file system size.