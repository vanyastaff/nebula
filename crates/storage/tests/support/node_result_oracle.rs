//! One shared acceptance oracle for the node-output / node-result store.
//!
//! The in-memory reference model, SQLite and PostgreSQL implement the same
//! `NodeResultStore`, so they must answer every save and load identically:
//! three independent slots (raw output, typed result, workflow input), the
//! latest write wins, rows bound to tenant, execution and node, bulk loads in
//! node-id byte order, and a schema version this binary cannot decode refused.
//! Each backend's test file supplies a store plus the execution store whose
//! rows the SQL backends reference, and runs the same cases.
//!
//! `node_result_relational_suite!` holds the cases only the SQL backends
//! answer: the reference from every row to its execution, which the
//! independently composed in-memory store does not check.
//!
//! Cases are keyed by a per-process namespace and a per-case seed so they can
//! share one durable store across runs.

use nebula_storage_port::dto::{MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord};
use nebula_storage_port::store::{ExecutionStore, NodeResultStore};
use nebula_storage_port::{Scope, StorageError};

/// Per-process namespace for tenants and executions. No case may adopt rows
/// left by an earlier test run.
static NAMESPACE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().simple().to_string());

pub(crate) fn scope() -> Scope {
    Scope::new(
        format!("ws-noderes-{}", *NAMESPACE),
        format!("org-noderes-{}", *NAMESPACE),
    )
}

/// A second tenant, used to prove one tenant cannot reach another's rows.
pub(crate) fn other_scope() -> Scope {
    Scope::new(
        format!("ws-noderes-other-{}", *NAMESPACE),
        format!("org-noderes-other-{}", *NAMESPACE),
    )
}

/// The `n`th execution of the case seeded `seed`.
pub(crate) fn execution_id(seed: u8, n: u8) -> String {
    format!("exe-noderes-{}-{seed:02x}-{n}", *NAMESPACE)
}

/// The workflow every case's executions run; SQL backends seed it first.
pub(crate) const WORKFLOW: &str = "workflow";

pub(crate) fn record(kind_tag: &str, json: serde_json::Value) -> NodeResultRecord {
    NodeResultRecord {
        kind_tag: kind_tag.into(),
        json,
        schema_version: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    }
}

/// Create execution `n` of `seed` under `scope` and return its id.
pub(crate) async fn create_execution(
    executions: &dyn ExecutionStore,
    scope: &Scope,
    seed: u8,
    n: u8,
) -> String {
    let id = execution_id(seed, n);
    executions
        .create(
            scope,
            &id,
            WORKFLOW,
            serde_json::json!({"status":"Created"}),
        )
        .await
        .expect("create the fixture execution");
    id
}

/// Runs every shared case against `$store`, an expression evaluating to a
/// future of `Option<(impl NodeResultStore, impl ExecutionStore)>`; `None`
/// fails each case loudly (an unreachable deployment backend is never a pass).
#[macro_export]
macro_rules! node_result_conformance_suite {
    ($store:expr) => {
        $crate::node_result_case!(absent_slots_load_as_nothing, 0x01, $store);
        $crate::node_result_case!(a_record_round_trips_as_an_equal_value, 0x02, $store);
        $crate::node_result_case!(a_save_replaces_its_slot, 0x03, $store);
        $crate::node_result_case!(output_and_result_are_separate_slots, 0x04, $store);
        $crate::node_result_case!(slots_are_bound_to_their_execution_and_node, 0x05, $store);
        $crate::node_result_case!(another_scope_never_observes_a_row, 0x06, $store);
        $crate::node_result_case!(bulk_loads_answer_in_node_id_byte_order, 0x07, $store);
        $crate::node_result_case!(a_newer_schema_version_is_refused_on_save, 0x08, $store);
        $crate::node_result_case!(the_workflow_input_is_one_slot_per_execution, 0x09, $store);
    };
}

/// Runs the SQL-only cases: a row references its execution.
#[macro_export]
macro_rules! node_result_relational_suite {
    ($store:expr) => {
        $crate::node_result_case!(a_save_for_a_missing_execution_is_not_found, 0x41, $store);
        $crate::node_result_case!(another_scope_cannot_write_into_an_execution, 0x42, $store);
    };
}

#[macro_export]
macro_rules! node_result_case {
    ($case:ident, $seed:expr, $store:expr) => {
        #[tokio::test]
        async fn $case() {
            let Some((store, executions)) = $store.await else {
                panic!(concat!(
                    stringify!($case),
                    ": backend unreachable — the case cannot run and must fail rather than \
                     pass unchecked; reach the backend (set DATABASE_URL for postgres) or run \
                     without this feature"
                ));
            };
            oracle::$case(&store, &executions, $seed).await;
        }
    };
}

