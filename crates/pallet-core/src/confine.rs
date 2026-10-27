//! Kernel-enforced warehouse boundary (SAFETY.md S1), using Landlock.
//!
//! The thread that performs every mutation confines itself to the warehouse
//! root before opening it. Landlock then checks, inside each filesystem call,
//! that the object is still beneath the root. For a rename the check runs while
//! the kernel holds the filesystem's rename lock, so another program can't move
//! a bay out of the warehouse between the check and the rename. If that has
//! already happened, the rename is refused (EXDEV/EACCES) and nothing moves.
//!
//! Requires Landlock ABI 2 (Linux 5.19+, Landlock enabled in the LSM list) for
//! the REFER right that governs cross-directory renames. Without it, real-file
//! mode is refused.

use landlock::{
    Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, ABI,
};
use std::path::Path;

/// Restricts the calling thread (and threads it spawns later) to filesystem
/// access beneath `root`. Other threads of the process are unaffected.
pub fn confine_current_thread(root: &Path) -> Result<(), String> {
    let abi = ABI::V2;
    let access = AccessFs::from_all(abi);
    let fail = |e: &dyn std::fmt::Display| format!("Landlock confinement isn't available ({e}). Real-file mode needs Linux 5.19+ with Landlock enabled.");
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(access)
        .map_err(|e| fail(&e))?
        .create()
        .map_err(|e| fail(&e))?
        .add_rule(PathBeneath::new(PathFd::new(root).map_err(|e| fail(&e))?, access))
        .map_err(|e| fail(&e))?
        .restrict_self()
        .map_err(|e| fail(&e))?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(fail(&"the ruleset was only partially enforced"));
    }
    Ok(())
}
