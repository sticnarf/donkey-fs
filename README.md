# donkey-fs

Donkey is a simple and naive file system for purposes of learning.

This project contains:

* `dkfs`: Library that supports `mkdk` and `mtdk`.
* `mkdk`: Binary to make a donkey file system.
* `mtdk`: Binary to mount a donkey files ystem.

## How to use

Run `cargo build` to build the project.

NLL needs to be enabled. Therefore, a nightly Rust compiler is required.

The library `dkfs` and the binary `mkdk` do not have any other dependencies.
However, `libfuse` headers and library are needed in order to build and run `mtdk`.