pub(crate) async fn absent_slots_load_as_nothing(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "a")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .load_node_result(&scope, &execution, "a")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.get_workflow_input(&scope, &execution).await.unwrap(),
        None
    );
    assert!(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .load_all_results(&scope, &execution)
            .await
            .unwrap()
            .is_empty()
    );
    // An execution nobody wrote for loads as nothing too.
    let unknown = execution_id(seed, 9);
    assert_eq!(
        store.load_node_output(&scope, &unknown, "a").await.unwrap(),
        None
    );
    assert!(
        store
            .load_all_results(&scope, &unknown)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Every JSON shape a node may emit: nesting, escapes, non-ASCII text,
/// integer extremes, floats, booleans and null.
fn rich_payload() -> serde_json::Value {
    serde_json::json!({
        "string": "line\nbreak \"quoted\" \\ tab\t ünïcødé 漢字 🚀",
        "empty": "",
        "integers": [0, -1, i64::MIN, i64::MAX, u64::MAX],
        "floats": [1.5, -0.25, 6.02e23],
        "flags": [true, false],
        "nothing": null,
        "nested": {"z": {"y": [[], {}]}, "a": [1, "two", {"three": 3}]},
    })
}

pub(crate) async fn a_record_round_trips_as_an_equal_value(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    let output = record("Value", rich_payload());
    let result = record("Branch", serde_json::json!({"Branch": {"selected": "yes"}}));
    let input = record("Input", serde_json::json!("plain string input"));
    let oldest = NodeResultRecord {
        kind_tag: String::new(),
        json: serde_json::Value::Null,
        schema_version: 0,
    };
    store
        .save_node_output(&scope, &execution, "node", output.clone())
        .await
        .unwrap();
    store
        .save_node_result(&scope, &execution, "node", result.clone())
        .await
        .unwrap();
    store
        .save_node_output(&scope, &execution, "oldest", oldest.clone())
        .await
        .unwrap();
    store
        .set_workflow_input(&scope, &execution, input.clone())
        .await
        .unwrap();

    assert_eq!(
        store
            .load_node_output(&scope, &execution, "node")
            .await
            .unwrap(),
        Some(output.clone())
    );
    assert_eq!(
        store
            .load_node_result(&scope, &execution, "node")
            .await
            .unwrap(),
        Some(result.clone())
    );
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "oldest")
            .await
            .unwrap(),
        Some(oldest.clone()),
        "schema version 0, an empty tag and a null payload are records too"
    );
    assert_eq!(
        store.get_workflow_input(&scope, &execution).await.unwrap(),
        Some(input)
    );
    assert_eq!(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap(),
        vec![("node".to_owned(), output), ("oldest".to_owned(), oldest)]
    );
    assert_eq!(
        store.load_all_results(&scope, &execution).await.unwrap(),
        vec![("node".to_owned(), result)]
    );
}

pub(crate) async fn a_save_replaces_its_slot(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    for (n, tag) in ["first", "second", "third"].into_iter().enumerate() {
        store
            .save_node_output(&scope, &execution, "a", record(tag, serde_json::json!(n)))
            .await
            .unwrap();
        store
            .save_node_result(
                &scope,
                &execution,
                "a",
                record(tag, serde_json::json!({"n": n})),
            )
            .await
            .unwrap();
    }
    // The very same record again is an ordinary save.
    store
        .save_node_output(
            &scope,
            &execution,
            "a",
            record("third", serde_json::json!(2)),
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .load_node_output(&scope, &execution, "a")
            .await
            .unwrap(),
        Some(record("third", serde_json::json!(2)))
    );
    assert_eq!(
        store
            .load_node_result(&scope, &execution, "a")
            .await
            .unwrap(),
        Some(record("third", serde_json::json!({"n": 2})))
    );
    assert_eq!(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap()
            .len(),
        1,
        "a replaced slot is one row"
    );
    assert_eq!(
        store
            .load_all_results(&scope, &execution)
            .await
            .unwrap()
            .len(),
        1
    );
}

