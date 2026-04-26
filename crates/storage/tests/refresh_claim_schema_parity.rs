//! Schema parity check for the refresh-claim migrations (0022 + 0023).
//!
//! Both SQLite and Postgres dialects must define the same tables with the
//! same logical column names. The driver-specific types differ
//! (TEXT/INTEGER vs TIMESTAMPTZ/UUID/BIGSERIAL) — we match on the column
//! identifiers, not their declared types.
//!
//! Per ADR-0041 + sub-spec §3.3 / §3.4.

use std::{collections::BTreeSet, path::Path};

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Parse a CREATE TABLE block and return the set of column identifiers
/// declared in it. Indices and constraints (CHECK, PRIMARY KEY clauses
/// without column intro) are skipped.
fn columns_of_table(sql: &str, table: &str) -> BTreeSet<String> {
    let needle = format!("CREATE TABLE {table} (");
    let start = sql
        .find(&needle)
        .unwrap_or_else(|| panic!("table {table} not found in:\n{sql}"));
    let body_start = start + needle.len();
    let body = &sql[body_start..];
    let end = body.find(");").expect("missing ); for table");
    let body = &body[..end];

    // Strip line comments (-- ...) before splitting on commas — comments
    // commonly contain commas that would otherwise confuse the splitter.
    let body_no_comments: String = body
        .lines()
        .map(|l| match l.find("--") {
            Some(idx) => &l[..idx],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut cols = BTreeSet::new();
    for raw in body_no_comments.split(',') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Skip standalone constraints — they don't start with an identifier
        // we treat as a column name.
        let upper = line.to_uppercase();
        if upper.starts_with("CHECK")
            || upper.starts_with("PRIMARY KEY (")
            || upper.starts_with("UNIQUE (")
            || upper.starts_with("FOREIGN KEY")
            || upper.starts_with("CONSTRAINT")
        {
            continue;
        }
        // First whitespace-separated token is the column name.
        let Some(col) = line.split_whitespace().next() else {
            continue;
        };
        let col = col.trim_matches(|c: char| c == '"').to_string();
        cols.insert(col);
    }
    cols
}

#[test]
fn refresh_claim_table_columns_match() {
    let sqlite = read("migrations/sqlite/0022_credential_refresh_claims.sql");
    let pg = read("migrations/postgres/0022_credential_refresh_claims.sql");

    let s = columns_of_table(&sqlite, "credential_refresh_claims");
    let p = columns_of_table(&pg, "credential_refresh_claims");

    assert_eq!(
        s, p,
        "credential_refresh_claims columns differ between SQLite and Postgres"
    );
    assert!(s.contains("credential_id"));
    assert!(s.contains("claim_id"));
    assert!(s.contains("generation"));
    assert!(s.contains("holder_replica_id"));
    assert!(s.contains("acquired_at"));
    assert!(s.contains("expires_at"));
    assert!(s.contains("sentinel"));
}

#[test]
fn sentinel_events_table_columns_match() {
    let sqlite = read("migrations/sqlite/0023_credential_sentinel_events.sql");
    let pg = read("migrations/postgres/0023_credential_sentinel_events.sql");

    let s = columns_of_table(&sqlite, "credential_sentinel_events");
    let p = columns_of_table(&pg, "credential_sentinel_events");

    assert_eq!(
        s, p,
        "credential_sentinel_events columns differ between SQLite and Postgres"
    );
    assert!(s.contains("id"));
    assert!(s.contains("credential_id"));
    assert!(s.contains("detected_at"));
    assert!(s.contains("crashed_holder"));
    assert!(s.contains("generation"));
}

#[test]
fn both_dialects_define_the_same_indices() {
    let sqlite = format!(
        "{}{}",
        read("migrations/sqlite/0022_credential_refresh_claims.sql"),
        read("migrations/sqlite/0023_credential_sentinel_events.sql")
    );
    let pg = format!(
        "{}{}",
        read("migrations/postgres/0022_credential_refresh_claims.sql"),
        read("migrations/postgres/0023_credential_sentinel_events.sql")
    );

    for index_name in [
        "idx_refresh_claims_expires",
        "idx_sentinel_events_cred_time",
    ] {
        assert!(
            sqlite.contains(index_name),
            "SQLite missing index `{index_name}`"
        );
        assert!(
            pg.contains(index_name),
            "Postgres missing index `{index_name}`"
        );
    }
}
