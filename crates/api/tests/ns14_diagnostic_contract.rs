//! NS14 diagnostic-contract evidence.
//!
//! Every activation rejection the workspace can raise must report all five
//! fields. Each producing crate already asserts that for its own variants; this
//! suite is the cross-crate half — it enumerates the rejections from the
//! workflow validator, the plan compiler, and the registry-compatibility check
//! together, and emits one versioned report covering all of them.
//!
//! The report is written to `target/ns14/diagnostic-contract.json` so CI can
//! retain it as evidence rather than re-deriving the claim from a passing exit
//! code. A test that only returned green would prove the suite ran, not what it
//! found.
//!
//! This lives in `nebula-api` because it is the only crate that depends on all
//! three producers *and* on the RFC 9457 boundary the diagnostics travel over,
//! so a gap between what a producer reports and what a client receives fails
//! here rather than in three places that each see one half.

use std::{fs, path::PathBuf};

use nebula_error::{
    ActivationDiagnostics, DIAGNOSTIC_CONTRACT_REPORT_VERSION, DiagnosticContractEntry,
    DiagnosticContractReport,
};

/// Collect every diagnostic one rejection reports into report entries.
fn entries_for(
    rejection_name: &str,
    rejection: &dyn ActivationDiagnostics,
) -> Vec<DiagnosticContractEntry> {
    let diagnostics = rejection.activation_diagnostics();
    assert!(
        !diagnostics.is_empty(),
        "{rejection_name} reported nothing, so a caller has nothing to act on"
    );
    diagnostics
        .iter()
        .map(|diagnostic| DiagnosticContractEntry::observed(rejection_name, diagnostic))
        .collect()
}

/// One instance of every workflow-validation rejection.
fn workflow_rejections() -> Vec<(&'static str, nebula_workflow::WorkflowError)> {
    use nebula_core::node_key;
    use nebula_workflow::WorkflowError;

    let a = node_key!("a");
    let b = node_key!("b");
    vec![
        ("WorkflowError::EmptyName", WorkflowError::EmptyName),
        ("WorkflowError::NoNodes", WorkflowError::NoNodes),
        (
            "WorkflowError::DuplicateNodeKey",
            WorkflowError::DuplicateNodeKey(a.clone()),
        ),
        (
            "WorkflowError::UnknownNode",
            WorkflowError::UnknownNode(a.clone()),
        ),
        (
            "WorkflowError::SelfLoop",
            WorkflowError::SelfLoop(a.clone()),
        ),
        ("WorkflowError::CycleDetected", WorkflowError::CycleDetected),
        ("WorkflowError::NoEntryNodes", WorkflowError::NoEntryNodes),
        (
            "WorkflowError::InvalidParameterReference",
            WorkflowError::InvalidParameterReference {
                node_key: a.clone(),
                source_node_key: b.clone(),
            },
        ),
        (
            "WorkflowError::ReferenceWithoutConnection",
            WorkflowError::ReferenceWithoutConnection {
                node_key: a.clone(),
                source_node_key: b.clone(),
            },
        ),
        (
            "WorkflowError::InvalidActionKey",
            WorkflowError::InvalidActionKey {
                key: "bad key".to_owned(),
                reason: "keys must be dotted".to_owned(),
            },
        ),
        (
            "WorkflowError::InvalidPluginKey",
            WorkflowError::InvalidPluginKey {
                key: "bad key".to_owned(),
                reason: "keys must be lowercase".to_owned(),
            },
        ),
        (
            "WorkflowError::InvalidTrigger",
            WorkflowError::InvalidTrigger {
                reason: "cron expression is empty".to_owned(),
            },
        ),
        (
            "WorkflowError::UnsupportedSchema",
            WorkflowError::UnsupportedSchema { version: 9, max: 1 },
        ),
        (
            "WorkflowError::InvalidOwnerId",
            WorkflowError::InvalidOwnerId,
        ),
        (
            "WorkflowError::GraphError",
            WorkflowError::GraphError("edge resolution failed".to_owned()),
        ),
        (
            "WorkflowError::DuplicateConnection",
            WorkflowError::DuplicateConnection { from: a, to: b },
        ),
    ]
}

