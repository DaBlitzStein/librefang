//! Shared env-var mutation lock for `#[cfg(test)]` code across this binary (#8239).
//!
//! `cargo test` runs every test in this crate's single binary on one process
//! with parallel threads, so two modules that each mutate the same
//! process-global env var through their own private mutex can still race —
//! only tests within the *same* module were ever serialized against each
//! other. `LIBREFANG_HOME` is mutated by both `templates.rs` and
//! `launcher.rs`; anything that touches it must lock here instead of
//! growing yet another module-private mutex.

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Acquire the crate-wide env-var test lock.
/// Recovers from a poisoned mutex (a panicking sibling test must not hang or
/// silently skip the rest of the suite).
pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}
