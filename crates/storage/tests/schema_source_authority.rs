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
    for forbidden_sql_catalog_surface in [
        "SqlitePlanFlavorCatalog",
        "PgPlanFlavorCatalog",
        "PostgresPlanFlavorCatalog",
        "port_worker_flavor_revisions",
        "port_executable_plan_revisions",
        "port_execution_revision_refs",
        "activate_execution_revision",
        "persist_execution_revision_ref",
    ] {
        assert!(
            !rust_source.contains(forbidden_sql_catalog_surface),
            "0041 is dormant DDL; found SQL catalog adapter or activation surface \
             `{forbidden_sql_catalog_surface}` in production Rust source"
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
