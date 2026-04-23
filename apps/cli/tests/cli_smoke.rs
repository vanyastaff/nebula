//! CLI integration checks: binary exists, help exits zero, output mentions the product.
//! Uses [`assert_cmd`] + [`predicates`]; see `docs/TESTING.md`.

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn nebula_help_succeeds() {
    Command::cargo_bin("nebula")
        .expect("cargo_bin finds nebula from this package")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Nebula").or(predicate::str::contains("nebula")));
}
