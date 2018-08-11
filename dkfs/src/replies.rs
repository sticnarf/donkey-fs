use *;

#[derive(Debug)]
pub struct Statvfs {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u64,
    pub namelen: u32,
}

#[derive(Debug)]
pub struct Stat {
    pub ino: u64,
    pub mode: FileMode,
    pub size: u64,
    pub blksize: u32,
    pub blocks: u64,
    pub atime: DkTimespec,
    pub mtime: DkTimespec,
    pub ctime: DkTimespec,
    pub crtime: DkTimespec,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
}