/// One instance of every plan-integrity and registry-compatibility rejection.
fn plugin_rejections() -> Vec<(&'static str, Box<dyn ActivationDiagnostics>)> {
    use nebula_core::{ExecutablePlanRevisionId, PluginSetId, WorkerFlavorRevisionId};
    use nebula_plugin::{
        ExecutablePlanIntegrityError, PlanRegistryCompatibilityError, WorkerFlavorIntegrityError,
    };

    vec![
        (
            "ExecutablePlanIntegrityError::UnsupportedFormat",
            Box::new(ExecutablePlanIntegrityError::UnsupportedFormat),
        ),
        (
            "ExecutablePlanIntegrityError::NonCanonical",
            Box::new(ExecutablePlanIntegrityError::NonCanonical {
                section: "bindings",
            }),
        ),
        (
            "ExecutablePlanIntegrityError::ConvertersUnsupported",
            Box::new(ExecutablePlanIntegrityError::ConvertersUnsupported),
        ),
        (
            "ExecutablePlanIntegrityError::UnknownCredentialCapability",
            Box::new(ExecutablePlanIntegrityError::UnknownCredentialCapability),
        ),
        (
            "ExecutablePlanIntegrityError::CanonicalEncoding",
            Box::new(ExecutablePlanIntegrityError::CanonicalEncoding),
        ),
        (
            "ExecutablePlanIntegrityError::RevisionIdMismatch",
            Box::new(ExecutablePlanIntegrityError::RevisionIdMismatch {
                claimed: ExecutablePlanRevisionId::from_bytes([0x11; 32]),
                computed: ExecutablePlanRevisionId::from_bytes([0x22; 32]),
            }),
        ),
        (
            "WorkerFlavorIntegrityError::UnsupportedRecordVersion",
            Box::new(WorkerFlavorIntegrityError::UnsupportedRecordVersion { found: 99 }),
        ),
        (
            "WorkerFlavorIntegrityError::UnsupportedCanonicalHashVersion",
            Box::new(WorkerFlavorIntegrityError::UnsupportedCanonicalHashVersion { found: 99 }),
        ),
        (
            "WorkerFlavorIntegrityError::RevisionIdMismatch",
            Box::new(WorkerFlavorIntegrityError::RevisionIdMismatch {
                claimed: WorkerFlavorRevisionId::from_bytes([0x11; 32]),
                computed: WorkerFlavorRevisionId::from_bytes([0x22; 32]),
            }),
        ),
        (
            "PlanRegistryCompatibilityError::PluginSetMismatch",
            Box::new(PlanRegistryCompatibilityError::PluginSetMismatch {
                plan: PluginSetId::from_bytes([0x11; 32]),
                registry: PluginSetId::from_bytes([0x22; 32]),
            }),
        ),
        (
            "PlanRegistryCompatibilityError::WorkerFlavorMismatch",
            Box::new(PlanRegistryCompatibilityError::WorkerFlavorMismatch {
                plan: WorkerFlavorRevisionId::from_bytes([0x33; 32]),
                registry: WorkerFlavorRevisionId::from_bytes([0x44; 32]),
            }),
        ),
        (
            "PlanRegistryCompatibilityError::ContractMismatch",
            Box::new(PlanRegistryCompatibilityError::ContractMismatch { section: "actions" }),
        ),
    ]
}

fn report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ns14")
        .join("diagnostic-contract.json")
}

/// Emit the versioned report and fail if any rejection is missing a field.
#[test]
fn every_activation_rejection_satisfies_the_five_field_contract() {
    let mut entries = Vec::new();
    for (name, rejection) in workflow_rejections() {
        entries.extend(entries_for(name, &rejection));
    }
    for (name, rejection) in plugin_rejections() {
        entries.extend(entries_for(name, rejection.as_ref()));
    }

    let report = DiagnosticContractReport::new(entries);
    assert_eq!(report.report_version, DIAGNOSTIC_CONTRACT_REPORT_VERSION);

    // Write before asserting, so a failing run still leaves the evidence that
    // shows *which* rejections fell short.
    let path = report_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("the report directory is creatable");
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&report).expect("the report serializes"),
    )
    .expect("the report is writable");

    let gaps = report.incomplete();
    assert!(
        gaps.is_empty(),
        "these rejections do not satisfy the NS14 five-field contract: {:?}",
        gaps.iter()
            .map(|entry| &entry.rejection)
            .collect::<Vec<_>>()
    );
}

/// Two rejections answering to one code would make the machine-readable half
/// of the contract useless — a client could not tell them apart.
#[test]
fn codes_are_distinct_across_every_producing_crate() {
    let mut entries = Vec::new();
    for (name, rejection) in workflow_rejections() {
        entries.extend(entries_for(name, &rejection));
    }
    for (name, rejection) in plugin_rejections() {
        entries.extend(entries_for(name, rejection.as_ref()));
    }

    let mut codes: Vec<&str> = entries.iter().map(|entry| entry.code.as_str()).collect();
    let total = codes.len();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(
        codes.len(),
        total,
        "two rejections must not answer to the same diagnostic code"
    );
}

/// Codes are namespaced by the layer that raises them, so a consumer can route
/// on the prefix without a lookup table.
#[test]
fn codes_are_namespaced_by_their_producing_layer() {
    for (name, rejection) in workflow_rejections() {
        for entry in entries_for(name, &rejection) {
            assert!(
                entry.code.starts_with("WORKFLOW:"),
                "{name} reported an unnamespaced code: {}",
                entry.code
            );
        }
    }
    for (name, rejection) in plugin_rejections() {
        for entry in entries_for(name, rejection.as_ref()) {
            assert!(
                entry.code.starts_with("PLUGIN_"),
                "{name} reported an unnamespaced code: {}",
                entry.code
            );
        }
    }
}
