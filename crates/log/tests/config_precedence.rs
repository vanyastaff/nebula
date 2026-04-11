use std::sync::{LazyLock, Mutex};

use nebula_log::{Config, LogError, LoggerBuilder};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[test]
fn explicit_config_has_highest_precedence() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");

    let explicit = Config {
        level: "trace".to_string(),
        ..Config::default()
    };
    let resolved = Config::resolve_startup(Some(explicit.clone()));

    assert_eq!(resolved.config.level, explicit.level);
}

#[test]
fn environment_overrides_preset_when_explicit_absent() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");

    // SAFETY: tests are serialized by ENV_LOCK for process-wide env changes.
    unsafe { std::env::set_var("NEBULA_LOG", "warn") };

    let resolved = Config::resolve_startup(None);
    assert_eq!(resolved.config.level, "warn");

    // SAFETY: tests are serialized by ENV_LOCK for process-wide env changes.
    unsafe { std::env::remove_var("NEBULA_LOG") };
}

#[test]
fn invalid_filter_returns_error() {
    let config = Config {
        level: "info,[".to_string(),
        ..Config::default()
    };

    let result = LoggerBuilder::from_config(config).build();
    match result {
        Err(LogError::Filter(_)) => {}
        Err(other) => panic!("expected filter error, got: {other}"),
        Ok(_) => panic!("expected filter error, got success"),
    }
}
