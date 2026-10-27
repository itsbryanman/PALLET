//! Native user-mode proof for PALLET's future NTFS adapter.
//!
//! This calls ntdll's `NtSetInformationFile` because the Win32 wrapper cannot
//! reliably perform a relative rename rooted at an already-open directory.
//! It is not the production adapter: its path opens are absolute; production
//! confinement must resolve descendants from retained handles and reject
//! reparse points.

use std::ffi::{c_void, OsStr};
use std::io;
use std::mem::{align_of, offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ADD_FILE,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, OPEN_EXISTING,
};

const FILE_RENAME_INFORMATION: u32 = 10;
const STATUS_PENDING: i32 = 0x0000_0103;

#[repr(C)]
struct IoStatusBlock {
    status: i32,
    information: usize,
}

#[repr(C)]
struct RenameHeader {
    replace_if_exists: u8,
    _padding: [u8; 7],
    root_directory: HANDLE,
    file_name_length: u32,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetInformationFile(
        file_handle: HANDLE,
        io_status_block: *mut IoStatusBlock,
        file_information: *mut c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn win32_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn ntstatus_error(stage: &str, status: i32) -> io::Error {
    io::Error::other(format!(
        "{stage} failed with NTSTATUS 0x{:08X}",
        status as u32
    ))
}

/// Explicitly aligned storage for a variable-size native structure. `len` is
/// the exact submitted byte count; trailing word padding is never submitted.
struct AlignedBytes {
    words: Vec<usize>,
    len: usize,
}

impl AlignedBytes {
    fn zeroed(len: usize) -> io::Result<Self> {
        let word = size_of::<usize>();
        let words = len
            .checked_add(word - 1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer overflow"))?
            / word;
        Ok(Self {
            words: vec![0; words],
            len,
        })
    }
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr().cast()
    }
}

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
                path.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                flags,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            Err(win32_error())
        } else {
            Ok(Self(h))
        }
    }

    pub fn identity(&self) -> io::Result<(u64, u64)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
            return Err(win32_error());
        }
        Ok((
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ))
    }
}

/// Renames an already-open source using native FileRenameInformation (10),
/// rooted at an already-open destination directory. Replacement is false.
pub fn rename_open_file_noreplace(
    source: &Path,
    destination_dir: &Path,
    name: &str,
) -> io::Result<()> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.ends_with(['.', ' ']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe Windows destination name",
        ));
    }
    let utf16: Vec<u16> = OsStr::new(name).encode_wide().collect();
    let name_bytes = utf16
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination name too long"))?;
    let submitted_len = size_of::<RenameHeader>()
        .checked_add(name_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer overflow"))?;
    let submitted_len = u32::try_from(submitted_len)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer too large"))?;

    assert_eq!(align_of::<RenameHeader>(), align_of::<usize>());
    assert_eq!(offset_of!(RenameHeader, root_directory), 8);
    assert_eq!(offset_of!(RenameHeader, file_name_length), 16);
    assert_eq!(size_of::<RenameHeader>(), 24);

    let source = Handle::open(
        source,
        DELETE | FILE_READ_ATTRIBUTES,
        FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let destination = Handle::open(
        destination_dir,
        FILE_READ_ATTRIBUTES | FILE_TRAVERSE | FILE_ADD_FILE,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let mut storage = AlignedBytes::zeroed(submitted_len as usize)?;
    debug_assert_eq!(storage.len, submitted_len as usize);
    let info = storage.as_mut_ptr().cast::<RenameHeader>();
    unsafe {
        (*info).replace_if_exists = 0;
        (*info).root_directory = destination.0;
        (*info).file_name_length = name_bytes as u32;
        std::ptr::copy_nonoverlapping(
            utf16.as_ptr(),
            storage.as_mut_ptr().add(size_of::<RenameHeader>()).cast(),
            utf16.len(),
        );
    }
    let mut iosb = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let status = unsafe {
        NtSetInformationFile(
            source.0,
            &mut iosb,
            storage.as_mut_ptr().cast(),
            submitted_len,
            FILE_RENAME_INFORMATION,
        )
    };
    if status == STATUS_PENDING {
        return Err(io::Error::other(
            "NtSetInformationFile returned STATUS_PENDING for a synchronous handle",
        ));
    }
    if status < 0 {
        return Err(ntstatus_error("NtSetInformationFile", status));
    }
    if iosb.status < 0 {
        return Err(ntstatus_error(
            "NtSetInformationFile completion",
            iosb.status,
        ));
    }
    Ok(())
}
