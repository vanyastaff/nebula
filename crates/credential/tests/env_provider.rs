//! Integration test for [`EnvKeyProvider::from_env`].
//!
//! Lives outside the crate proper because the crate applies
//! `#![forbid(unsafe_code)]` and `std::env::set_var` / `std::env::remove_var`
//! are `unsafe` under the Rust 2024 edition. The in-crate unit tests cover
//! the validation logic via `from_base64`; this binary exercises the env
//! lookup itself and verifies the fail-closed contract against a missing
//! `NEBULA_CRED_MASTER_KEY`.

use base64::Engine;
use nebula_credential::{EnvKeyProvider, ProviderError};

/// Serialize env-var manipulation across tests in this binary so parallel
/// nextest execution does not clobber shared state.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn clear_env() {
    // SAFETY: the env_lock() mutex serialises env mutations across this
    // binary's tests, so no other test thread reads or writes the var while
    // we mutate it. This is the same pattern used in
    // `crates/api/src/config.rs` tests.
    unsafe {
        std::env::remove_var(EnvKeyProvider::ENV_VAR);
    }
}

fn set_env(value: &str) {
    // SAFETY: see clear_env() — env_lock() holds the invariant.
    unsafe {
        std::env::set_var(EnvKeyProvider::ENV_VAR, value);
    }
}

#[test]
fn from_env_missing_var_fails_closed() {
    let _g = env_lock();
    clear_env();

    let err = EnvKeyProvider::from_env().expect_err("missing var must error");
    match err {
        ProviderError::NotConfigured { name } => {
            assert_eq!(name, EnvKeyProvider::ENV_VAR);
        },
        other => panic!("wrong variant: {other:?}"),
    }

    clear_env();
}

#[test]
fn from_env_dev_placeholder_rejected() {
    let _g = env_lock();
    clear_env();
    set_env(EnvKeyProvider::DEV_PLACEHOLDER);

    let err = EnvKeyProvider::from_env().expect_err("dev placeholder must error");
    assert!(matches!(err, ProviderError::DevPlaceholder));

    clear_env();
}

#[test]
fn from_env_short_value_rejected() {
    let _g = env_lock();
    clear_env();
    let short = base64::engine::general_purpose::STANDARD.encode([0x42u8; 16]);
    set_env(&short);

    let err = EnvKeyProvider::from_env().expect_err("short key must error");
    assert!(matches!(err, ProviderError::KeyMaterialRejected { .. }));

    clear_env();
}

#[test]
fn from_env_valid_key_round_trips() {
    let _g = env_lock();
    clear_env();
    let valid = base64::engine::general_purpose::STANDARD.encode([0x11u8; 32]);
    set_env(&valid);

    let provider = EnvKeyProvider::from_env().expect("valid key must succeed");
    let version = nebula_credential::KeyProvider::version(&provider);
    // Version is "env:<sha256 prefix fingerprint>" (16 hex chars).
    assert!(
        version.starts_with("env:"),
        "version has env prefix; got {version}"
    );
    assert_eq!(
        version.len(),
        "env:".len() + 16,
        "version has 16-char fingerprint tail; got {version}"
    );

    clear_env();
}
