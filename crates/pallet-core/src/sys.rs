//! Linux platform adapter (S1, S3). Every descendant of the warehouse root is
//! reached through `openat2` relative to the root handle with
//! `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV`.
//! Mutations are `*at` calls on directory handles. No path string is
//! re-resolved at rename time.

use crate::journal::Ident;
use crate::platform::{Errc, Kind, Meta};
use rustix::fd::OwnedFd;
use rustix::fs::{
    self, Access, AtFlags, FileType, FlockOperation, Mode, OFlags, RenameFlags, ResolveFlags, Stat,
};
use std::fs::File;
use std::io;
use std::path::Path;

/// `EXT2/3/4_SUPER_MAGIC`. ext4 is the only filesystem the v1 adapter is tested on.
pub const EXT4_MAGIC: u64 = 0xEF53;
/// `FS_CASEFOLD_FL`. Casefolded directories change collision rules; refused in v1.
const FS_CASEFOLD_FL: u32 = 0x4000_0000;

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS)
    .union(ResolveFlags::NO_XDEV);

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW);

fn err(e: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(e.raw_os_error())
}

/// A directory handle. Every descendant is reached through one of these.
pub type Dir = OwnedFd;

fn meta(st: &Stat) -> Meta {
    let kind = match FileType::from_raw_mode(st.st_mode) {
        FileType::RegularFile => Kind::File,
        FileType::Directory => Kind::Dir,
        FileType::Symlink => Kind::Link,
        FileType::Fifo => Kind::Other("pipe"),
        FileType::Socket => Kind::Other("socket"),
        FileType::CharacterDevice | FileType::BlockDevice => Kind::Other("device"),
        _ => Kind::Other("special file"),
    };
    #[allow(clippy::unnecessary_cast)]
    Meta {
        id: ident(st),
        kind,
        size: st.st_size as u64,
        mtime_ns: st.st_mtime as i128 * 1_000_000_000 + st.st_mtime_nsec as i128,
        links: st.st_nlink as u64,
        writable: st.st_mode & 0o200 != 0,
    }
}

pub fn classify(e: &io::Error) -> Errc {
    match e.raw_os_error() {
        Some(2) | Some(20) => Errc::NotFound,
        Some(17) | Some(39) => Errc::Exists,
        Some(18) => Errc::CrossDevice,
        Some(22) | Some(95) => Errc::Unsupported,
        Some(1) | Some(13) => Errc::Permission,
        Some(28) | Some(122) => Errc::NoSpace,
        Some(30) => Errc::ReadOnlyFs,
        Some(40) => Errc::Loop,
        Some(16) | Some(26) => Errc::Busy,
        _ => Errc::Other,
    }
}

/// Real-file mode is tested on ext4 only; anything else is refused.
pub fn fs_check(root: &Dir) -> Result<(), String> {
    let magic = fs_magic(root).map_err(|e| e.to_string())?;
    if magic != EXT4_MAGIC {
        return Err(format!("This folder is on a filesystem PALLET hasn't been tested on (type 0x{magic:x}). Real-file mode supports ext4 only."));
    }
    Ok(())
}

pub fn file_meta(f: &File) -> io::Result<Meta> {
    Ok(meta(&fs::fstat(f).map_err(err)?))
}

/// Permission bits to give a verified copy.
pub fn file_mode(f: &File) -> io::Result<u32> {
    Ok(fs::fstat(f).map_err(err)?.st_mode & 0o777)
}

/// Positional read that doesn't move the file cursor.
pub fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, off)
}

fn ident(st: &Stat) -> Ident {
    #[allow(clippy::unnecessary_cast)] // st_dev/st_ino widths vary by target
    Ident {
        dev: st.st_dev as u64,
        ino: st.st_ino as u64,
    }
}

/// Opens the root itself. The final component must not be a symlink.
pub fn open_root(path: &Path) -> io::Result<OwnedFd> {
    fs::open(path, DIR_FLAGS, Mode::empty()).map_err(err)
}

/// Opens a directory below `root` by components. `rel` is empty for the root.
/// Components were validated as names, but the kernel enforces the boundary.
pub fn open_dir_beneath(root: &OwnedFd, rel: &[String]) -> io::Result<OwnedFd> {
    let path = if rel.is_empty() {
        ".".to_string()
    } else {
        rel.join("/")
    };
    fs::openat2(root, path, DIR_FLAGS, Mode::empty(), RESOLVE).map_err(err)
}

pub fn fstat(fd: &OwnedFd) -> io::Result<Meta> {
    Ok(meta(&fs::fstat(fd).map_err(err)?))
}

