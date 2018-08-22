extern crate dkfs;
extern crate rand;

use dkfs::device::Memory;
use dkfs::replies::*;
use dkfs::*;
use rand::distributions::{Alphanumeric, Standard};
use rand::prng::XorShiftRng;
use rand::{thread_rng, Rng, SeedableRng};
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

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
        None,
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
    let stat = handle.mknod(0, 0, dir_ino, homura, FileMode::REGULAR_FILE, None)?;
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
        None,
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
        None,
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
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE, None)?;
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
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE, None)?;
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
    let mut rng = XorShiftRng::from_seed([1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0, 8, 9, 3]);
    let files: BTreeMap<OsString, Vec<u8>> = (0..=16)
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
        let stat = handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE, None)?;
        let fh = handle.open(stat.ino, Flags::WRITE_ONLY)?;
        let len = handle.write(fh, 0, data)?;
        assert_eq!(len, data.len());
    }
    // A rough estimate
    assert!(handle.statfs()?.bfree < 2078);
    for (name, data) in &files {
        let stat = handle.lookup(ROOT_INODE, name)?;
        assert!(stat.blocks >= stat.size / 512);
        let fh = handle.open(stat.ino, Flags::READ_ONLY)?;

        // Read once
        let read = handle.read(fh.clone(), 0, data.len() as u64)?;
        assert_eq!(read.len(), data.len());
        assert_eq!(&read, data);

        // Offset read
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
    let stat = handle.mknod(0, 0, ROOT_INODE, homura, FileMode::REGULAR_FILE, None)?;
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
        handle.mknod(0, 0, ROOT_INODE, name, FileMode::REGULAR_FILE, None)?;
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

#[test]
fn shrink_size() -> DkResult<()> {
    prepare!(handle);

    let mut rng = XorShiftRng::from_seed([1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0, 8, 9, 3]);
    let homura = OsStr::new("Homura");
    let data: Vec<u8> = rng.sample_iter(&Standard).take(1 << 24).collect(); // 16 MB
    let stat = handle.mknod(0, 0, ROOT_INODE, homura, FileMode::REGULAR_FILE, None)?;
    let statfs = handle.statfs()?;

    let fh = handle.open(stat.ino, Flags::WRITE_ONLY)?;
    let len = handle.write(fh.clone(), 0, &data)?;
    assert_eq!(len, data.len());
    assert_ne!(handle.statfs()?, statfs);
    drop(fh);
    assert_ne!(handle.statfs()?, statfs);

    let fh = handle.open(stat.ino, Flags::READ_ONLY)?;
    let read = handle.read(fh, 0, data.len() as u64)?;
    assert_eq!(data.len(), read.len());

    handle.setattr(
        stat.ino,
        None,
        None,
        None,
        None,
        Some(0),
        None,
        None,
        None,
        None,
    )?; // Set size to 0
    assert_eq!(handle.getattr(stat.ino)?.size, 0);
    assert_eq!(handle.getattr(stat.ino)?.blocks, 0);
    assert_eq!(handle.statfs()?, statfs);

    Ok(())
}

#[test]
fn rename() -> DkResult<()> {
    prepare!(handle);

    let homura = OsStr::new("Homura");
    let madoka = OsStr::new("Madoka");
    let stat = handle.mknod(0, 0, ROOT_INODE, homura, FileMode::REGULAR_FILE, None)?;
    let new_dir = handle
        .mkdir(ROOT_INODE, 0, 0, OsStr::new("newdir"), FileMode::USER_RWX)?
        .ino;
    handle.rename(ROOT_INODE, homura, new_dir, madoka)?;
    assert!(handle.lookup(ROOT_INODE, homura).is_err());
    assert_eq!(stat.blocks, handle.lookup(new_dir, madoka)?.blocks);
    Ok(())
}

#[test]
fn rmdir() -> DkResult<()> {
    prepare!(handle);

    let homura = OsStr::new("Homura");
    let madoka = OsStr::new("Madoka");
    let statfs = handle.statfs()?;
    let new_dir = handle.mkdir(ROOT_INODE, 0, 0, homura, FileMode::USER_RWX)?;
    handle.mknod(0, 0, new_dir.ino, madoka, FileMode::REGULAR_FILE, None)?;
    assert_eq!(handle.getattr(ROOT_INODE)?.nlink, 3);
    assert!(handle.rmdir(ROOT_INODE, homura).is_err());
    handle.unlink(new_dir.ino, madoka)?;
    handle.rmdir(ROOT_INODE, homura)?;
    assert!(handle.lookup(ROOT_INODE, homura).is_err());
    handle.apply_releases()?;
    assert_eq!(handle.getattr(ROOT_INODE)?.nlink, 2);
    assert_eq!(statfs, handle.statfs()?);
    Ok(())
}

#[test]
fn symlink() -> DkResult<()> {
    prepare!(handle);
    let homura = OsStr::new("Homura");
    let homura_link = OsStr::new("/暁美ほむら");
    let path = Path::new(homura_link);
    let link = handle.symlink(0, 0, ROOT_INODE, homura, path)?;
    assert!(link.mode.contains(
        FileMode::SYMBOLIC_LINK | FileMode::USER_RWX | FileMode::GROUP_RWX | FileMode::OTHERS_RWX
    ));

    let fh = handle.open(link.ino, Flags::READ_ONLY)?;
    let read = handle.read(fh, 0, 4096)?;
    assert_eq!(homura_link.as_bytes(), read.as_slice());
    Ok(())
}

#[test]
fn exhaust_inodes() -> DkResult<()> {
    prepare!(handle);
    let mut rng = XorShiftRng::from_seed([1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0, 8, 9, 3]);
    let mut names: HashSet<OsString> = HashSet::new();
    let statfs = handle.statfs()?;
    while names.len() < statfs.ffree as usize {
        names.insert(
            rng.sample_iter(&Alphanumeric)
                .take(63)
                .collect::<String>()
                .into(),
        );
    }
    let mut dirs = Vec::new();
    for names in names.iter().take(100) {
        dirs.push(
            handle
                .mkdir(ROOT_INODE, 0, 0, names, FileMode::empty())?
                .ino,
        );
    }
    let statfs = handle.statfs()?;
    let mut unused: Vec<&OsStr> = names
        .iter()
        .take(statfs.ffree as usize)
        .map(|s| s.as_os_str())
        .collect();;
    let mut used = Vec::new();
    for i in 0..20000 {
        let r = rng.gen::<f64>();
        if r < 0.05 {
            rng.shuffle(unused.as_mut());
            rng.shuffle(used.as_mut());
        } else if r < 0.4 {
            if let Some((ino, name)) = used.pop() {
                handle.unlink(ino, name)?;
                unused.push(name);
            }
        } else {
            if let Some(name) = unused.pop() {
                let ino = dirs[i % dirs.len()];
                handle.mknod(0, 0, ino, name, FileMode::REGULAR_FILE, None)?;
                used.push((ino, name));
            }
        }
    }
    while let Some((ino, name)) = used.pop() {
        handle.unlink(ino, name)?;
    }
    handle.apply_releases()?;
    assert_eq!(statfs, handle.statfs()?);
    Ok(())
}
