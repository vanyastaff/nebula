//! Test-only helpers for manipulating process environment variables.
//!
//! `std::env::{set_var, remove_var}` are `unsafe` under edition 2024 because
//! they mutate global process state without synchronization. [`EnvGuard`]
//! serializes all mutation behind a process-global lock and restores the
//! prior values on drop, so tests never clobber each other under nextest's
//! in-process parallelism. This replaces the hand-rolled `env_lock` / `ENV_LOCK`
//! harnesses previously duplicated across `nebula-api`, `nebula-log`, and
//! `nebula-storage` test modules.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// RAII guard that serializes env mutation and restores prior values on drop.
///
/// Acquire one per test via [`EnvGuard::acquire`] and hold it for the test
/// body; every [`set`](EnvGuard::set) / [`remove`](EnvGuard::remove) is undone
/// when the guard drops.
#[must_use = "the guard must be held for the duration of the test"]
pub struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: HashMap<String, Option<String>>,
}

impl EnvGuard {
    /// Acquire the process-global env lock.
    pub fn acquire() -> Self {
        Self {
            _lock: lock(),
            saved: HashMap::new(),
        }
    }

    /// Set `key` to `value`, remembering the prior value for restoration.
    pub fn set(&mut self, key: &str, value: &str) {
        self.remember(key);
        // guard-justified: edition-2024 set_var is unsafe; the held lock
        // serializes all env mutation so no other thread races this write.
        #[allow(unsafe_code)]
        // SAFETY: `self._lock` is held for the lifetime of this guard, so no
        // concurrent reader or writer of the environment can observe a torn
        // state during this single-threaded mutation.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    /// Remove `key`, remembering the prior value for restoration.
    pub fn remove(&mut self, key: &str) {
        self.remember(key);
        // guard-justified: edition-2024 remove_var is unsafe; serialized by
        // the held process-global lock, same invariant as `set`.
        #[allow(unsafe_code)]
        // SAFETY: the held lock serializes this mutation; see `set`.
        unsafe {
            std::env::remove_var(key);
        }
    }

    fn remember(&mut self, key: &str) {
        self.saved
            .entry(key.to_owned())
            .or_insert_with(|| std::env::var(key).ok());
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prior) in &self.saved {
            // guard-justified: restoring env on drop is unsafe under edition
            // 2024 but serialized by the still-held lock.
            #[allow(unsafe_code)]
            // SAFETY: the lock is held until this Drop completes, so the
            // restore is serialized against all other env access.
            unsafe {
                match prior {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
