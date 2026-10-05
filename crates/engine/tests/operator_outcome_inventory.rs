//! Inventory guard for the execution-control operator vocabulary (NS21).
//!
//! `nebula_execution::ExecutionControlOutcome` is the one operator-facing
//! execution-control vocabulary. Every `*Outcome` enum declared at an
//! execution-control seam must either map to it or say why it is not an
//! operator-visible control decision. A new outcome enum at one of these
//! seams fails this test until it is added to [`INVENTORY`] with that reason.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where execution-control decisions are made or projected.
const SEAMS: &[&str] = &[
    "crates/engine/src/engine",
    "crates/engine/src/control_consumer.rs",
    "crates/execution/src",
    "crates/storage-port/src/batch.rs",
    "crates/storage-port/src/store/turn_handoff.rs",
    "crates/storage-port/src/dto/operation_ledger.rs",
    "crates/storage-port/src/dto/revision_catalog.rs",
];

/// Every outcome enum at a seam, with its mapping or its exclusion reason.
const INVENTORY: &[(&str, &str)] = &[
    (
        "ExecutionControlOutcome",
        "the vocabulary itself: accepted, fenced, deferred, throttled, recovered, flavor-mismatch",
    ),
    (
        "ControlTurnCommitOutcome",
        "Accepted -> accepted; ClaimFenced and FencedOut -> fenced; VersionConflict -> deferred; \
         FlavorMismatch -> flavor-mismatch; ClaimSuperseded proves no delivery, so nothing is observed",
    ),
    (
        "ControlFlavorRefusalOutcome",
        "Recorded and AlreadyRecorded -> flavor-mismatch; ClaimFenced -> fenced; \
         ClaimSuperseded and NoMismatch are not decisions",
    ),
    (
        "ExecutionAdmissionRefusalOutcome",
        "Recorded and AlreadyRecorded -> throttled; FencedOut and \
         MissingAcceptedTurn are an owner that cannot attribute the refusal (counted as unrecorded)",
    ),
    (
        "ClaimedControlTurnOutcome",
        "engine projection of ControlTurnCommitOutcome; journaled by the storage seam",
    ),
    (
        "ClaimedStartOutcome",
        "engine projection of the Start handoff; accepted is journaled with the handoff",
    ),
    (
        "ClaimedControlDispatchOutcome",
        "queue-delivery projection of the claimed turn outcomes above",
    ),
    (
        "RecoveryTurnOutcome",
        "Accepted -> recovered, journaled with the recovery acceptance; the other \
         variants leave the retained turn untouched",
    ),
    (
        "TransitionOutcome",
        "ordinary CAS/fence result of ExecutionStore::commit, not a control decision",
    ),
    (
        "ResumeOutcome",
        "wait/signal re-arm acknowledgement; ownership decisions go through the control turn",
    ),
    (
        "SatisfyOutcome",
        "signal-wait satisfaction count, not an ownership decision",
    ),
    (
        "CancelDanglingOutcome",
        "cancellation sweep count for in-flight nodes, not an ownership decision",
    ),
    (
        "FailureOutcome",
        "node failure routing: the action-result axis (ADR-0105), not execution control",
    ),
    (
        "AttemptOutcome",
        "node attempt result: the action-result axis, not execution control",
    ),
    (
        "PrepareOutcome",
        "provider effect-slot preparation: the effect axis, not execution control",
    ),
    (
        "KnownOutcome",
        "provider effect answer: the effect axis, not execution control",
    ),
    (
        "RevisionInsertOutcome",
        "plan/flavor catalog installation; flavor-mismatch is decided at the control seam",
    ),
    (
        "BeginDrainOutcome",
        "plan/flavor catalog drain lifecycle, an administrative operation",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/engine sits two levels below the workspace root")
        .to_path_buf()
}

fn rust_sources(path: &Path, into: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path.to_path_buf());
        }
        return;
    }
    let entries = std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("seam {} must be readable: {error}", path.display()));
    for entry in entries {
        rust_sources(&entry.expect("directory entry").path(), into);
    }
}

