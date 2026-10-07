//! Facts about the ordered migration catalogs, shared by setup, tests and
//! operator tooling — the one place a new migration is acknowledged.
//!
//! Adding a migration: add `migrations/postgres/NNNN_<slug>.sql` and the same
//! name under `migrations/sqlite/`; backend-specific objects live inside that
//! shared version rather than reserving gaps. Record its
//! admission classification in the review log on the head test in
//! `src/migration/mod.rs`; then raise [`REVIEWED_HEAD`].

/// The newest migration whose setup admission has been reviewed. Setup tests
/// fail until a new migration is acknowledged here.
pub const REVIEWED_HEAD: i64 = 8;
