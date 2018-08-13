extern crate dkfs;
extern crate rand;

use dkfs::device::Memory;
use dkfs::replies::*;
use dkfs::*;
use rand::distributions::{Alphanumeric, Standard};
use rand::{thread_rng, Rng};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};

macro_rules! prepare {
    ($i: ident) => {
        let mut mem = vec![0; 33554432]; // 32MB
        let mem = Box::new(Memory::new(&mut mem[..]));
        let $i = format(mem, FormatOptions::default())?;
    };
}
#[test]
fn statfs() -> DkResult<()> {
    prepare!(handle);
    assert_eq!(
        handle.statfs()?,
        Statvfs {
            blocks: 8063,
            bfree: 8062,
            bavail: 8062,
            files: 2048,
            ffree: 2047,
            bsize: 4096,
            namelen: 256,
        }
    );
    Ok(())
}

#[test]
fn get_root_attr() -> DkResult<()> {
    prepare!(handle);
    let stat = handle.getattr(ROOT_INODE)?;
    assert_eq!(stat.ino, ROOT_INODE);
    assert!(stat.mode.is_directory());
    assert_eq!(stat.uid, 0);
    assert_eq!(stat.gid, 0);
    assert_eq!(stat.rdev, 0);
    Ok(())
}

#[test]
fn mknod_in_root() -> DkResult<()> {
    prepare!(handle);
    let stat = handle.mknod(
        0,
        0,
        ROOT_INODE,
        OsStr::new("Homura"),
        FileMode::REGULAR_FILE | FileMode::USER_RWX,
    )?;
    assert_eq!(stat.uid, 0);
    assert_eq!(stat.gid, 0);
    assert_eq!(stat.nlink, 1);
    assert!(stat.mode.is_regular_file());
    assert!(stat.mode.contains(FileMode::USER_RWX));
    assert_eq!(handle.getattr(stat.ino)?, stat);
    assert_eq!(handle.lookup(ROOT_INODE, OsStr::new("Homura"))?, stat);
    Ok(())
}

#[test]
fn mknod_in_newdir() -> DkResult<()> {
    prepare!(handle);
    let stat = handle.mkdir(ROOT_INODE, 0, 0, OsStr::new("Madoka"), FileMode::USER_RWX)?;
    assert!(stat.mode.is_directory());
    let dir_ino = stat.ino;
    let homura = OsStr::new("Homura");
    let stat = handle.mknod(0, 0, dir_ino, homura, FileMode::REGULAR_FILE)?;
    assert_eq!(handle.lookup(dir_ino, homura)?, stat);
    let dir = handle.opendir(dir_ino)?;
    assert!(
        handle
            .readdir(dir, 0)
            .any(|(name, ino)| name == homura && ino == stat.ino)
    );
    Ok(())
}

#[test]
fn set_attrs_except_size() -> DkResult<()> {
    prepare!(handle);
    let stat = handle.mknod(
        0,
        0,
        ROOT_INODE,
        OsStr::new("Homura"),
        FileMode::REGULAR_FILE | FileMode::USER_RWX,
    )?;
    let ino = stat.ino;
    let stat = handle.setattr(
        ino,
        None,
        Some(FileMode::REGULAR_FILE | FileMode::USER_READ),
        Some(1000),
        Some(1000),
        None,
        Some(DkTimespec {
            sec: 612921600,
            nsec: 3,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 2,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 1,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 0,
        }),
    )?;
    assert_eq!(stat.ino, ino);
    assert_eq!(stat.mode, FileMode::REGULAR_FILE | FileMode::USER_READ);
    assert_eq!(stat.uid, 1000);
    assert_eq!(stat.gid, 1000);
    assert_eq!(
        stat.atime,
        DkTimespec {
            sec: 612921600,
            nsec: 3,
        }
    );
    assert_eq!(
        stat.mtime,
        DkTimespec {
            sec: 612921600,
            nsec: 2,
        }
    );
    assert_eq!(
        stat.ctime,
        DkTimespec {
            sec: 612921600,
            nsec: 1,
        }
    );
    assert_eq!(
        stat.crtime,
        DkTimespec {
            sec: 612921600,
            nsec: 0,
        }
    );
    assert_eq!(handle.getattr(ino)?, stat);
    Ok(())
}

#[test]
fn set_fh_attrs_except_size() -> DkResult<()> {
    prepare!(handle);
    let stat = handle.mknod(
        0,
        0,
        ROOT_INODE,
        OsStr::new("Homura"),
        FileMode::REGULAR_FILE | FileMode::USER_RWX,
    )?;
    let ino = stat.ino;
    let fh = handle.open(ino, Flags::READ_ONLY)?;
    let stat = handle.setattr(
        ino,
        Some(fh),
        Some(FileMode::REGULAR_FILE | FileMode::USER_READ),
        Some(1000),
        Some(1000),
        None,
        Some(DkTimespec {
            sec: 612921600,
            nsec: 3,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 2,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 1,
        }),
        Some(DkTimespec {
            sec: 612921600,
            nsec: 0,
        }),
    )?;
    assert_eq!(stat.ino, ino);
    assert_eq!(stat.mode, FileMode::REGULAR_FILE | FileMode::USER_READ);
    assert_eq!(stat.uid, 1000);
    assert_eq!(stat.gid, 1000);
    assert_eq!(
        stat.atime,
        DkTimespec {
            sec: 612921600,
            nsec: 3,
        }
    );
    assert_eq!(
        stat.mtime,
        DkTimespec {
            sec: 612921600,
            nsec: 2,
        }
    );
    assert_eq!(
        stat.ctime,
        DkTimespec {
            sec: 612921600,
            nsec: 1,
        }
    );
    assert_eq!(
        stat.crtime,
        DkTimespec {
            sec: 612921600,
            nsec: 0,
        }
    );
    assert_eq!(handle.getattr(ino)?, stat);
    Ok(())
}

