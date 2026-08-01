//! Source-level guard for the single ordered-migration schema authority.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

fn files_below(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("source directory must be readable") {
            let entry = entry.expect("source entry must be readable");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn ordered_migrations_are_the_only_embedded_schema_source() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let source_files = files_below(&source_root);

    let schema_snapshots = source_files
        .iter()
        .filter(|path| path.extension() == Some(OsStr::new("sql")))
        .collect::<Vec<_>>();
    assert!(
        schema_snapshots.is_empty(),
        "src must not contain independent SQL schema snapshots: {schema_snapshots:?}"
    );

    let rust_source = source_files
        .iter()
        .filter(|path| path.extension() == Some(OsStr::new("rs")))
        .map(|path| fs::read_to_string(path).expect("Rust source must be UTF-8"))
        .collect::<String>();
    assert!(
        !rust_source.contains("SCHEMA_SQL"),
        "the removed snapshot constant must not be recreated"
    );
    assert!(
        !rust_source.contains("include_str!(\"schema.sql\")"),
        "setup must not embed a schema snapshot"
    );
    // The SQL catalog adapters read and write the 0041 tables, so naming those
    // tables in Rust source is now expected. What must stay true is that
    // migration 0041 remains their only schema authority: an adapter that
    // issued its own DDL would be a second, divergent definition of the same
    // tables, reachable without the ordered catalog ever running.
    let catalog_adapters = [
        source_root.join("sqlite/plan_flavor_catalog.rs"),
        source_root.join("postgres/plan_flavor_catalog.rs"),
        source_root.join("revision_catalog.rs"),
    ];
    for adapter in &catalog_adapters {
        let source = fs::read_to_string(adapter).expect("catalog adapter source must be UTF-8");
        for forbidden_ddl in ["CREATE TABLE", "CREATE INDEX", "ALTER TABLE", "DROP TABLE"] {
            assert!(
                !source.contains(forbidden_ddl),
                "migration 0041 is the only schema authority; found `{forbidden_ddl}` in \
                 {adapter:?}"
            );
        }
    }

    // Execution-owned revision references are still read-only from production
    // code: creating or transitioning one has to compose with the execution
    // aggregate's own transaction, which does not exist yet. Until it does, no
    // production API may write `port_execution_revision_refs` — otherwise a
    // reference could outlive, or never reach, the execution that owns it.
    for reference_mutation in [
        "INSERT INTO port_execution_revision_refs",
        "UPDATE port_execution_revision_refs",
        "DELETE FROM port_execution_revision_refs",
        "activate_execution_revision",
        "persist_execution_revision_ref",
    ] {
        assert!(
            !rust_source.contains(reference_mutation),
            "revision-reference mutation belongs to the execution-owner transaction; found \
             `{reference_mutation}` in production Rust source"
        );
    }

    let migration_module_root = source_root.join("migration");
    let migration_owner = fs::read_to_string(migration_module_root.join("mod.rs"))
        .expect("migration owner must exist");
    assert!(migration_owner.contains("sqlx::migrate!(\"./migrations/sqlite\")"));
    assert!(migration_owner.contains("sqlx::migrate!(\"./migrations/postgres\")"));
    let migration_sources = files_below(&migration_module_root)
        .into_iter()
        .map(|path| fs::read_to_string(path).expect("migration source must be UTF-8"))
        .collect::<String>();
    for forbidden_dependency in [
        "crate::credential",
        "CredentialStoreStartupError",
        "CredentialSchemaAdmissionReason",
        "schema::sqlite::admit",
        "schema::postgres::admit",
        "AdmissionScope::Credential",
    ] {
        assert!(
            !migration_sources.contains(forbidden_dependency),
            "migration coordination must not depend on aggregate-specific admission: \
             `{forbidden_dependency}`"
        );
    }

    for aggregate_probe in [
        source_root.join("credential/schema/sqlite.rs"),
        source_root.join("credential/schema/postgres.rs"),
    ] {
        let source =
            fs::read_to_string(&aggregate_probe).expect("aggregate probe source must be UTF-8");
        assert!(
            !source.contains("_sqlx_migrations"),
            "physical migration-ledger observation has one owner; found a second observer in \
             {aggregate_probe:?}"
        );
    }

    let build_script =
        fs::read_to_string(crate_root.join("build.rs")).expect("storage build script must exist");
    assert!(
        build_script.contains("cargo::rerun-if-changed=migrations"),
        "the embedded catalog must rebuild when migration files change"
    );
}
