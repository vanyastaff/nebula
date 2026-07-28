//! Source contract for the admitted PostgreSQL migration operator.

#[cfg(feature = "postgres")]
use std::process::Command;
use std::{fs, path::Path};

fn task_block(taskfile: &str, task_name: &str) -> String {
    let header = format!("  {task_name}:");
    let mut found = false;
    let mut block = Vec::new();

    for line in taskfile.lines() {
        if line == header {
            found = true;
            block.push(line);
            continue;
        }
        if found && line.starts_with("  ") && !line.starts_with("    ") && line.ends_with(':') {
            break;
        }
        if found {
            block.push(line);
        }
    }

    assert!(found, "Taskfile must define `{task_name}`");
    block.join("\n")
}

fn binary_manifest_block(manifest: &str, binary_name: &str) -> String {
    let name_line = format!("name = \"{binary_name}\"");
    let name_offset = manifest
        .find(&name_line)
        .unwrap_or_else(|| panic!("server manifest must define `{binary_name}`"));
    let block_start = manifest[..name_offset]
        .rfind("[[bin]]")
        .expect("binary declaration must start with [[bin]]");
    let remainder = &manifest[block_start..];
    let block_end = remainder[7..]
        .find("\n[[")
        .map_or(remainder.len(), |offset| offset + 7);
    remainder[..block_end].to_owned()
}

#[test]
fn db_migrate_uses_only_the_curated_admission_operator() {
    let server_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = server_root
        .parent()
        .and_then(Path::parent)
        .expect("server package must be nested under the workspace");
    let taskfile =
        fs::read_to_string(workspace_root.join("Taskfile.yml")).expect("Taskfile must be readable");
    let server_manifest =
        fs::read_to_string(server_root.join("Cargo.toml")).expect("manifest must be readable");

    let migrate = task_block(&taskfile, "db:migrate");
    assert!(
        migrate.contains("cargo run -p nebula-server --bin nebula-db-migrate --features postgres"),
        "db:migrate must invoke the curated server-owned migration operator"
    );
    assert!(
        !migrate.contains("sqlx migrate run"),
        "db:migrate must not bypass catalog and aggregate admission"
    );
    assert!(
        !taskfile.contains("\n  db:migrate:revert:"),
        "immutable migration history must not advertise a revert task"
    );

    // Setup fails closed on a database provisioned before ordered migrations,
    // so the one supported way forward must stay advertised and must stay an
    // explicit operator assertion rather than an implicit default.
    let adopt = task_block(&taskfile, "db:migrate:adopt");
    assert!(
        adopt.contains(
            "cargo run -p nebula-server --bin nebula-db-migrate --features postgres -- adopt"
        ),
        "adoption must go through the curated operator, not raw sqlx"
    );
    assert!(
        adopt.contains("THROUGH_VERSION"),
        "adoption must require the operator to state the baseline explicitly"
    );

    let reset = task_block(&taskfile, "db:reset");
    assert!(
        reset.contains("prompt:") && reset.contains("sqlx database drop -y"),
        "raw reset is allowed only behind an explicit destructive prompt and drop"
    );
    assert!(
        reset.find("sqlx database drop -y") < reset.find("sqlx database create")
            && reset.find("sqlx database create") < reset.find("sqlx migrate run"),
        "raw migration may run only after reset has dropped and recreated the database"
    );
    assert_eq!(
        taskfile.matches("sqlx migrate run").count(),
        1,
        "raw migration must exist only in the destructive fresh-database reset"
    );

    let binary = binary_manifest_block(&server_manifest, "nebula-db-migrate");
    assert!(binary.contains("path = \"src/bin/db_migrate.rs\""));
    assert!(binary.contains("required-features = [\"postgres\"]"));
}

#[cfg(feature = "postgres")]
#[test]
fn missing_database_url_uses_bounded_display_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_nebula-db-migrate"))
        .env_remove("DATABASE_URL")
        .output()
        .expect("migration operator process must start");

    assert!(
        !output.status.success(),
        "missing DATABASE_URL must produce a non-zero exit status"
    );

    let stderr = String::from_utf8(output.stderr).expect("operator stderr must be UTF-8");
    assert!(
        stderr.contains("DATABASE_URL is not configured"),
        "stderr must render the human Display diagnostic: {stderr}"
    );
    for forbidden in ["MigrationOperatorError", "MissingDatabaseUrl"] {
        assert!(
            !stderr.contains(forbidden),
            "stderr leaked debug or private text `{forbidden}`: {stderr}"
        );
    }
}

#[cfg(feature = "postgres")]
#[test]
fn invalid_database_url_is_redacted_at_the_process_boundary() {
    const PRIVATE_DATABASE_URL: &str = "operator-private-process-marker";

    let output = Command::new(env!("CARGO_BIN_EXE_nebula-db-migrate"))
        .env("DATABASE_URL", PRIVATE_DATABASE_URL)
        .output()
        .expect("migration operator process must start");

    assert!(
        !output.status.success(),
        "an invalid DATABASE_URL must produce a non-zero exit status"
    );

    let stderr = String::from_utf8(output.stderr).expect("operator stderr must be UTF-8");
    assert!(
        stderr.contains("database is unavailable"),
        "stderr must render the bounded availability diagnostic: {stderr}"
    );
    for forbidden in [
        PRIVATE_DATABASE_URL,
        "DatabaseUnavailable",
        "MigrationOperatorError",
        "caused by:",
    ] {
        assert!(
            !stderr.contains(forbidden),
            "stderr leaked a DSN, Debug spelling, or source detail `{forbidden}`: {stderr}"
        );
    }
}
