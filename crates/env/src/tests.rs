//! Unit tests for the typed env reader. `EnvGuard` serializes mutation and
//! restores prior values, so these run safely under nextest parallelism.

use crate::testing::EnvGuard;
use crate::{EnvError, flag, list, parse, parse_or, var, var_opt};

#[test]
fn var_reports_missing_and_optional() {
    let mut guard = EnvGuard::acquire();
    guard.remove("NEBULA_ENV_TEST_X");
    assert!(matches!(
        var("NEBULA_ENV_TEST_X"),
        Err(EnvError::Missing { .. })
    ));
    assert_eq!(var_opt("NEBULA_ENV_TEST_X"), Ok(None));
}

#[test]
fn parse_roundtrips_and_defaults() {
    let mut guard = EnvGuard::acquire();
    guard.set("NEBULA_ENV_TEST_N", "42");
    assert_eq!(parse::<u64>("NEBULA_ENV_TEST_N"), Ok(Some(42)));
    guard.remove("NEBULA_ENV_TEST_N");
    assert_eq!(parse_or::<u64>("NEBULA_ENV_TEST_N", 7), Ok(7));
}

#[test]
fn parse_rejects_garbage() {
    let mut guard = EnvGuard::acquire();
    guard.set("NEBULA_ENV_TEST_N", "not-a-number");
    assert!(matches!(
        parse::<u64>("NEBULA_ENV_TEST_N"),
        Err(EnvError::Parse { .. })
    ));
}

#[test]
fn flag_accepts_aliases_and_rejects_others() {
    let mut guard = EnvGuard::acquire();
    for value in ["true", "1", "YES", "on"] {
        guard.set("NEBULA_ENV_TEST_B", value);
        assert_eq!(flag("NEBULA_ENV_TEST_B"), Ok(Some(true)));
    }
    for value in ["false", "0", "no", "OFF"] {
        guard.set("NEBULA_ENV_TEST_B", value);
        assert_eq!(flag("NEBULA_ENV_TEST_B"), Ok(Some(false)));
    }
    guard.set("NEBULA_ENV_TEST_B", "maybe");
    assert!(matches!(
        flag("NEBULA_ENV_TEST_B"),
        Err(EnvError::Invalid { .. })
    ));
}

#[test]
fn list_splits_on_commas_and_whitespace() {
    let mut guard = EnvGuard::acquire();
    guard.set("NEBULA_ENV_TEST_L", "a, b  c,,d");
    assert_eq!(
        list("NEBULA_ENV_TEST_L"),
        ["a", "b", "c", "d"].map(str::to_owned)
    );
}
