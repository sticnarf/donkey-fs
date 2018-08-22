use dkfs::*;
use libc::*;
use time::Timespec;

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
        }
        if (mode & S_ISGID) != 0 {
            res |= FileMode::SET_GROUP_ID;
        }
        if (mode & S_ISVTX) != 0 {
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

pub fn flags(flags: u32) -> Flags {
    let mut res = Flags::empty();

    let access_flags = (flags as c_int) & O_ACCMODE;
    match access_flags {
        O_RDONLY => res |= Flags::READ_ONLY,
        O_WRONLY => res |= Flags::WRITE_ONLY,
        O_RDWR => res |= Flags::READ_WRITE,
        _ => return Flags::INVALID,
    }

    let flags = flags as c_int;
    if (flags & O_APPEND) != 0 {
        res |= Flags::APPEND;
    }

    res
}

pub fn timespec(t: Timespec) -> DkTimespec {
    DkTimespec {
        sec: t.sec,
        nsec: t.nsec as u32,
    }
}
