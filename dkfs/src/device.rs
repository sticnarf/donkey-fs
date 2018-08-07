use super::*;
use block::Block;
use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

pub trait Device: Read + Write + Seek + Debug {
    fn block_count(&self) -> u64;

    fn block_size(&self) -> u64;

    fn size(&self) -> u64 {
        self.block_size() * self.block_count()
    }

    fn read_at<'a>(&'a mut self, ptr: u64) -> DkResult<Box<dyn Read + 'a>> {
        let size = self.size();
        if ptr >= size {
            Err(format_err!("Read at {}, but device size is {}", ptr, size))
        } else {
            self.seek(SeekFrom::Start(ptr))?;
            Ok(Box::new(self))
        }
    }

    fn write_block_at(&mut self, block: &Block, ptr: u64) -> DkResult<()> {
        let size = self.size();
        let bytes = block.as_bytes()?;
        let len = bytes.len() as u64;
        if ptr + len >= size {
            Err(format_err!(
                "Write {} bytes at {}, but device size is {}",
                len,
                ptr,
                size
            ))
        } else {
            self.seek(SeekFrom::Start(ptr))?;
            Ok(self.write_all(&bytes)?)
        }
    }
}

pub fn open<P: AsRef<Path>>(dev_path: P) -> DkResult<Box<dyn Device>> {
    let file = OpenOptions::new().read(true).write(true).open(dev_path)?;
    let file_type = file.metadata()?.file_type();
    if file_type.is_file() {
        Ok(Box::new(ImageFile::new(file)?))
    } else if file_type.is_block_device() {
        Ok(Box::new(BlockDevice::new(file)?))
    } else {
        Err(format_err!("This device is not supported."))
    }
}

// The default block size is 4 KiB
const DEFAULT_BLOCK_SIZE: u64 = 4096;

#[derive(Debug)]
struct ImageFile {
    file: File,
    block_count: u64,
}

impl ImageFile {
    /// Creates an `ImageFile`.
    /// The block size for an image file is 4 KiB.
    fn new(file: File) -> DkResult<Self> {
        let metadata = file.metadata()?;
        // `file` must be a regular file
        assert!(metadata.is_file());
        let size = metadata.len();

        let dev = ImageFile {
            file,
            block_count: size / DEFAULT_BLOCK_SIZE,
        };
        Ok(dev)
    }
}

impl Device for ImageFile {
    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn block_size(&self) -> u64 {
        DEFAULT_BLOCK_SIZE
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

#[derive(Debug)]
struct BlockDevice {
    file: File,
    block_count: u64,
    block_size: u64,
}

impl BlockDevice {
    /// Creates a `BlockDevice`.
    /// We just use 4 KiB as the block size for a block device.
    /// We do not detect the raw block size of the device
    /// at this time.
    fn new(file: File) -> DkResult<Self> {
        let file_type = file.metadata()?.file_type();
        // `file` must be a block device
        assert!(file_type.is_block_device());
        let size = Self::block_dev_size(&file)?;

        let dev = BlockDevice {
            file,
            block_count: size / DEFAULT_BLOCK_SIZE,
            block_size: DEFAULT_BLOCK_SIZE,
        };
        Ok(dev)
    }

    fn block_dev_size(dev: &File) -> DkResult<u64> {
        let fd = dev.as_raw_fd();
        #[cfg(target_os = "linux")]
        fn getsize(fd: RawFd) -> DkResult<u64> {
            // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
            ioctl_read!(getsize64, 0x12, 114, u64);
            let mut size: u64 = 0;
            unsafe {
                getsize64(fd, &mut size)?;
            }
            Ok(size)
        }

        #[cfg(target_os = "macos")]
        fn getsize(fd: RawFd) -> DkResult<u64> {
            // https://github.com/apple/darwin-xnu/blob/xnu-4570.1.46/bsd/sys/disk.h#L203
            ioctl_read!(getblksize, b'd', 24, u32);
            ioctl_read!(getblkcount, b'd', 25, u64);
            let mut blksize: u32 = 0;
            let mut blkcount: u64 = 0;
            unsafe {
                getblksize(fd, &mut blksize)?;
                getblkcount(fd, &mut blkcount)?;
            }
            Ok(blksize as u64 * blkcount)
        }

        #[cfg(target_os = "freebsd")]
        fn getsize(fd: RawFd) -> DkResult<u64> {
            // https://github.com/freebsd/freebsd/blob/stable/11/sys/sys/disk.h#L37
            ioctl_read!(getmediasize, b'd', 129, u64);
            let mut size: u64 = 0;
            unsafe {
                getmediasize(fd, &mut size)?;
            }
            Ok(size)
        }

        getsize(fd)
    }
}

impl Device for BlockDevice {
    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn block_size(&self) -> u64 {
        self.block_size
    }
}

impl Read for BlockDevice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for BlockDevice {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for BlockDevice {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}