pub(crate) async fn output_and_result_are_separate_slots(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    store
        .save_node_output(&scope, &execution, "only-output", record("Value", 1.into()))
        .await
        .unwrap();
    store
        .save_node_result(&scope, &execution, "only-result", record("Value", 2.into()))
        .await
        .unwrap();

    assert_eq!(
        store
            .load_node_result(&scope, &execution, "only-output")
            .await
            .unwrap(),
        None,
        "an output is not a result"
    );
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "only-result")
            .await
            .unwrap(),
        None,
        "a result is not an output"
    );
    assert_eq!(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap(),
        vec![("only-output".to_owned(), record("Value", 1.into()))]
    );
    assert_eq!(
        store.load_all_results(&scope, &execution).await.unwrap(),
        vec![("only-result".to_owned(), record("Value", 2.into()))]
    );
    assert_eq!(
        store.get_workflow_input(&scope, &execution).await.unwrap(),
        None,
        "node slots never fill the workflow input"
    );
}

pub(crate) async fn slots_are_bound_to_their_execution_and_node(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let first = create_execution(executions, &scope, seed, 0).await;
    let second = create_execution(executions, &scope, seed, 1).await;
    store
        .save_node_output(&scope, &first, "a", record("first", 1.into()))
        .await
        .unwrap();
    store
        .save_node_result(&scope, &first, "a", record("first", 1.into()))
        .await
        .unwrap();
    store
        .set_workflow_input(&scope, &first, record("first", 1.into()))
        .await
        .unwrap();
    store
        .save_node_output(&scope, &second, "a", record("second", 2.into()))
        .await
        .unwrap();

    assert_eq!(
        store.load_node_output(&scope, &first, "a").await.unwrap(),
        Some(record("first", 1.into())),
        "another execution's write of the same node is another row"
    );
    assert_eq!(
        store.load_node_output(&scope, &second, "a").await.unwrap(),
        Some(record("second", 2.into()))
    );
    assert_eq!(
        store.load_node_result(&scope, &second, "a").await.unwrap(),
        None
    );
    assert_eq!(
        store.get_workflow_input(&scope, &second).await.unwrap(),
        None
    );
    assert_eq!(
        store.load_node_output(&scope, &first, "b").await.unwrap(),
        None,
        "another node of the same execution is another row"
    );
    assert_eq!(
        store.load_node_output(&scope, &first, "A").await.unwrap(),
        None,
        "node ids compare exactly"
    );
    assert_eq!(
        store.load_all_node_outputs(&scope, &second).await.unwrap(),
        vec![("a".to_owned(), record("second", 2.into()))]
    );
}

pub(crate) async fn another_scope_never_observes_a_row(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    store
        .save_node_output(&scope, &execution, "a", record("Value", 1.into()))
        .await
        .unwrap();
    store
        .save_node_result(&scope, &execution, "a", record("Value", 1.into()))
        .await
        .unwrap();
    store
        .set_workflow_input(&scope, &execution, record("Input", 1.into()))
        .await
        .unwrap();

    // The other tenant, and each half of the scope swapped for the other's.
    let other = other_scope();
    let probes = [
        other.clone(),
        Scope::new(scope.workspace_id.clone(), other.org_id.clone()),
        Scope::new(other.workspace_id.clone(), scope.org_id.clone()),
    ];
    for probe in &probes {
        assert_eq!(
            store
                .load_node_output(probe, &execution, "a")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .load_node_result(probe, &execution, "a")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store.get_workflow_input(probe, &execution).await.unwrap(),
            None
        );
        assert!(
            store
                .load_all_node_outputs(probe, &execution)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .load_all_results(probe, &execution)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

pub(crate) async fn bulk_loads_answer_in_node_id_byte_order(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    // Written out of order; byte order puts upper case before lower case and
    // multi-byte UTF-8 after ASCII, unlike a locale collation.
    let written = ["b", "ä", "a1", "B", "a", "_", "Z"];
    for (n, node) in written.into_iter().enumerate() {
        store
            .save_node_output(&scope, &execution, node, record("Value", n.into()))
            .await
            .unwrap();
        store
            .save_node_result(&scope, &execution, node, record("Value", n.into()))
            .await
            .unwrap();
    }
    let expected = ["B", "Z", "_", "a", "a1", "b", "ä"];
    let outputs: Vec<String> = store
        .load_all_node_outputs(&scope, &execution)
        .await
        .unwrap()
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    assert_eq!(outputs, expected);
    let results: Vec<String> = store
        .load_all_results(&scope, &execution)
        .await
        .unwrap()
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    assert_eq!(results, expected);
}

pub(crate) async fn a_newer_schema_version_is_refused_on_save(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    let accepted = record("Value", 1.into());
    store
        .save_node_output(&scope, &execution, "a", accepted.clone())
        .await
        .unwrap();
    store
        .save_node_result(&scope, &execution, "a", accepted.clone())
        .await
        .unwrap();
    store
        .set_workflow_input(&scope, &execution, accepted.clone())
        .await
        .unwrap();

    for newer in [MAX_SUPPORTED_RESULT_SCHEMA_VERSION + 1, u32::MAX] {
        let unreadable = NodeResultRecord {
            schema_version: newer,
            ..record("Future", 2.into())
        };
        let refused = |result: Result<(), StorageError>| {
            assert!(
                matches!(
                    result,
                    Err(StorageError::UnknownSchemaVersion { found, max })
                        if found == newer && max == MAX_SUPPORTED_RESULT_SCHEMA_VERSION
                ),
                "schema version {newer} must be refused, got {result:?}"
            );
        };
        refused(
            store
                .save_node_output(&scope, &execution, "a", unreadable.clone())
                .await,
        );
        refused(
            store
                .save_node_output(&scope, &execution, "b", unreadable.clone())
                .await,
        );
        refused(
            store
                .save_node_result(&scope, &execution, "a", unreadable.clone())
                .await,
        );
        refused(
            store
                .set_workflow_input(&scope, &execution, unreadable.clone())
                .await,
        );
    }

    // Nothing was written: the earlier records stand, no new slot appeared.
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "a")
            .await
            .unwrap(),
        Some(accepted.clone())
    );
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "b")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .load_node_result(&scope, &execution, "a")
            .await
            .unwrap(),
        Some(accepted.clone())
    );
    assert_eq!(
        store.get_workflow_input(&scope, &execution).await.unwrap(),
        Some(accepted)
    );
}

