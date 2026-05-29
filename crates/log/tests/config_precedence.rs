use nebula_env::testing::EnvGuard;
use nebula_log::{Config, LogError, LoggerBuilder};

#[test]
fn explicit_config_has_highest_precedence() {
    // Hold the guard so this test is serialized against the env-mutating one
    // below, even though it does not touch the environment itself.
    let _env = EnvGuard::acquire();

    let explicit = Config {
        level: "trace".to_string(),
        ..Config::default()
    };
    let resolved = Config::resolve_startup(Some(explicit.clone()));

    assert_eq!(resolved.config.level, explicit.level);
}

#[test]
fn environment_overrides_preset_when_explicit_absent() {
    let mut env = EnvGuard::acquire();
    env.set("NEBULA_LOG", "warn");

    let resolved = Config::resolve_startup(None);
    assert_eq!(resolved.config.level, "warn");

    // `NEBULA_LOG` is restored to its prior value (or unset) when `env` drops.
}

#[test]
fn invalid_filter_returns_error() {
    let config = Config {
        level: "info,[".to_string(),
        ..Config::default()
    };

    let result = LoggerBuilder::from_config(config).build();
    match result {
        Err(LogError::Filter(_)) => {},
        Err(other) => panic!("expected filter error, got: {other}"),
        Ok(_) => panic!("expected filter error, got success"),
    }
}
