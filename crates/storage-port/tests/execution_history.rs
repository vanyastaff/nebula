//! Contract of the execution listing projection and history query: the
//! reference predicate every backend's `list_history` is held to.

use chrono::{DateTime, Duration, TimeZone, Utc};
use nebula_storage_port::{
    ExecutionHistoryCursor, ExecutionHistoryPage, ExecutionHistoryPageSize, ExecutionHistoryQuery,
    ExecutionListing, ExecutionListingStatus as Status, ExecutionStatusSet, ExecutionSummary,
    MicrosInstant,
};

fn datetime(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 5, 0, 0, 0).unwrap() + Duration::seconds(seconds)
}

fn at(seconds: i64) -> MicrosInstant {
    MicrosInstant::floor(datetime(seconds))
}

fn summary(id: &str, workflow: &str, status: Status, created: i64) -> ExecutionSummary {
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
fn status_names_round_trip_through_display_fromstr_and_serde() {
    for status in Status::ALL {
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!(status.as_str())
        );
        assert_eq!(status.to_string().parse::<Status>(), Ok(status));
    }
    assert_eq!(Status::TimedOut.as_str(), "timed_out");
    assert!(
        "Created".parse::<Status>().is_err(),
        "names are case-sensitive"
    );
}

#[test]
fn status_sets_are_closed_under_their_constants() {
    assert_eq!(
        ExecutionStatusSet::ACTIVE.iter().collect::<Vec<_>>(),
        Status::ALL
            .into_iter()
            .filter(|status| !status.is_terminal())
            .collect::<Vec<_>>()
    );
    assert!(ExecutionStatusSet::ALL.is_all());
    assert_eq!(ExecutionStatusSet::ALL.iter().count(), Status::ALL.len());
    assert!(ExecutionStatusSet::EMPTY.is_empty());
    assert_eq!(ExecutionStatusSet::default(), ExecutionStatusSet::ALL);

    let failures = ExecutionStatusSet::from([Status::Failed, Status::TimedOut]);
    assert!(failures.contains(Status::Failed) && !failures.contains(Status::Running));
    assert_eq!(format!("{failures:?}"), "{Failed, TimedOut}");
}

#[test]
fn micros_instants_round_explicitly() {
    let base = datetime(10);
    let fine = base + Duration::nanoseconds(1_500);
    assert_eq!(
        MicrosInstant::floor(fine).as_micros(),
        base.timestamp_micros() + 1
    );
    assert_eq!(
        MicrosInstant::ceil(fine).as_micros(),
        base.timestamp_micros() + 2
    );
    assert_eq!(MicrosInstant::ceil(base), MicrosInstant::floor(base));
    let instant = MicrosInstant::floor(fine);
    assert_eq!(
        MicrosInstant::from_micros(instant.as_micros()),
        Some(instant)
    );
}

#[test]
fn a_listing_is_stored_precision_and_only_terminal_listings_finish() {
    let fine = datetime(1) + Duration::nanoseconds(700);
    let running = ExecutionListing::new(Status::Running, Some(fine), Some(datetime(2)));
    assert_eq!(
        running.finished_at(),
        None,
        "a running execution is never finished"
    );
    assert_eq!(
        running.started_at(),
        Some(at(1)),
        "truncated to microseconds"
    );

    let failed = ExecutionListing::new(Status::Failed, Some(datetime(1)), Some(datetime(2)));
    assert_eq!(failed.finished_at(), Some(at(2)));
}

#[test]
fn page_size_is_bounded_and_fetches_one_extra_row() {
    assert!(ExecutionHistoryPageSize::new(0).is_err());
    assert!(ExecutionHistoryPageSize::new(101).is_err());
    assert_eq!(ExecutionHistoryPageSize::MAX.get(), 100);
    assert_eq!(ExecutionHistoryPageSize::default().get(), 20);
    let query = ExecutionHistoryQuery::new().with_page_size(ExecutionHistoryPageSize::MAX);
    assert_eq!(query.fetch_limit(), 101);
}

#[test]
fn query_filters_compose() {
    let failed = summary("b", "wf1", Status::Failed, 10);
    let running = summary("a", "wf1", Status::Running, 10);
    let other_workflow = summary("c", "wf2", Status::Failed, 10);

    let query = ExecutionHistoryQuery::new()
        .with_workflow("wf1")
        .with_statuses([Status::Failed].into());
    assert!(query.admits(&failed));
    assert!(!query.admits(&running));
    assert!(!query.admits(&other_workflow));
    assert!(
        ExecutionHistoryQuery::new().admits(&running),
        "no filter admits all"
    );
}

#[test]
fn creation_range_is_half_open() {
    let query = ExecutionHistoryQuery::new()
        .with_created_after(datetime(10))
        .with_created_before(datetime(20));
    assert!(query.admits(&summary("x", "w", Status::Created, 10)));
    assert!(query.admits(&summary("x", "w", Status::Created, 19)));
    assert!(!query.admits(&summary("x", "w", Status::Created, 20)));
    assert!(!query.admits(&summary("x", "w", Status::Created, 9)));
}

/// Stored keys are whole microseconds; a sub-microsecond bound must admit the
/// same rows as the SQL backends, which compare against integer microseconds.
#[test]
fn sub_microsecond_bounds_round_up_to_the_stored_precision() {
    let row = summary("x", "w", Status::Created, 10);
    let just_after = datetime(10) + Duration::nanoseconds(500);
    let after = ExecutionHistoryQuery::new().with_created_after(just_after);
    assert!(
        !after.admits(&row),
        "a row older than the bound is excluded"
    );
    let before = ExecutionHistoryQuery::new().with_created_before(just_after);
    assert!(
        before.admits(&row),
        "a row older than the bound is included"
    );
}

#[test]
fn cursor_is_strictly_after_in_descending_order_with_id_tiebreak() {
    let query = ExecutionHistoryQuery::new().with_cursor(ExecutionHistoryCursor::new(at(10), "m"));
    // Same instant: only smaller ids follow.
    assert!(query.admits(&summary("l", "w", Status::Created, 10)));
    assert!(!query.admits(&summary("m", "w", Status::Created, 10)));
    assert!(!query.admits(&summary("n", "w", Status::Created, 10)));
    // Older instants always follow, newer never.
    assert!(query.admits(&summary("z", "w", Status::Created, 9)));
    assert!(!query.admits(&summary("a", "w", Status::Created, 11)));
}

#[test]
fn overfetched_rows_yield_a_cursor_only_when_another_page_exists() {
    let query =
        ExecutionHistoryQuery::new().with_page_size(ExecutionHistoryPageSize::new(2).unwrap());
    let rows = vec![
        summary("c", "w", Status::Created, 3),
        summary("b", "w", Status::Created, 2),
        summary("a", "w", Status::Created, 1),
    ];
    let page = ExecutionHistoryPage::from_overfetched(rows.clone(), &query);
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.next_cursor,
        Some(ExecutionHistoryCursor::new(at(2), "b"))
    );

    let last = ExecutionHistoryPage::from_overfetched(rows[..2].to_vec(), &query);
    assert_eq!(last.items.len(), 2);
    assert_eq!(last.next_cursor, None);
}
