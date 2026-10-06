//! Contract of the execution listing projection and history query: the
//! reference predicate every backend's `list_history` is held to.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use nebula_storage_port::{
    ExecutionHistoryCursor, ExecutionHistoryPage, ExecutionHistoryPageSize, ExecutionHistoryQuery,
    ExecutionListingStatus, ExecutionSummary,
};

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 5, 0, 0, 0).unwrap() + Duration::seconds(seconds)
}

fn summary(id: &str, workflow: &str, status: ExecutionListingStatus, created: i64) -> ExecutionSummary {
    ExecutionSummary {
        id: id.into(),
        workflow_id: workflow.into(),
        status,
        created_at: at(created),
        started_at: None,
        finished_at: None,
        updated_at: at(created),
    }
}

#[test]
fn status_wire_names_are_the_execution_serde_names() {
    for status in ExecutionListingStatus::ALL {
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json, serde_json::json!(status.as_str()));
        assert_eq!(ExecutionListingStatus::from_stored(status.as_str()), Some(status));
    }
    assert_eq!(ExecutionListingStatus::TimedOut.as_str(), "timed_out");
    assert_eq!(ExecutionListingStatus::from_stored("Created"), None);
}

#[test]
fn active_set_is_exactly_the_non_terminal_statuses() {
    let active = ExecutionListingStatus::active();
    assert_eq!(
        active,
        BTreeSet::from([
            ExecutionListingStatus::Created,
            ExecutionListingStatus::Running,
            ExecutionListingStatus::Paused,
            ExecutionListingStatus::Cancelling,
        ])
    );
}

#[test]
fn page_size_is_bounded() {
    assert!(ExecutionHistoryPageSize::new(0).is_err());
    assert!(ExecutionHistoryPageSize::new(101).is_err());
    assert_eq!(ExecutionHistoryPageSize::new(100).unwrap().get(), 100);
    assert_eq!(ExecutionHistoryPageSize::default().get(), 20);
}

#[test]
fn query_filters_compose() {
    let failed = summary("b", "wf1", ExecutionListingStatus::Failed, 10);
    let running = summary("a", "wf1", ExecutionListingStatus::Running, 10);
    let other_workflow = summary("c", "wf2", ExecutionListingStatus::Failed, 10);

    let query = ExecutionHistoryQuery::new()
        .workflow("wf1")
        .statuses(BTreeSet::from([ExecutionListingStatus::Failed]));
    assert!(query.admits(&failed));
    assert!(!query.admits(&running));
    assert!(!query.admits(&other_workflow));
    assert!(ExecutionHistoryQuery::new().admits(&running), "empty filter admits all");
}

#[test]
fn creation_range_is_half_open() {
    let query = ExecutionHistoryQuery::new()
        .created_after(at(10))
        .created_before(at(20));
    assert!(query.admits(&summary("x", "w", ExecutionListingStatus::Created, 10)));
    assert!(query.admits(&summary("x", "w", ExecutionListingStatus::Created, 19)));
    assert!(!query.admits(&summary("x", "w", ExecutionListingStatus::Created, 20)));
    assert!(!query.admits(&summary("x", "w", ExecutionListingStatus::Created, 9)));
}

#[test]
fn cursor_is_strictly_after_in_descending_order_with_id_tiebreak() {
    let query = ExecutionHistoryQuery::new().after(ExecutionHistoryCursor::new(at(10), "m"));
    // Same instant: only smaller ids follow.
    assert!(query.admits(&summary("l", "w", ExecutionListingStatus::Created, 10)));
    assert!(!query.admits(&summary("m", "w", ExecutionListingStatus::Created, 10)));
    assert!(!query.admits(&summary("n", "w", ExecutionListingStatus::Created, 10)));
    // Older instants always follow, newer never.
    assert!(query.admits(&summary("z", "w", ExecutionListingStatus::Created, 9)));
    assert!(!query.admits(&summary("a", "w", ExecutionListingStatus::Created, 11)));
}

#[test]
fn overfetched_rows_yield_a_cursor_only_when_another_page_exists() {
    let size = ExecutionHistoryPageSize::new(2).unwrap();
    let rows = vec![
        summary("c", "w", ExecutionListingStatus::Created, 3),
        summary("b", "w", ExecutionListingStatus::Created, 2),
        summary("a", "w", ExecutionListingStatus::Created, 1),
    ];
    let page = ExecutionHistoryPage::from_overfetched(rows.clone(), size);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_cursor, Some(ExecutionHistoryCursor::new(at(2), "b")));

    let last = ExecutionHistoryPage::from_overfetched(rows[..2].to_vec(), size);
    assert_eq!(last.items.len(), 2);
    assert_eq!(last.next_cursor, None);
}
