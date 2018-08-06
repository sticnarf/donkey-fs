use super::Block;
use super::DkResult;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::io::{self, Seek, SeekFrom};

pub trait Device: Read + Write + Seek {
    fn block_count(&self) -> u64;

    fn block_size(&self) -> u64;

    fn read_block<B: Block>(&mut self, bid: u64) -> DkResult<B> {
        let (bc, bs) = (self.block_count(), self.block_size());
        if bid >= bc {
            Err(format_err!("Read block {} of {}", bid, bc))
        } else {
            self.seek(SeekFrom::Start(bid * bs))?;
            let bytes = self.bytes().take(bs as usize);
            B::from_bytes(bytes)
        }
    }

    fn write_block<B: Block>(&mut self, bid: u64, block: &B) -> DkResult<()> {
        let (bc, bs) = (self.block_count(), self.block_size());
        if bid >= bc {
            Err(format_err!("Write block {} of {}", bid, bc))
        } else {
            self.seek(SeekFrom::Start(bid * bs))?;
            let bytes = block.as_bytes();
            Ok(self.write_all(bytes)?)
        }
    }
}

pub struct ImageFile {
    file: File,
    block_count: u64,
}

impl Device for ImageFile {
    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn block_size(&self) -> u64 {
        // The default block size for an image file is 4KiB
        4096
    }
}

impl Read for ImageFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for ImageFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for ImageFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}
