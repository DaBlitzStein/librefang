//! Shared test helper for tests that mutate process-global environment
//! variables.
//!
//! `doctor.rs` and `commands/skill.rs` both compile into the same
//! `librefang-cli` unit-test binary, and `cargo test` runs tests in parallel,
//! so every env-mutating test must take the one lock in this module — a
//! private `ENV_LOCK` in one module excludes the sibling module's tests and
//! nothing else, which is exactly the race the #8179 review found.

#![cfg(test)]

use std::sync::MutexGuard;

/// Process-wide lock for tests that mutate `LIBREFANG_VAULT_KEY`,
/// `GITHUB_TOKEN` and every other process-global environment variable.
/// `cargo test` runs tests in parallel by default, and env-var mutation is
/// process-global, so without serialization these races clobber each other.
///
/// Delegates to [`crate::test_env_lock::env_lock`] rather than owning a
/// `OnceLock<Mutex<()>>` of its own: #8239 landed that module for the same
/// reason this one exists, and two modules each holding "the" process-wide
/// mutex is the very race both were written to close — `templates.rs` and
/// `launcher.rs` would be serializing on one mutex while `doctor.rs` and
/// `commands/skill.rs` serialize on another.
pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    crate::test_env_lock::env_lock()
}

/// Run `f` with each `(key, value)` applied to the process environment —
/// `None` removes the variable — under the shared [`env_lock`].
///
/// Every variable listed is saved when the call starts and restored when it
/// ends, **including when `f` panics**, so a failing assertion cannot leave
/// `LIBREFANG_VAULT_KEY` (or a developer's exported `GITHUB_TOKEN`) set for
/// every test that runs afterwards. Each `key` may appear once per call.
pub(crate) fn with_env_vars<F, R>(specs: &[(&'static str, Option<&str>)], f: F) -> R
where
    F: FnOnce() -> R,
{
    struct Restore {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            for (key, prev) in &self.saved {
                // SAFETY: the shared env lock is held for this closure's whole
                // lifetime, so no concurrent test reads or writes these
                // variables.
                unsafe {
                    match prev {
                        Some(prev) => std::env::set_var(key, prev),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    let _lock = env_lock();
    let saved = specs
        .iter()
        .map(|(key, _)| (*key, std::env::var(key).ok()))
        .collect::<Vec<_>>();
    let restore = Restore { saved };
    for (key, value) in specs {
        // SAFETY: same as Restore::drop.
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    let result = f();
    drop(restore);
    result
}
