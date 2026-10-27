//! Crash-test hooks. With the `failpoints` feature:
//!
//! - `PALLET_FAILPOINT=<name>` aborts the process at that point (no destructors,
//!   no buffered writes), `<name>:pause` prints `FAILPOINT <name>` and waits to
//!   be SIGKILLed.
//! - `<name>:enospc`, `:eio`, `:exdev` make an I/O step fail with that error.
//!   Append `@n` to fire only on the n-th hit (`journal_append:enospc@3`).
//! - In-process tests can use [`scoped`] and [`set_hook`], which are thread-local
//!   so parallel tests don't interfere.
//!
//! Without the feature these compile to nothing.

/// A test callback run at each failpoint.
#[cfg(feature = "failpoints")]
pub type Hook = Box<dyn FnMut(&str)>;

#[cfg(feature = "failpoints")]
mod imp {
    use super::Hook;
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static SPEC: RefCell<Option<String>> = const { RefCell::new(None) };
        static COUNTS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
        static HOOK: RefCell<Option<Hook>> = RefCell::new(None);
    }

    pub fn action(name: &str) -> Option<String> {
        let spec = SPEC
            .with(|s| s.borrow().clone())
            .or_else(|| std::env::var("PALLET_FAILPOINT").ok())?;
        for part in spec.split(',') {
            let (point, rest) = part.split_once(':').unwrap_or((part, "abort"));
            if point != name {
                continue;
            }
            let (action, nth) = match rest.split_once('@') {
                Some((a, n)) => (a, n.parse::<u32>().ok()),
                None => (rest, None),
            };
            let hits = COUNTS.with(|c| {
                let mut c = c.borrow_mut();
                let e = c.entry(part.to_string()).or_insert(0);
                *e += 1;
                *e
            });
            if nth.is_none_or(|n| n == hits) {
                return Some(action.to_string());
            }
        }
        None
    }

    pub fn run_hook(name: &str) {
        let hook = HOOK.with(|h| h.borrow_mut().take());
        if let Some(mut f) = hook {
            f(name);
            HOOK.with(|h| {
                let mut slot = h.borrow_mut();
                if slot.is_none() {
                    *slot = Some(f);
                }
            });
        }
    }

    pub fn scoped<T>(spec: &str, f: impl FnOnce() -> T) -> T {
        SPEC.with(|s| *s.borrow_mut() = Some(spec.to_string()));
        COUNTS.with(|c| c.borrow_mut().clear());
        let out = f();
        SPEC.with(|s| *s.borrow_mut() = None);
        out
    }

    pub fn set_hook(f: Option<Hook>) {
        HOOK.with(|h| *h.borrow_mut() = f);
    }
}

/// Runs `f` with failpoint spec `spec` active on this thread only.
#[cfg(feature = "failpoints")]
pub fn scoped<T>(spec: &str, f: impl FnOnce() -> T) -> T {
    imp::scoped(spec, f)
}

/// Installs a callback run at every [`hit`] on this thread (tests use it to
/// race the commit: create a colliding name, swap a folder, rename the root).
#[cfg(feature = "failpoints")]
pub fn set_hook(f: Option<Hook>) {
    imp::set_hook(f)
}

/// A point where a crash test may terminate the process.
#[inline]
pub fn hit(name: &str) {
    #[cfg(feature = "failpoints")]
    {
        imp::run_hook(name);
        match imp::action(name).as_deref() {
            Some("abort") => std::process::abort(),
            Some("pause") => {
                use std::io::Write;
                println!("FAILPOINT {name}");
                let _ = std::io::stdout().flush();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
            }
            _ => {}
        }
    }
    let _ = name;
}

/// True when `name` is configured as `torn`: the caller writes part of its
/// record and then aborts, producing a real torn journal tail.
#[inline]
pub fn torn(name: &str) -> bool {
    #[cfg(feature = "failpoints")]
    {
        return imp::action(name).as_deref() == Some("torn");
    }
    #[allow(unreachable_code)]
    {
        let _ = name;
        false
    }
}

/// Returns an injected I/O error at `name`, if configured.
#[inline]
pub fn error(name: &str) -> std::io::Result<()> {
    #[cfg(feature = "failpoints")]
    {
        let code = match imp::action(name).as_deref() {
            Some("enospc") => Some(28),
            Some("eio") => Some(5),
            Some("exdev") => Some(18),
            _ => None,
        };
        if let Some(code) = code {
            return Err(std::io::Error::from_raw_os_error(code));
        }
    }
    let _ = name;
    Ok(())
}
