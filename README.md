# donkey-fs

Donkey is a simple and naive file system for purposes of learning.
**Performance or reliability is never taken into consideration.**

This project contains:

* `dkfs`: Library that supports `mkdk` and `mtdk`.
* `mkdk`: Binary to make a donkey file system.
* `mtdk`: Binary to mount a donkey file ystem.

## Build

Rust 1.28 or above is required in order to build this project.

`libfuse` headers and library are needed in order to build and run `mtdk`.

This project should compile on Linux, macOS and FreeBSD,
but only **Linux** is assured on which `mtdk` performs properly.
See [#4](https://github.com/sticnarf/donkey-fs/issues/4).

## Usage

Run `mkdk --help` or `mtdk --help` for usage.

Root permission is usually required to format a block device and mount a file system.