/// `lstat` of one entry inside an open directory.
pub fn lstat_at(dir: &OwnedFd, name: &str) -> io::Result<Meta> {
    Ok(meta(
        &fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?,
    ))
}

/// `lstat` of an absolute path. Used only to check whether the root's original
/// pathname still names the bound directory (S4, root replaced).
pub fn lstat_path(path: &Path) -> io::Result<Meta> {
    Ok(meta(
        &fs::statat(fs::CWD, path, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?,
    ))
}

/// Atomic no-replace rename. `EINVAL` (no RENAME_NOREPLACE support) is returned as
/// is and becomes a refusal; there is deliberately no fallback.
pub fn rename_noreplace(
    src_dir: &OwnedFd,
    src: &str,
    dst_dir: &OwnedFd,
    dst: &str,
) -> io::Result<()> {
    fs::renameat_with(src_dir, src, dst_dir, dst, RenameFlags::NOREPLACE).map_err(err)
}

pub fn fsync(fd: &OwnedFd) -> io::Result<()> {
    fs::fsync(fd).map_err(err)
}

pub fn fs_magic(fd: &OwnedFd) -> io::Result<u64> {
    Ok(fs::fstatfs(fd).map_err(err)?.f_type as u64)
}

pub fn is_casefold(dir: &OwnedFd) -> io::Result<bool> {
    let flags = fs::ioctl_getflags(dir).map_err(err)?;
    Ok(flags.bits() & FS_CASEFOLD_FL != 0)
}

pub fn free_bytes(fd: &OwnedFd) -> io::Result<u64> {
    let s = fs::fstatvfs(fd).map_err(err)?;
    Ok(s.f_bavail.saturating_mul(s.f_frsize))
}

/// Whether this process may create and remove entries in `dir`.
pub fn dir_writable(dir: &OwnedFd) -> bool {
    fs::accessat(
        dir,
        ".",
        Access::WRITE_OK | Access::EXEC_OK,
        AtFlags::EACCESS,
    )
    .is_ok()
}

/// Creates a new file that must not already exist.
pub fn create_excl(dir: &OwnedFd, name: &str, mode: u32) -> io::Result<File> {
    let fd = fs::openat2(
        dir,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(mode),
        RESOLVE,
    )
    .map_err(err)?;
    Ok(File::from(fd))
}

/// Opens or creates one of PALLET's own metadata files inside `.pallet`.
pub fn open_meta_file(meta: &OwnedFd, name: &str, append: bool) -> io::Result<File> {
    let mut flags = OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    if append {
        flags |= OFlags::APPEND;
    }
    let fd = fs::openat2(meta, name, flags, Mode::from_raw_mode(0o600), RESOLVE).map_err(err)?;
    Ok(File::from(fd))
}

pub fn open_file_ro(dir: &OwnedFd, name: &str) -> io::Result<File> {
    let fd = fs::openat2(
        dir,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
        RESOLVE,
    )
    .map_err(err)?;
    Ok(File::from(fd))
}

/// Copies permission bits to a new file (the copy matches its original).
pub fn set_mode(file: &File, mode: u32) -> io::Result<()> {
    fs::fchmod(file, Mode::from_raw_mode(mode)).map_err(err)
}

/// Creates a directory; an existing directory is fine. The caller reopens it
/// with `open_dir_beneath` to confirm it is a real directory on this device.
pub fn mkdir_at(dir: &OwnedFd, name: &str, mode: u32) -> io::Result<bool> {
    match fs::mkdirat(dir, name, Mode::from_raw_mode(mode)) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::EXIST) => Ok(false),
        Err(e) => Err(err(e)),
    }
}

/// Entry names in a directory, excluding `.` and `..`. Names that aren't
/// UTF-8 are returned lossily with a flag so the caller can mark them unavailable.
pub fn read_dir(dir: &OwnedFd) -> io::Result<Vec<(String, bool)>> {
    let mut out = Vec::new();
    let reader = fs::Dir::read_from(dir).map_err(err)?;
    for entry in reader {
        let entry = entry.map_err(err)?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        match std::str::from_utf8(bytes) {
            Ok(s) => out.push((s.to_string(), true)),
            Err(_) => out.push((String::from_utf8_lossy(bytes).into_owned(), false)),
        }
    }
    Ok(out)
}

/// Non-blocking exclusive lock. `WouldBlock` means another instance holds it.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    fs::flock(file, FlockOperation::NonBlockingLockExclusive).map_err(err)
}