pub(crate) async fn the_workflow_input_is_one_slot_per_execution(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    store
        .set_workflow_input(
            &scope,
            &execution,
            record("Input", serde_json::json!({"v": 1})),
        )
        .await
        .unwrap();
    store
        .set_workflow_input(
            &scope,
            &execution,
            record("Input", serde_json::json!({"v": 2})),
        )
        .await
        .unwrap();
    assert_eq!(
        store.get_workflow_input(&scope, &execution).await.unwrap(),
        Some(record("Input", serde_json::json!({"v": 2}))),
        "the latest input wins"
    );
    assert!(
        store
            .load_all_node_outputs(&scope, &execution)
            .await
            .unwrap()
            .is_empty(),
        "the workflow input is not a node output"
    );
}

/// Assert `result` is the SQL backends' answer for a row whose execution
/// they do not hold.
fn assert_missing_execution<T: std::fmt::Debug>(result: Result<T, StorageError>, execution: &str) {
    assert!(
        matches!(
            &result,
            Err(StorageError::NotFound { entity: "execution", id }) if id == execution
        ),
        "a row needs its execution, got {result:?}"
    );
}

pub(crate) async fn a_save_for_a_missing_execution_is_not_found(
    store: &dyn NodeResultStore,
    _executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let missing = execution_id(seed, 0);
    assert_missing_execution(
        store
            .save_node_output(&scope, &missing, "a", record("Value", 1.into()))
            .await,
        &missing,
    );
    assert_missing_execution(
        store
            .save_node_result(&scope, &missing, "a", record("Value", 1.into()))
            .await,
        &missing,
    );
    assert_missing_execution(
        store
            .set_workflow_input(&scope, &missing, record("Input", 1.into()))
            .await,
        &missing,
    );
    assert!(
        store
            .load_all_node_outputs(&scope, &missing)
            .await
            .unwrap()
            .is_empty()
    );
}

pub(crate) async fn another_scope_cannot_write_into_an_execution(
    store: &dyn NodeResultStore,
    executions: &dyn ExecutionStore,
    seed: u8,
) {
    let scope = scope();
    let execution = create_execution(executions, &scope, seed, 0).await;
    store
        .save_node_output(&scope, &execution, "a", record("Owner", 1.into()))
        .await
        .unwrap();

    // The execution id is the owner's; under another scope it does not exist.
    let other = other_scope();
    assert_missing_execution(
        store
            .save_node_output(&other, &execution, "a", record("Intruder", 2.into()))
            .await,
        &execution,
    );
    assert_missing_execution(
        store
            .set_workflow_input(&other, &execution, record("Intruder", 2.into()))
            .await,
        &execution,
    );
    assert_eq!(
        store
            .load_node_output(&scope, &execution, "a")
            .await
            .unwrap(),
        Some(record("Owner", 1.into()))
    );
}
