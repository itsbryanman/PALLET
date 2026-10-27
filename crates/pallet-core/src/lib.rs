//! PALLET's filesystem core. See SAFETY.md at the repository root; clause
//! numbers (S1–S7) in comments refer to it.

pub mod failpoint;
pub mod journal;
pub mod model;
pub mod names;
pub mod platform;
pub mod sample;

#[cfg(target_os = "linux")]
pub mod confine;
#[cfg(target_os = "linux")]
pub mod sys;
#[cfg(target_os = "linux")]
pub mod warehouse;
#[cfg(target_os = "linux")]
pub mod worker;
#[cfg(windows)]
pub mod windows_proof;

/// Whether this build has a real-file adapter at all. Runtime checks
/// (filesystem type, kernel support) happen when a warehouse is opened.
pub const REAL_FILES_ADAPTER: Option<&str> = if cfg!(target_os = "linux") {
    Some("linux-ext4")
} else {
    None
};

/// Why real-file mode is off on this platform, for the UI.
pub fn unsupported_reason() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        None
    } else {
        Some("PALLET doesn't have a tested real-file adapter for this operating system yet, so this build only runs the virtual sample warehouse.")
    }
}
