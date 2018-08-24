use dkfs::replies::*;
use dkfs::*;
use fuse::*;
use libc::*;
use time::Timespec;

pub fn file_type(mode: FileMode) -> FileType {
    match mode & FileMode::FILE_TYPE_MASK {
        FileMode::REGULAR_FILE => FileType::RegularFile,
        FileMode::SOCKET => FileType::Socket,
        FileMode::DIRECTORY => FileType::Directory,
        FileMode::SYMBOLIC_LINK => FileType::Symlink,
        FileMode::CHARACTER_DEVICE => FileType::CharDevice,
        FileMode::BLOCK_DEVICE => FileType::BlockDevice,
        FileMode::FIFO => FileType::NamedPipe,
        _ => unreachable!(),
    }
}

pub fn permission(mode: FileMode) -> u16 {
    0o7777 & mode.bits() as u16
}

pub fn timespec(t: DkTimespec) -> Timespec {
    Timespec {
        sec: t.sec,
        nsec: t.nsec as i32,
    }
}

pub fn flags(flags: Flags) -> u32 {
    let access_flags = flags & Flags::ACCESS_MODE_MASK;
    let res = match access_flags {
        Flags::READ_ONLY => O_RDONLY,
        Flags::WRITE_ONLY => O_WRONLY,
        Flags::READ_WRITE => O_RDWR,
        _ => unreachable!(),
    };
    res as u32
}

pub fn file_attr(stat: &Stat) -> FileAttr {
    FileAttr {
        ino: stat.ino,
        size: stat.size,
        blocks: stat.blocks,
        atime: timespec(stat.atime),
        mtime: timespec(stat.mtime),
        ctime: timespec(stat.ctime),
        crtime: timespec(stat.crtime),
        kind: file_type(stat.mode),
        perm: permission(stat.mode),
        nlink: stat.nlink as u32,
        uid: stat.uid,
        gid: stat.gid,
        rdev: stat.rdev as u32,
        flags: 0,
    }
}

pub fn errno(error: &DkError) -> c_int {
    use DkError::*;
    match error {
        IoError(_) | Corrupted(_) | Other(_) => EIO,
        Exhausted => EDQUOT,
        NotSupported => ENOSYS,
        NotFound => ENOENT,
        NotEmpty => ENOTEMPTY,
        NotDirectory => ENOTDIR,
        AlreadyExists => EEXIST,
        Invalid(_) => EINVAL,
        NameTooLong => ENAMETOOLONG,
    }
}
