//! Platform-neutral view of filesystem objects. Each adapter (`sys` on Linux,
//! `sys_windows` on Windows) fills these in from its native calls, so the
//! warehouse logic is shared and the same rules apply everywhere.

use crate::journal::{Fingerprint, Ident};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// Symlink, junction, or other reparse point. Never followed, never cargo.
    Link,
    /// Pipe, socket, device, or anything else that isn't plain data.
    Other(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Meta {
    pub id: Ident,
    pub kind: Kind,
    pub size: u64,
    pub mtime_ns: i128,
    pub links: u64,
    /// Product rule (S4): no owner-write bit, or the read-only attribute, means not cargo.
    pub writable: bool,
}

impl Meta {
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            size: self.size,
            mtime_ns: self.mtime_ns,
        }
    }
}

/// What an I/O error means for the commit protocol, independent of platform codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errc {
    NotFound,
    Exists,
    CrossDevice,
    Unsupported,
    Permission,
    NoSpace,
    ReadOnlyFs,
    /// A link or reparse point in the path.
    Loop,
    /// Another program has the object open in a way that blocks this.
    Busy,
    Other,
}
