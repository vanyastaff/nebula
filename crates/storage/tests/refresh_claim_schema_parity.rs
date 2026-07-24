//! Schema parity check for refresh-claim migrations 0022, 0023, and the
//! incident-identity extension in 0039, plus the structural retry gate in
//! 0040.
//!
//! Both SQLite and Postgres dialects must define the same tables with the
//! same logical column names. The driver-specific types differ
//! (TEXT/INTEGER vs TIMESTAMPTZ/UUID/BIGSERIAL) — we match on the column
//! identifiers, not their declared types.
//!
//! Per refresh-claim contract (SQLite vs Postgres column parity).

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
        // we treat as a column name. Tolerate either `KW(` or `KW (` so a
        // dialect that omits the space (e.g. `UNIQUE(col)`) is still
        // detected as a constraint and not mis-parsed as a column.
        let upper = line.to_uppercase();
        let is_constraint = [
            "CHECK",
            "UNIQUE",
            "PRIMARY KEY",
            "FOREIGN KEY",
            "CONSTRAINT",
        ]
        .iter()
        .any(|kw| {
            upper
                .strip_prefix(kw)
                .is_some_and(|rest| rest.starts_with('(') || rest.starts_with(' '))
        });
        if is_constraint {
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
fn sentinel_incident_identity_is_paired_and_globally_unique() {
    let sqlite = read("migrations/sqlite/0039_credentials_owner_and_record_state.sql");
    let pg = read("migrations/postgres/0039_credentials_owner_and_record_state.sql");

    for (backend, migration, data_type) in [
        ("SQLite", sqlite.as_str(), "ADD COLUMN claim_id TEXT"),
        ("Postgres", pg.as_str(), "ADD COLUMN claim_id UUID"),
    ] {
        assert!(
            migration.contains("ALTER TABLE credential_sentinel_events"),
            "{backend} 0039 must extend the sentinel event relation"
        );
        assert!(
            migration.contains(data_type),
            "{backend} 0039 must use its canonical claim-id type"
        );
        assert!(
            migration.contains("CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id"),
            "{backend} 0039 must enforce global incident identity"
        );
        assert!(
            migration.contains("ON credential_sentinel_events(claim_id)")
                && migration.contains("WHERE claim_id IS NOT NULL"),
            "{backend} incident identity must be a partial single-column unique index"
        );
    }
}

#[test]
fn credential_refresh_retry_gate_is_paired_and_closed() {
    let sqlite = read("migrations/sqlite/0040_credential_refresh_retry_gate.sql");
    let pg = read("migrations/postgres/0040_credential_refresh_retry_gate.sql");

    for column in [
        "material_epoch",
        "refresh_retry_mode",
        "refresh_retry_not_before",
        "refresh_retry_phase",
        "refresh_retry_kind",
        "refresh_retry_diagnostic_code",
    ] {
        assert!(sqlite.contains(column), "SQLite 0040 misses `{column}`");
        assert!(pg.contains(column), "Postgres 0040 misses `{column}`");
    }
    for closed_code in [
        "never",
        "not_before",
        "before_dispatch",
        "provider_confirmed_not_applied",
        "transient_network",
        "provider_unavailable",
        "protocol_error",
    ] {
        assert!(
            sqlite.contains(&format!("'{closed_code}'")),
            "SQLite 0040 misses closed code `{closed_code}`"
        );
        assert!(
            pg.contains(&format!("'{closed_code}'")),
            "Postgres 0040 misses closed code `{closed_code}`"
        );
    }
    for migration in [&sqlite, &pg] {
        assert!(migration.contains("credentials_refresh_retry_gate_shape"));
        assert!(migration.contains("credentials_material_epoch_range"));
        assert!(migration.contains("credentials_record_shape"));
        assert!(
            !migration.contains("$.refresh_retry"),
            "retry gate must not be encoded in user metadata"
        );
    }

    assert!(
        sqlite.contains("material_epoch                INTEGER NOT NULL")
            && sqlite.contains("version,\n    material_epoch,")
            && sqlite.contains("version,\n    1,"),
        "SQLite 0040 must rebuild every legacy row at material epoch 1"
    );
    assert!(
        pg.contains("ADD COLUMN material_epoch BIGINT NOT NULL DEFAULT 1")
            && pg.contains("ALTER COLUMN material_epoch DROP DEFAULT")
            && pg.contains("CHECK (material_epoch BETWEEN 1 AND 9223372036854775807)"),
        "Postgres 0040 must backfill epoch 1, remove the write-time default, and close the range"
    );
    assert!(
        sqlite.contains("refresh_retry_not_before      INTEGER")
            && pg.contains("refresh_retry_not_before TIMESTAMPTZ"),
        "retry deadlines must use each backend's canonical clock representation"
    );
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
