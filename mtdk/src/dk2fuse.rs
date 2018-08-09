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
    0o777 & mode.bits() as u16
}

pub fn timespec(t: DkTimespec) -> Timespec {
    Timespec {
        sec: t.sec,
        nsec: t.nsec as i32,
    }
}

pub fn flags(flags: Flags) -> u32 {
    let access_flags = flags & Flags::ACCESS_MODE_MASK;
    let mut res = match access_flags {
        Flags::READ_ONLY => O_RDONLY,
        Flags::WRITE_ONLY => O_WRONLY,
        Flags::READ_WRITE => O_RDWR,
        _ => unreachable!(),
    };

    if flags.contains(Flags::APPEND) {
        res |= O_APPEND;
    }

    res as u32
}
