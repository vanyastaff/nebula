//! Every named constraint and index fits PostgreSQL's 63-byte identifier limit.
//!
//! PostgreSQL silently truncates a longer identifier, which drops the
//! predicate suffix and lets the two backends' catalogs disagree; the
//! database standard's shortening steps keep every name within the limit.

use std::{fs, path::Path};

const PREFIXES: [&str; 5] = ["pk_", "uq_", "fk_", "ck_", "ix_"];
const MAX_IDENTIFIER_BYTES: usize = 63;

/// The standard-named identifiers declared in one migration source.
fn standard_names(sql: &str) -> Vec<&str> {
    sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| PREFIXES.iter().any(|prefix| word.starts_with(prefix)))
        .collect()
}

#[test]
fn schema_object_names_fit_the_identifier_limit() {
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut too_long = Vec::new();
    for backend in ["postgres", "sqlite"] {
        let directory = migrations.join(backend);
        for entry in fs::read_dir(&directory).expect("migration directory must be readable") {
            let path = entry.expect("migration entry must be readable").path();
            let sql = fs::read_to_string(&path).expect("migration must be UTF-8");
            too_long.extend(
                standard_names(&sql)
                    .into_iter()
                    .filter(|name| name.len() > MAX_IDENTIFIER_BYTES)
                    .map(|name| format!("{}: {name}", path.display())),
            );
        }
    }
    assert!(
        too_long.is_empty(),
        "names over {MAX_IDENTIFIER_BYTES} bytes: {too_long:#?}"
    );
}
