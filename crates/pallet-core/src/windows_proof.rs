//! Minimal Windows-native proof for PALLET's future NTFS adapter.
//!
//! This deliberately is **not** wired into `Warehouse` yet: it establishes the
//! critical primitive first—rename an already-open file relative to an
//! already-open destination directory, with replacement disabled. The full
//! adapter must additionally resolve every component from the retained root
//! handle and reject reparse points before real-file mode can be enabled.

use std::ffi::OsStr;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_RENAME_INFO_0, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileRenameInfo, OPEN_EXISTING, DELETE,
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn err() -> io::Error {
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

/// RAII wrapper used throughout the proof so failure paths cannot leak a
/// native handle.
pub struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Handle {
    pub fn open(path: &Path, access: u32, flags: u32) -> io::Result<Self> {
        let path = wide(path.as_os_str());
        let h = unsafe {
            CreateFileW(
                path.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(), OPEN_EXISTING, flags, std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE { Err(err()) } else { Ok(Self(h)) }
    }

    /// `(volume serial, 64-bit file ID)` evidence used by the future journal.
    pub fn identity(&self) -> io::Result<(u64, u64)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
            return Err(err());
        }
        Ok((
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ))
    }
}

/// Opens the source once and publishes its new name using `FileRenameInfo`
/// relative to `destination_dir`. `ReplaceIfExists` is always false; a
/// collision is returned from the kernel and never overwritten.
pub fn rename_open_file_noreplace(source: &Path, destination_dir: &Path, name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.ends_with(['.', ' ']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe Windows destination name"));
    }
    let source = Handle::open(source, DELETE | FILE_READ_ATTRIBUTES, FILE_FLAG_OPEN_REPARSE_POINT)?;
    let destination = Handle::open(
        destination_dir,
        FILE_READ_ATTRIBUTES,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let utf16: Vec<u16> = OsStr::new(name).encode_wide().collect();
    let bytes = (utf16.len() * size_of::<u16>()) as u32;
    // FILE_RENAME_INFO ends with a one-WCHAR placeholder, but Rust rounds the
    // structure itself up to pointer alignment. Windows validates the full
    // `sizeof(INFO) - sizeof(WCHAR) + FileNameLength` layout, including that
    // trailing alignment, rather than the byte offset of FileName alone.
    let base = offset_of!(FILE_RENAME_INFO, FileName);
    let record_len = size_of::<FILE_RENAME_INFO>() - size_of::<u16>() + bytes as usize;
    debug_assert!(record_len >= base + bytes as usize);
    let mut record = vec![0u8; record_len];
    let info = record.as_mut_ptr() as *mut FILE_RENAME_INFO;
    unsafe {
        (*info).Anonymous = FILE_RENAME_INFO_0 { ReplaceIfExists: false };
        (*info).RootDirectory = destination.0;
        (*info).FileNameLength = bytes;
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), (*info).FileName.as_mut_ptr(), utf16.len());
        if SetFileInformationByHandle(source.0, FileRenameInfo, info.cast(), record.len() as u32) == 0 {
            return Err(err());
        }
    }
    Ok(())
}
