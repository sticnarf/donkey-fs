use dkfs::*;
use libc::*;

type DkTimespec = ::dkfs::Timespec;
type TmTimespec = ::time::Timespec;

pub fn file_mode(mode: u32) -> FileMode {
    let mut res = FileMode::empty();

    let type_bits = (mode as mode_t) & S_IFMT;
    match type_bits {
        S_IFSOCK => res |= FileMode::SOCKET,
        S_IFLNK => res |= FileMode::SYMBOLIC_LINK,
        S_IFREG => res |= FileMode::REGULAR_FILE,
        S_IFBLK => res |= FileMode::BLOCK_DEVICE,
        S_IFDIR => res |= FileMode::DIRECTORY,
        S_IFCHR => res |= FileMode::CHARACTER_DEVICE,
        S_IFIFO => res |= FileMode::FIFO,
        _ => {}
    }

    {
        let mode = mode as c_int;
        if (mode & S_ISUID) != 0 {
            res |= FileMode::SET_USER_ID;
        } else if (mode & S_ISGID) != 0 {
            res |= FileMode::SET_GROUP_ID;
        } else if (mode & S_ISVTX) != 0 {
            res |= FileMode::STICKY;
        }
    }

    let mode = mode as mode_t;
    if (mode & S_IRUSR) != 0 {
        res |= FileMode::USER_READ;
    }
    if (mode & S_IWUSR) != 0 {
        res |= FileMode::USER_WRITE;
    }
    if (mode & S_IXUSR) != 0 {
        res |= FileMode::USER_EXECUTE;
    }
    if (mode & S_IRGRP) != 0 {
        res |= FileMode::GROUP_READ;
    }
    if (mode & S_IWGRP) != 0 {
        res |= FileMode::GROUP_WRITE;
    }
    if (mode & S_IXGRP) != 0 {
        res |= FileMode::GROUP_EXECUTE;
    }
    if (mode & S_IROTH) != 0 {
        res |= FileMode::OTHERS_READ;
    }
    if (mode & S_IWOTH) != 0 {
        res |= FileMode::OTHERS_WRITE;
    }
    if (mode & S_IXOTH) != 0 {
        res |= FileMode::OTHERS_EXECUTE;
    }

    res
}

pub fn open_flags(flags: u32) -> OpenFlags {
    let mut res = OpenFlags::empty();

    let access_flags = (flags as c_int) & O_ACCMODE;
    match access_flags {
        O_RDONLY => res |= OpenFlags::READ_ONLY,
        O_WRONLY => res |= OpenFlags::WRITE_ONLY,
        O_RDWR => res |= OpenFlags::READ_WRITE,
        _ => unreachable!(),
    }

    let flags = flags as c_int;
    if (flags & O_APPEND) != 0 {
        res |= OpenFlags::APPEND;
    }

    res
}

pub fn timespec(t: TmTimespec) -> DkTimespec {
    DkTimespec {
        sec: t.sec,
        nsec: t.nsec as i64,
    }
}