#[test]
fn traverse_dir() -> DkResult<()> {
    prepare!(handle);
    let mut names: HashSet<OsString> = [
        "鹿目まどか",
        "暁美ほむら",
        "美樹さやか",
        "佐倉杏子",
        "巴マミ",
    ]
        .iter()
        .map(|name| name.to_string().into())
        .collect();
    for name in &names {
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE)?;
    }
    names.insert(".".to_string().into());
    names.insert("..".to_string().into());
    let dir = handle.opendir(ROOT_INODE)?;
    let names_read: HashSet<OsString> = handle.readdir(dir, 0).map(|(name, _)| name).collect();
    assert_eq!(names, names_read);
    Ok(())
}

#[test]
fn traverse_big_dir() -> DkResult<()> {
    prepare!(handle);
    let mut rng = thread_rng();
    let mut names: HashSet<OsString> = (0..1024)
        .map(|_| {
            rng.sample_iter(&Alphanumeric)
                .take(63)
                .collect::<String>()
                .into()
        }).collect();
    for name in &names {
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE)?;
    }
    names.insert(".".to_string().into());
    names.insert("..".to_string().into());
    let dir = handle.opendir(ROOT_INODE)?;
    let names_read: HashSet<OsString> = handle.readdir(dir, 0).map(|(name, _)| name).collect();
    assert_eq!(names, names_read);
    Ok(())
}

#[test]
fn read_write() -> DkResult<()> {
    prepare!(handle);
    let mut rng = thread_rng();
    let files: HashMap<OsString, Vec<u8>> = (0..16)
        .map(|i| {
            let name = rng
                .sample_iter(&Alphanumeric)
                .take(63)
                .collect::<String>()
                .into();
            let data = rng.sample_iter(&Standard).take(i * i * 16384).collect();
            (name, data)
        }).collect();
    for (name, data) in &files {
        println!("Write {:?}", name);
        let stat = handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE)?;
        let fh = handle.open(stat.ino, Flags::WRITE_ONLY)?;
        let len = handle.write(fh, 0, data)?;
        assert_eq!(len, data.len());
    }
    for (name, data) in &files {
        println!("Read {:?}", name);
        let stat = handle.lookup(ROOT_INODE, name)?;
        let fh = handle.open(stat.ino, Flags::READ_ONLY)?;
        let mut offset = 0;
        loop {
            let read = handle.read(fh.clone(), offset as u64, 4096)?;
            if read.len() == 0 {
                assert_eq!(offset, data.len());
                break;
            }
            assert_eq!(&data[offset..(offset + read.len())], &read[..]);
            offset += read.len();
        }
    }
    Ok(())
}

#[test]
fn xattrs() -> DkResult<()> {
    prepare!(handle);
    let madoka = OsStr::new("Madoka");
    let homura = OsStr::new("Homura");
    let stat = handle.mknod(0, 0, ROOT_INODE, homura, FileMode::REGULAR_FILE)?;
    assert!(handle.listxattr(stat.ino)?.is_empty());
    assert_eq!(handle.getxattr(stat.ino, madoka)?, None);

    handle.setxattr(stat.ino, madoka, "鹿目まどか".as_bytes())?;
    handle.setxattr(stat.ino, homura, "暁美ほむら".as_bytes())?;
    let v = handle.listxattr(stat.ino)?;
    let set: HashSet<_> = v.iter().map(|s| s.as_os_str()).collect();
    assert_eq!(set, [madoka, homura].iter().map(|s| *s).collect());
    Ok(())
}

#[test]
fn unlink() -> DkResult<()> {
    prepare!(handle);
    let mut names: HashSet<OsString> = [
        "鹿目まどか",
        "暁美ほむら",
        "美樹さやか",
        "佐倉杏子",
        "巴マミ",
    ]
        .iter()
        .map(|name| name.to_string().into())
        .collect();
    for name in &names {
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE)?;
    }
    let mami = OsStr::new("巴マミ");
    handle.unlink(ROOT_INODE, mami)?;
    names.remove(mami);
    names.insert(".".to_string().into());
    names.insert("..".to_string().into());
    let dir = handle.opendir(ROOT_INODE)?;
    let names_read: HashSet<OsString> = handle.readdir(dir, 0).map(|(name, _)| name).collect();
    assert_eq!(names, names_read);
    Ok(())
}
