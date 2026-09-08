//! Policy over raw ordered migration and reconnect snapshots.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256, Sha384};
use std::{collections::BTreeSet, fs, path::Path};
use thiserror::Error;

#[derive(Debug)]
struct MigrationCatalog {
    row_count: usize,
    head: i64,
    digest: String,
    #[cfg(test)]
    rows: Value,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum OrderedMigrationError {
    #[error("ordered migration observation shape is invalid")]
    Shape,
    #[error("ordered migration producer, inventory, or migration version is unsupported")]
    Version,
    #[error("ordered migration backend or scenario inventory is invalid")]
    Inventory,
    #[error("ordered migration applied migration ledger differs from the reviewed catalog")]
    Catalog,
    #[error("ordered migration applied migration ledger has an unexpected row count")]
    CatalogCount,
    #[error("ordered migration applied migration ledger row is malformed")]
    CatalogRow,
    #[error("ordered migration applied migration ledger digest differs from reviewed SQL")]
    CatalogDigest,
    #[error(
        "ordered migration migration snapshots do not survive close, reopen, and reinitialization"
    )]
    Reconnect,
    #[error("ordered migration previous-supported data was not preserved exactly")]
    Sentinel,
}

pub(crate) fn verify(workspace: &Path, value: &Value) -> Result<(), OrderedMigrationError> {
    let root = object(
        value,
        &[
            "producer_version",
            "contract",
            "scenario_inventory_version",
            "backend",
            "database_version",
            "previous_supported_version",
            "current_head",
            "scenarios",
        ],
    )?;
    if root["producer_version"].as_u64() != Some(2)
        || root["scenario_inventory_version"].as_u64() != Some(1)
        || root["contract"].as_str() != Some("ordered-migrations")
        || root["previous_supported_version"].as_i64() != Some(45)
    {
        return Err(OrderedMigrationError::Version);
    }
    let expected_catalog = match root["backend"].as_str() {
        Some("sqlite") => migration_catalog(workspace, "sqlite")?,
        Some("postgresql") => migration_catalog(workspace, "postgres")?,
        _ => return Err(OrderedMigrationError::Inventory),
    };
    if root["current_head"].as_i64() != Some(expected_catalog.head) {
        return Err(OrderedMigrationError::Version);
    }
    let database_version = root["database_version"]
        .as_str()
        .ok_or(OrderedMigrationError::Shape)?;
    if database_version.trim().is_empty()
        || database_version.len() > 128
        || database_version.chars().any(char::is_control)
    {
        return Err(OrderedMigrationError::Shape);
    }
    let scenarios = root["scenarios"]
        .as_array()
        .ok_or(OrderedMigrationError::Inventory)?;
    if scenarios.len() != 2 {
        return Err(OrderedMigrationError::Inventory);
    }
    let mut seen = BTreeSet::new();
    for scenario in scenarios {
        let fields = object(scenario, &["scenario", "events"])?;
        let name = fields["scenario"]
            .as_str()
            .ok_or(OrderedMigrationError::Inventory)?;
        if !matches!(name, "clean" | "previous-supported-version") || !seen.insert(name) {
            return Err(OrderedMigrationError::Inventory);
        }
        let events = fields["events"]
            .as_array()
            .ok_or(OrderedMigrationError::Shape)?;
        if events.len() != 4 {
            return Err(OrderedMigrationError::Reconnect);
        }
        let closed = object(&events[1], &["sequence", "kind"])?;
        if closed["sequence"].as_u64() != Some(1) || closed["kind"].as_str() != Some("pool_closed")
        {
            return Err(OrderedMigrationError::Reconnect);
        }
        let mut baseline: Option<(&Value, &Value)> = None;
        for (sequence, stage) in [(0, "migrated"), (2, "reopened"), (3, "reinitialized")] {
            let event = object(
                &events[sequence],
                &["sequence", "kind", "stage", "migrations", "sentinel"],
            )?;
            if event["sequence"].as_u64() != Some(sequence as u64)
                || event["kind"].as_str() != Some("migration_snapshot")
                || event["stage"].as_str() != Some(stage)
            {
                return Err(OrderedMigrationError::Reconnect);
            }
            verify_catalog(&event["migrations"], &expected_catalog)?;
            verify_sentinel(&event["sentinel"], name)?;
            if let Some((migrations, sentinel)) = baseline {
                if migrations != &event["migrations"] || sentinel != &event["sentinel"] {
                    return Err(OrderedMigrationError::Reconnect);
                }
            } else {
                baseline = Some((&event["migrations"], &event["sentinel"]));
            }
        }
    }
    Ok(())
}

fn verify_catalog(value: &Value, expected: &MigrationCatalog) -> Result<(), OrderedMigrationError> {
    let rows = value.as_array().ok_or(OrderedMigrationError::Catalog)?;
    if rows.len() != expected.row_count {
        return Err(OrderedMigrationError::CatalogCount);
    }
    let mut hash = Sha256::new();
    let mut previous = 0_i64;
    for row in rows {
        let fields = object(row, &["version", "description", "checksum", "success"])?;
        let version = fields["version"]
            .as_i64()
            .ok_or(OrderedMigrationError::Catalog)?;
        let description = fields["description"]
            .as_str()
            .ok_or(OrderedMigrationError::Catalog)?;
        let checksum = fields["checksum"]
            .as_str()
            .ok_or(OrderedMigrationError::Catalog)?;
        if version <= previous
            || description.is_empty()
            || checksum.len() != 96
            || !checksum
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || fields["success"].as_bool() != Some(true)
        {
            return Err(OrderedMigrationError::CatalogRow);
        }
        previous = version;
        hash.update(format!("{version}\0{description}\0{checksum}\0true\n"));
    }
    let actual = super::super::loader::hex(hash.finalize());
    if previous != expected.head || actual != expected.digest {
        return Err(OrderedMigrationError::CatalogDigest);
    }
    Ok(())
}

