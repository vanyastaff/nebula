//! Facts about the ordered migration catalogs, shared by setup, tests and
//! operator tooling — the one place a new migration is acknowledged.
//!
//! Adding a migration: add `migrations/postgres/NNNN_<slug>.sql` and, unless
//! it is PostgreSQL-only, the same name under `migrations/sqlite/`; record its
//! admission classification in the review log on the head test in
//! `src/migration/mod.rs`; then raise [`REVIEWED_HEAD`] (and list a
//! PostgreSQL-only version in [`POSTGRES_ONLY_VERSIONS`]).

/// The newest migration whose setup admission has been reviewed. Setup tests
/// fail until a new migration is acknowledged here.
pub const REVIEWED_HEAD: i64 = 64;

/// Versions present only in the PostgreSQL catalog. The SQLite catalog skips
/// them, and a SQLite ledger that records one is rejected.
pub const POSTGRES_ONLY_VERSIONS: &[i64] = &[29, 36, 37, 38, 60];
