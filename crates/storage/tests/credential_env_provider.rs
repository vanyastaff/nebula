//! Integration test for [`EnvKeyProvider::from_env`].
//!
//! Lives outside the crate proper because the crate applies
//! `#![forbid(unsafe_code)]`; the unsafe env-mutation boundary is centralized
//! behind [`nebula_env::testing::EnvGuard`], which serializes mutation under a
//! process-global lock and restores prior values on drop. The in-crate unit
//! tests cover the validation logic via `from_base64`; this binary exercises
//! the env lookup itself and the fail-closed contract against a missing
//! `NEBULA_CRED_MASTER_KEY`.

use base64::Engine;
use nebula_env::testing::EnvGuard;
use nebula_storage::credential::{EnvKeyProvider, ProviderError};

#[test]
fn from_env_missing_var_fails_closed() {
    let mut env = EnvGuard::acquire();
    env.remove(EnvKeyProvider::ENV_VAR);

    let err = EnvKeyProvider::from_env().expect_err("missing var must error");
    match err {
        ProviderError::NotConfigured { name } => {
            assert_eq!(name, EnvKeyProvider::ENV_VAR);
        },
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn from_env_dev_placeholder_rejected() {
    let mut env = EnvGuard::acquire();
    env.set(EnvKeyProvider::ENV_VAR, EnvKeyProvider::DEV_PLACEHOLDER);

    let err = EnvKeyProvider::from_env().expect_err("dev placeholder must error");
    assert!(matches!(err, ProviderError::DevPlaceholder));
}

#[test]
fn from_env_short_value_rejected() {
    let mut env = EnvGuard::acquire();
    let short = base64::engine::general_purpose::STANDARD.encode([0x42u8; 16]);
    env.set(EnvKeyProvider::ENV_VAR, &short);

    let err = EnvKeyProvider::from_env().expect_err("short key must error");
    assert!(matches!(err, ProviderError::KeyMaterialRejected { .. }));
}

#[test]
fn from_env_valid_key_round_trips() {
    let mut env = EnvGuard::acquire();
    let valid = base64::engine::general_purpose::STANDARD.encode([0x11u8; 32]);
    env.set(EnvKeyProvider::ENV_VAR, &valid);

    let provider = EnvKeyProvider::from_env().expect("valid key must succeed");
    let version = nebula_storage::credential::KeyProvider::version(&provider);
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
}