/// The name of an `enum *Outcome` declared on this line, if any.
fn declared_outcome_enum(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.starts_with("//") {
        return None;
    }
    let after = line.split("enum ").nth(1)?;
    let before = &line[..line.len() - after.len() - "enum ".len()];
    let visibility_only = before
        .split_whitespace()
        .all(|token| token == "pub" || token.starts_with("pub("));
    if !visibility_only {
        return None;
    }
    let name = after
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .next()?;
    name.ends_with("Outcome").then_some(name)
}

fn declared_outcome_enums() -> BTreeSet<String> {
    let root = workspace_root();
    let mut files = Vec::new();
    for seam in SEAMS {
        rust_sources(&root.join(seam), &mut files);
    }
    let mut names = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", file.display()));
        names.extend(
            source
                .lines()
                .filter_map(declared_outcome_enum)
                .map(str::to_owned),
        );
    }
    names
}

#[test]
fn every_outcome_enum_at_a_control_seam_is_mapped_or_excluded() {
    let declared = declared_outcome_enums();
    let documented: BTreeSet<String> = INVENTORY
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    let unmapped: Vec<_> = declared.difference(&documented).collect();
    assert!(
        unmapped.is_empty(),
        "outcome enums at an execution-control seam need a mapping to \
         ExecutionControlOutcome or an exclusion reason in INVENTORY: {unmapped:?}"
    );
    let stale: Vec<_> = documented.difference(&declared).collect();
    assert!(
        stale.is_empty(),
        "INVENTORY names outcome enums that no longer exist at a seam: {stale:?}"
    );
    assert!(
        INVENTORY
            .iter()
            .all(|(_, reason)| !reason.trim().is_empty()),
        "every inventory entry states its mapping or exclusion"
    );
}

#[test]
fn the_scanner_recognises_declarations_and_ignores_everything_else() {
    assert_eq!(
        declared_outcome_enum("pub enum ControlTurnCommitOutcome {"),
        Some("ControlTurnCommitOutcome")
    );
    assert_eq!(
        declared_outcome_enum("    pub(crate) enum ResumeOutcome {"),
        Some("ResumeOutcome")
    );
    assert_eq!(
        declared_outcome_enum("enum FailureOutcome<T> {"),
        Some("FailureOutcome")
    );
    assert_eq!(declared_outcome_enum("/// enum DocOutcome"), None);
    assert_eq!(declared_outcome_enum("pub enum ControlReason {"), None);
    assert_eq!(
        declared_outcome_enum("let x = Some(enum_outcome);"),
        None,
        "only declarations count"
    );
}

/// Production sources that may construct a control observation.
const PRODUCERS: &[&str] = &["crates/storage/src", "crates/engine/src"];

/// Variant names of `ExecutionControlReason`, read from its declaration.
fn declared_reason_variants() -> Vec<String> {
    let path = workspace_root().join("crates/execution/src/control_observation.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    let body = source
        .split("pub enum ExecutionControlReason {")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("ExecutionControlReason declaration");
    body.lines()
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .map(str::trim)
        .filter(|line| !line.starts_with("//") && !line.starts_with('}'))
        .filter_map(|line| {
            let name: String = line
                .chars()
                .take_while(|character| character.is_alphanumeric())
                .collect();
            name.chars()
                .next()
                .is_some_and(char::is_uppercase)
                .then_some(name)
        })
        .collect()
}

/// Vocabulary nothing produces must not ship: every reason variant needs a
/// constructor in production storage or engine code (test files excluded).
#[test]
fn every_reason_variant_has_a_production_producer() {
    let variants = declared_reason_variants();
    assert!(
        variants.len() >= 6,
        "the reason declaration was not parsed: {variants:?}"
    );
    let root = workspace_root();
    let mut files = Vec::new();
    for producer in PRODUCERS {
        rust_sources(&root.join(producer), &mut files);
    }
    let production: String = files
        .iter()
        .filter(|file| {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            name != "tests.rs" && !name.ends_with("_tests.rs")
        })
        .map(|file| std::fs::read_to_string(file).unwrap_or_default())
        .collect();
    let unproduced: Vec<_> = variants
        .iter()
        .filter(|variant| !production.contains(&format!("ExecutionControlReason::{variant}")))
        .collect();
    assert!(
        unproduced.is_empty(),
        "ExecutionControlReason variants with no production producer: {unproduced:?}"
    );
}