fn migration_catalog(
    workspace: &Path,
    backend_directory: &str,
) -> Result<MigrationCatalog, OrderedMigrationError> {
    let directory = workspace
        .join("crates/storage/migrations")
        .join(backend_directory);
    let mut migrations = Vec::new();
    for entry in fs::read_dir(directory).map_err(|_| OrderedMigrationError::Catalog)? {
        let entry = entry.map_err(|_| OrderedMigrationError::Catalog)?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| OrderedMigrationError::CatalogRow)?;
        let Some((version, remainder)) = file_name.split_once('_') else {
            continue;
        };
        let Some(description) = remainder.strip_suffix(".sql") else {
            continue;
        };
        let version = version
            .parse::<i64>()
            .map_err(|_| OrderedMigrationError::CatalogRow)?;
        let sql = fs::read_to_string(entry.path()).map_err(|_| OrderedMigrationError::Catalog)?;
        migrations.push((
            version,
            description.replace('_', " "),
            super::super::loader::hex(Sha384::digest(sql)),
        ));
    }
    migrations.sort_by_key(|(version, _, _)| *version);
    let mut digest = Sha256::new();
    for (version, description, checksum) in &migrations {
        digest.update(format!("{version}\0{description}\0{checksum}\0true\n"));
    }
    let head = migrations
        .last()
        .map(|(version, _, _)| *version)
        .ok_or(OrderedMigrationError::Catalog)?;
    #[cfg(test)]
    let rows = Value::Array(
        migrations
            .iter()
            .map(|(version, description, checksum)| {
                serde_json::json!({"version":version,"description":description,"checksum":checksum,"success":true})
            })
            .collect(),
    );
    Ok(MigrationCatalog {
        row_count: migrations.len(),
        head,
        digest: super::super::loader::hex(digest.finalize()),
        #[cfg(test)]
        rows,
    })
}

fn verify_sentinel(value: &Value, scenario: &str) -> Result<(), OrderedMigrationError> {
    let fields = object(value, &["execution", "journal"])?;
    if scenario == "clean" {
        return if fields["execution"].is_null()
            && fields["journal"].as_array().is_some_and(Vec::is_empty)
        {
            Ok(())
        } else {
            Err(OrderedMigrationError::Sentinel)
        };
    }
    let expected = serde_json::json!({"execution":{"fencing_generation":11,"id":"migration-sentinel","org_id":"org","state":{"sentinel":true},"status":"Running","version":7,"workflow_id":"workflow","workspace_id":"workspace"},"journal":[{"payload":{"event":"preserved"},"seq":3}]});
    if value == &expected {
        Ok(())
    } else {
        Err(OrderedMigrationError::Sentinel)
    }
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, OrderedMigrationError> {
    let fields = value.as_object().ok_or(OrderedMigrationError::Shape)?;
    if fields.len() != keys.len() || !keys.iter().all(|key| fields.contains_key(*key)) {
        return Err(OrderedMigrationError::Shape);
    }
    Ok(fields)
}

#[cfg(test)]
pub(super) fn fixture(workspace: &Path, backend: &str) -> Value {
    let catalog_directory = if backend == "postgresql" {
        "postgres"
    } else {
        backend
    };
    let catalog = migration_catalog(workspace, catalog_directory)
        .expect("the checked-in migration catalog is readable");
    let clean_sentinel = serde_json::json!({"execution":null,"journal":[]});
    let retained_sentinel = serde_json::json!({"execution":{"fencing_generation":11,"id":"migration-sentinel",
        "org_id":"org","state":{"sentinel":true},"status":"Running","version":7,
        "workflow_id":"workflow","workspace_id":"workspace"},
        "journal":[{"payload":{"event":"preserved"},"seq":3}]});
    let scenario = |name: &str, sentinel: &Value| {
        serde_json::json!({"scenario":name,"events":[
            {"sequence":0,"kind":"migration_snapshot","stage":"migrated","migrations":catalog.rows,"sentinel":sentinel},
            {"sequence":1,"kind":"pool_closed"},
            {"sequence":2,"kind":"migration_snapshot","stage":"reopened","migrations":catalog.rows,"sentinel":sentinel},
            {"sequence":3,"kind":"migration_snapshot","stage":"reinitialized","migrations":catalog.rows,"sentinel":sentinel}
        ]})
    };
    serde_json::json!({"producer_version":2,"contract":"ordered-migrations","scenario_inventory_version":1,
        "backend":backend,"database_version":format!("synthetic-{backend}-version"),
        "previous_supported_version":45,"current_head":catalog.head,
        "scenarios":[scenario("clean",&clean_sentinel),
            scenario("previous-supported-version",&retained_sentinel)]})
}

#[cfg(test)]
mod tests {
    fn workspace() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn incomplete_observation_cannot_qualify() {
        assert_eq!(
            super::verify(std::path::Path::new("."), &serde_json::json!({})),
            Err(super::OrderedMigrationError::Shape)
        );
    }

    #[test]
    fn checked_in_catalog_qualifies_and_a_changed_checksum_is_rejected() {
        let workspace = workspace();
        let value = super::fixture(&workspace, "sqlite");
        assert_eq!(super::verify(&workspace, &value), Ok(()));
        let mut invalid = value;
        invalid["scenarios"][0]["events"][0]["migrations"][0]["checksum"] = "0".repeat(96).into();
        assert_eq!(
            super::verify(&workspace, &invalid),
            Err(super::OrderedMigrationError::CatalogDigest)
        );
    }
}
