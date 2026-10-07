//! Shared by `ordered_migration_observations` (SQLite) and
//! `ordered_migration_observations_postgres`: the retained-observation writer and
//! the populated-head fixture both backends record against.

use std::io::Write;

pub(crate) use serde_json::{Value, json};

const CURRENT_HEAD: i64 = nebula_storage::migration_catalog::REVIEWED_HEAD;

pub(crate) fn expected_sentinel(populated: bool) -> Value {
    if populated {
        json!({"execution":{"fencing_generation":1,"id":"migration-sentinel",
            "org_id":"org","state":{"sentinel":true},"status":"running","version":1,
            "workflow_id":"workflow","workspace_id":"workspace"},
            "journal":[{"payload":{"event":"preserved"},"seq":1}]})
    } else {
        json!({"execution":null,"journal":[]})
    }
}

pub(crate) fn assert_snapshot(
    snapshot: &Value,
    catalog: &sqlx::migrate::Migrator,
    populated: bool,
) {
    let expected_migrations = Value::Array(
        catalog
            .iter()
            .map(|migration| {
                json!({
                    "version": migration.version, "description": migration.description,
                    "checksum": hex(&migration.checksum), "success": true,
                })
            })
            .collect(),
    );
    assert_eq!(
        snapshot["migrations"], expected_migrations,
        "exact reviewed catalog survives setup"
    );
    assert_eq!(
        snapshot["sentinel"],
        expected_sentinel(populated),
        "execution state and atomic journal survive setup"
    );
}

/// Populate through the same scoped ports as production. The journal entry is
/// committed atomically with the state, rather than inserted through a test-only
/// writer. The retained snapshots below verify these facts after pool replacement.
pub(crate) async fn populate_head(
    tenants: &dyn nebula_storage_port::store::TenantProvisioningStore,
    workflows: &dyn nebula_storage_port::store::WorkflowStore,
    executions: &dyn nebula_storage_port::store::ExecutionStore,
) {
    use nebula_storage_port::dto::{
        ExecutionListing, ExecutionListingStatus, JournalEntry, PrincipalKind,
        TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningOutcome,
        TenantProvisioningRequest, WorkflowRecord,
    };
    use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};

    let scope = Scope::new("workspace", "org");
    let request = TenantProvisioningRequest::new(
        TenantOrgCreate::new(
            "org".into(),
            "org".into(),
            "Fixture".into(),
            "fixture".into(),
            "free".into(),
            None,
            json!({}),
        )
        .unwrap(),
        TenantDefaultWorkspaceCreate::new(
            "workspace".into(),
            "default".into(),
            "Default".into(),
            None,
            "fixture".into(),
            json!({}),
        )
        .unwrap(),
        PrincipalKind::User,
        "fixture-owner".into(),
        None,
    )
    .unwrap();
    assert_eq!(
        tenants.provision_tenant(request).await.unwrap(),
        TenantProvisioningOutcome::Created
    );
    workflows
        .create(
            &scope,
            WorkflowRecord {
                id: "workflow".into(),
                scope: scope.clone(),
                version: 1,
                slug: "workflow".into(),
            },
        )
        .await
        .unwrap();
    executions
        .create(&scope, "migration-sentinel", "workflow", json!({}))
        .await
        .unwrap();
    let fencing = executions
        .acquire_lease(
            &scope,
            "migration-sentinel",
            "migration-observer",
            std::time::Duration::from_mins(1),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        executions
            .commit(
                TransitionBatch::new(
                    scope,
                    "migration-sentinel",
                    0,
                    fencing,
                    json!({"sentinel": true}),
                    ExecutionListing::new(ExecutionListingStatus::Running, None, None),
                )
                .with_journal(vec![JournalEntry {
                    seq: None,
                    payload: json!({"event": "preserved"})
                }])
            )
            .await
            .unwrap(),
        TransitionOutcome::Applied { new_version: 1 }
    );
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        },
    )
}

pub(crate) fn retain(
    variable: &str,
    backend: &str,
    database_version: String,
    scenarios: Vec<Value>,
) {
    if let Some(path) = std::env::var_os(variable) {
        let path = std::path::Path::new(&path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let bytes = serde_json::to_vec_pretty(&json!({
            "producer_version": 3,
            "contract": "ordered-migrations",
            "scenario_inventory_version": 2,
            "backend": backend,
            "database_version": database_version,
            "current_head": CURRENT_HEAD,
            "scenarios": scenarios,
        }))
        .unwrap();
        assert!(bytes.len() <= 512 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
}
