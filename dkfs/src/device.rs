use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use *;

pub trait Device: Read + Write + Seek + Debug {
    fn block_count(&self) -> u64;

    fn block_size(&self) -> u64;

    fn size(&self) -> u64 {
        self.block_size() * self.block_count()
    }

    /// No length limit
    fn read_at<'a>(&'a mut self, ptr: u64) -> DkResult<Box<dyn Read + 'a>> {
        let size = self.size();
        if ptr > size {
            Err(Corrupted(format!(
                "Read at {}, but device size is {}",
                ptr, size
            )))
        } else {
            self.seek(SeekFrom::Start(ptr))?;
            Ok(Box::new(self))
        }
    }

    /// Limit length to `len`
    fn read_len_at<'a>(&'a mut self, ptr: u64, len: u64) -> DkResult<Box<dyn Read + 'a>> {
        let size = self.size();
        if ptr + len > size {
            Err(Corrupted(format!(
                "Read {} bytes at {}, but device size is {}",
                len, ptr, size
            )))
        } else {
            self.seek(SeekFrom::Start(ptr))?;
            Ok(Box::new(self.take(len)))
        }
    }

    /// Limit length to one block size
    fn read_block_at<'a>(&'a mut self, ptr: u64) -> DkResult<Box<dyn Read + 'a>> {
        let bs = self.block_size();
        self.read_len_at(ptr, bs)
    }

    fn write_at(&mut self, writable: &Writable, ptr: u64) -> DkResult<()> {
        let size = self.size();
        let bytes = writable.as_bytes()?;
        let len = bytes.len() as u64;
        if ptr + len > size {
            Err(Corrupted(format!(
                "Write {} bytes at {}, but device size is {}",
                len, ptr, size
            )))
        } else {
            self.seek(SeekFrom::Start(ptr))?;
            Ok(self.write_all(&bytes)?)
        }
    }
}

pub fn dev<P: AsRef<Path>>(dev_path: P) -> DkResult<Box<dyn Device>> {
    let file = OpenOptions::new().read(true).write(true).open(dev_path)?;
    let file_type = file.metadata()?.file_type();
    if file_type.is_file() {
        Ok(Box::new(ImageFile::new(file)?))
    } else if file_type.is_block_device() || file_type.is_char_device() {
        Ok(Box::new(BlockDevice::new(file)?))
    } else {
        Err(NotSupported)
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
        // `file` must be a block device or a character device (on FreeBSD)
        assert!(file_type.is_block_device() || file_type.is_char_device());
        let size = Self::dev_size(&file)?;

        let dev = BlockDevice {
            file,
            block_count: size / DEFAULT_BLOCK_SIZE,
            block_size: DEFAULT_BLOCK_SIZE,
        };
        Ok(dev)
    }

    fn dev_size(dev: &File) -> DkResult<u64> {
        let fd = dev.as_raw_fd();
        #[cfg(target_os = "linux")]
        fn getsize(fd: RawFd) -> DkResult<u64> {
            // https://github.com/torvalds/linux/blob/v4.17/include/uapi/linux/fs.h#L216
            ioctl_read!(getsize64, 0x12, 114, u64);
            let mut size: u64 = 0;
            unsafe {
                getsize64(fd, &mut size).map_err(|e| Other(e.into()))?;
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
                getblksize(fd, &mut blksize).map_err(|e| Other(e.into()))?;
                getblkcount(fd, &mut blkcount).map_err(|e| Other(e.into()))?;
            }
            Ok(blksize as u64 * blkcount)
        }

        #[cfg(target_os = "freebsd")]
        fn getsize(fd: RawFd) -> DkResult<u64> {
            // https://github.com/freebsd/freebsd/blob/stable/11/sys/sys/disk.h#L37
            ioctl_read!(getmediasize, b'd', 129, u64);
            let mut size: u64 = 0;
            unsafe {
                getmediasize(fd, &mut size).map_err(|e| Other(e.into()))?;
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

#[derive(Debug)]
pub struct Memory<'a>(Cursor<&'a mut [u8]>);

impl<'a> Memory<'a> {
    pub fn new(mem: &'a mut [u8]) -> Self {
        Memory(Cursor::new(mem))
    }
}

impl<'a> Device for Memory<'a> {
    fn block_count(&self) -> u64 {
        self.0.get_ref().len() as u64 / DEFAULT_BLOCK_SIZE
    }

    fn block_size(&self) -> u64 {
        DEFAULT_BLOCK_SIZE
    }
}

impl<'a> Read for Memory<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<'a> Write for Memory<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> Seek for Memory<'a> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}
