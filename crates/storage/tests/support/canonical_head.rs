//! The canonical migration head, derived from the embedded catalog.
//!
//! Several suites assert "setup migrated this database all the way to head".
//! Spelling the head as a literal made every one of them a hidden dependency
//! on the current migration count: adding `0042` broke six unrelated suites
//! whose subject is pool visibility, adoption, or lock release — none of which
//! care which migration is last.
//!
//! Deriving it keeps the assertion real (a database that stops short of head
//! still fails) while leaving version literals to the assertions that are
//! genuinely *about* one migration's description, checksum, or rows.
//!
//! Included by `#[path]`; integration tests compile as their own crates and
//! cannot see `nebula_storage`'s `#[cfg(test)]` helpers.

/// Highest version in `migrator`'s embedded catalog.
pub(crate) fn of(migrator: &sqlx::migrate::Migrator) -> i64 {
    migrator
        .iter()
        .map(|migration| migration.version)
        .max()
        .expect("the embedded migration catalog is never empty")
}
