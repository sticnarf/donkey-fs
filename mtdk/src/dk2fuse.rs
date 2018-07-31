use dkfs::*;
use fuse::*;

type DkTimespec = ::dkfs::Timespec;
type TmTimespec = ::time::Timespec;

type DkFileAttr = ::dkfs::FileAttr;
type FuseFileAttr = ::fuse::FileAttr;

pub fn file_type(mode: FileMode) -> FileType {
    if is_directory(mode) {
        FileType::Directory
    } else if is_regular_file(mode) {
        FileType::RegularFile
    } else {
        unimplemented!()
    }
}

pub fn permission(mode: FileMode) -> u16 {
    0o777 & mode.bits() as u16
}

pub fn timespec(t: DkTimespec) -> TmTimespec {
    TmTimespec {
        sec: t.sec,
        nsec: t.nsec as i32,
    }
}

pub fn attr(attr: DkFileAttr, ino: u64) -> FuseFileAttr {
    FuseFileAttr {
        ino,
        size: attr.size,
        blocks: (attr.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
        atime: timespec(attr.atime),
        mtime: timespec(attr.mtime),
        ctime: timespec(attr.ctime),
        crtime: timespec(attr.crtime),
        kind: file_type(attr.mode),
        perm: permission(attr.mode),
        nlink: attr.nlink as u32,
        uid: attr.uid,
        gid: attr.gid,
        rdev: attr.rdev as u32,
        flags: 0,
    }
}
