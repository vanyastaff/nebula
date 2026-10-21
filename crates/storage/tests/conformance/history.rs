//! Execution listing projection and history in the execution baseline.
//!
//! Every commit updates the listing projection alongside state. These
//! assertions pin status filtering and keyset history on every backend.

use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{
    ExecutionHistoryCursor, ExecutionHistoryPageSize, ExecutionHistoryQuery, ExecutionListing,
    ExecutionListingStatus as Status, ExecutionStatusSet, FencingToken, MicrosInstant, Scope,
    TransitionBatch, TransitionOutcome,
};

use super::{Backend, scope_a, scope_b, seed_workflow};

fn micros(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_micros(1_759_665_600_000_000 + seconds * 1_000_000 + 123_456)
        .expect("fixed instant is in range")
}

/// Commit `listing` as the next snapshot of `id`, returning the new version.
async fn commit_listing(
    store: &dyn ExecutionStore,
    scope: &Scope,
    id: &str,
    expected_version: u64,
    fencing: FencingToken,
    listing: ExecutionListing,
) -> u64 {
    let batch = TransitionBatch::new(
        scope.clone(),
        id,
        expected_version,
        fencing,
        serde_json::json!({"status": listing.status().as_str()}),
        listing,
    );
    match store.commit(batch).await.expect("commit") {
        TransitionOutcome::Applied { new_version } => new_version,
        other => panic!("listing commit must apply, got {other:?}"),
    }
}

async fn lease(store: &dyn ExecutionStore, scope: &Scope, id: &str) -> FencingToken {
    store
        .acquire_lease(scope, id, "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .expect("lease is free")
}

fn whole_history() -> ExecutionHistoryQuery {
    ExecutionHistoryQuery::new().with_page_size(ExecutionHistoryPageSize::MAX)
}

/// The status column follows every commit, the projection timestamps
/// round-trip at microsecond precision, and a terminal execution leaves the
/// active scan.
pub(crate) async fn assert_status_projection_follows_commit(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    // The due-timer sweep is worker-global.
    let worker = backend.worker_execution_store().await;
    let s = scope_a();
    seed_workflow(backend, &s, "wf_listing").await;
    store
        .create(&s, "exe_listing", "wf_listing", serde_json::json!({}))
        .await
        .expect("create");
    let created = store.get(&s, "exe_listing").await.expect("get").unwrap();
    assert_eq!(
        created.status,
        Status::Created,
        "[{}] new row",
        backend.name()
    );

    let token = lease(store.as_ref(), &s, "exe_listing").await;
    let running = ExecutionListing::new(Status::Running, Some(micros(1)), None)
        .with_next_wake_at(Some(micros(3)));
    let version = commit_listing(
        store.as_ref(),
        &s,
        "exe_listing",
        created.version,
        token,
        running,
    )
    .await;
    assert_eq!(
        store.get(&s, "exe_listing").await.unwrap().unwrap().status,
        Status::Running,
        "[{}] status column must follow the commit",
        backend.name()
    );
    assert!(
        worker
            .list_due_timers(micros(4), None, 100)
            .await
            .expect("due scan")
            .iter()
            .any(|row| row.record.id == "exe_listing"),
        "[{}] a running execution with a due timer is in the due scan",
        backend.name()
    );

    let failed = ExecutionListing::new(Status::Failed, Some(micros(1)), Some(micros(2)))
        .with_next_wake_at(Some(micros(3)));
    commit_listing(store.as_ref(), &s, "exe_listing", version, token, failed).await;
    let page = store
        .list_history(&s, &whole_history())
        .await
        .expect("history");
    let summary = page
        .items
        .iter()
        .find(|item| item.id == "exe_listing")
        .unwrap_or_else(|| panic!("[{}] history lists the execution", backend.name()));
    assert_eq!(summary.status, Status::Failed, "[{}]", backend.name());
    assert_eq!(summary.workflow_id, "wf_listing");
    assert_eq!(
        summary.started_at,
        Some(MicrosInstant::floor(micros(1))),
        "[{}] started_at",
        backend.name()
    );
    assert_eq!(
        summary.finished_at,
        Some(MicrosInstant::floor(micros(2))),
        "[{}] finished_at",
        backend.name()
    );
    assert!(
        !worker
            .list_due_timers(micros(4), None, 100)
            .await
            .expect("due scan")
            .iter()
            .any(|row| row.record.id == "exe_listing"),
        "[{}] a terminal execution leaves the due scan",
        backend.name()
    );
}

/// Due timers are listed across scopes in `(next_wake_at, id)` order, at or
/// before the due instant only, and page by keyset without gaps or repeats.
/// Rows without a timer, with a future timer, or terminal are never listed.
pub(crate) async fn assert_due_timers_page_in_wake_order(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let (a, b) = (scope_a(), scope_b());
    seed_workflow(backend, &a, "wf_due").await;
    seed_workflow(backend, &b, "wf_due").await;
    // (scope, id, status, timer)
    let rows = [
        (&a, "exe_due_1", Status::Running, Some(micros(5))),
        (&b, "exe_due_2", Status::Paused, Some(micros(5))),
        (&a, "exe_due_3", Status::Running, Some(micros(9))),
        (&b, "exe_due_none", Status::Running, None),
        (&a, "exe_due_later", Status::Running, Some(micros(20))),
        (&b, "exe_due_done", Status::Completed, Some(micros(5))),
    ];
    for (scope, id, status, timer) in rows {
        store
            .create(scope, id, "wf_due", serde_json::json!({}))
            .await
            .expect("create");
        let token = lease(store.as_ref(), scope, id).await;
        let finished = status.is_terminal().then(|| micros(6));
        let listing =
            ExecutionListing::new(status, Some(micros(1)), finished).with_next_wake_at(timer);
        commit_listing(store.as_ref(), scope, id, 0, token, listing).await;
    }

    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .list_due_timers(micros(9), cursor.as_ref(), 2)
            .await
            .expect("due scan");
        assert!(
            page.len() <= 2,
            "[{}] the limit bounds a page",
            backend.name()
        );
        let Some(last) = page.last() else { break };
        cursor = Some(last.cursor());
        listed.extend(
            page.into_iter()
                .filter(|row| row.record.id.starts_with("exe_due_"))
                .map(|row| (row.record.id, row.record.scope, row.next_wake_at)),
        );
    }
    assert_eq!(
        listed,
        vec![
            (
                "exe_due_1".to_owned(),
                a.clone(),
                MicrosInstant::floor(micros(5))
            ),
            ("exe_due_2".to_owned(), b, MicrosInstant::floor(micros(5))),
            ("exe_due_3".to_owned(), a, MicrosInstant::floor(micros(9))),
        ],
        "[{}] due rows in wake order with an id tiebreak, across scopes, \
         including a timer exactly at the due instant",
        backend.name()
    );
}

/// History is newest first with an id tiebreak, pages concatenate to the
/// whole history without gaps or repeats, an execution created while paging
/// never shifts a later page, and every filter narrows exactly.
pub(crate) async fn assert_history_orders_filters_and_pages(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    let ids = ["exe_h1", "exe_h2", "exe_h3", "exe_h4", "exe_h5"];
    seed_workflow(backend, &s, "wf_even").await;
    seed_workflow(backend, &s, "wf_odd").await;
    for (index, id) in ids.iter().enumerate() {
        let workflow = if index % 2 == 0 { "wf_even" } else { "wf_odd" };
        store
            .create(&s, id, workflow, serde_json::json!({}))
            .await
            .expect("create");
    }

    let whole = store
        .list_history(&s, &whole_history())
        .await
        .expect("whole history");
    assert_eq!(
        whole.items.len(),
        ids.len(),
        "[{}] every row",
        backend.name()
    );
    assert!(whole.next_cursor.is_none(), "[{}] one page", backend.name());
    assert!(
        whole.items.windows(2).all(|pair| {
            (pair[0].created_at, pair[0].id.as_str()) > (pair[1].created_at, pair[1].id.as_str())
        }),
        "[{}] strictly newest first with id tiebreak",
        backend.name()
    );

    // Page through two at a time; create a newer execution after page one.
    let two = ExecutionHistoryPageSize::new(2).expect("page size");
    let mut paged = Vec::new();
    let mut query = ExecutionHistoryQuery::new().with_page_size(two);
    let mut inserted = false;
    loop {
        let page = store.list_history(&s, &query).await.expect("page");
        assert!(page.items.len() <= 2);
        paged.extend(page.items.iter().map(|item| item.id.clone()));
        if !inserted {
            store
                .create(&s, "exe_h_late", "wf_even", serde_json::json!({}))
                .await
                .expect("create while paging");
            inserted = true;
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        query = ExecutionHistoryQuery::new()
            .with_page_size(two)
            .with_cursor(cursor);
    }
    let expected: Vec<String> = whole.items.iter().map(|item| item.id.clone()).collect();
    assert_eq!(
        paged,
        expected,
        "[{}] pages concatenate to the snapshot taken before paging",
        backend.name()
    );

    let even = store
        .list_history(&s, &whole_history().with_workflow("wf_even"))
        .await
        .expect("workflow filter");
    assert_eq!(
        even.items.len(),
        4,
        "[{}] workflow filter: three originals plus the late one",
        backend.name()
    );
    assert!(even.items.iter().all(|item| item.workflow_id == "wf_even"));

    let token = lease(store.as_ref(), &s, "exe_h2").await;
    commit_listing(
        store.as_ref(),
        &s,
        "exe_h2",
        0,
        token,
        ExecutionListing::new(Status::Failed, None, Some(micros(5))),
    )
    .await;
    let failed = store
        .list_history(
            &s,
            &whole_history().with_statuses([Status::Failed, Status::TimedOut].into()),
        )
        .await
        .expect("status filter");
    let failed_ids: Vec<&str> = failed.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(failed_ids, ["exe_h2"], "[{}] status filter", backend.name());
    let active = store
        .list_history(
            &s,
            &whole_history().with_statuses(ExecutionStatusSet::ACTIVE),
        )
        .await
        .expect("active filter");
    assert_eq!(
        active.items.len(),
        ids.len(),
        "[{}] six rows minus the failed one",
        backend.name()
    );

    // Half-open creation range around the oldest row.
    let oldest = whole.items.last().expect("rows").clone();
    let from_oldest = store
        .list_history(
            &s,
            &whole_history().with_created_after(oldest.created_at.to_datetime()),
        )
        .await
        .expect("created_after");
    assert!(
        from_oldest.items.iter().any(|item| item.id == oldest.id),
        "[{}] created_after is inclusive",
        backend.name()
    );
    let before_oldest = store
        .list_history(
            &s,
            &whole_history().with_created_before(oldest.created_at.to_datetime()),
        )
        .await
        .expect("created_before");
    assert!(
        before_oldest
            .items
            .iter()
            .all(|item| item.created_at < oldest.created_at),
        "[{}] created_before is exclusive",
        backend.name()
    );
}

/// History never shows another tenant's executions, and a cursor built from
/// one tenant's rows pages only the caller's tenant.
pub(crate) async fn assert_history_is_scope_isolated(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    seed_workflow(backend, &scope_a(), "wf_1").await;
    seed_workflow(backend, &scope_b(), "wf_1").await;
    store
        .create(&scope_a(), "exe_iso_a", "wf_1", serde_json::json!({}))
        .await
        .expect("create a");
    store
        .create(&scope_b(), "exe_iso_b", "wf_1", serde_json::json!({}))
        .await
        .expect("create b");
    let b = store
        .list_history(&scope_b(), &whole_history())
        .await
        .expect("history b");
    let b_ids: Vec<&str> = b.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        b_ids,
        ["exe_iso_b"],
        "[{}] tenant b sees only b",
        backend.name()
    );

    let a = store
        .list_history(&scope_a(), &whole_history())
        .await
        .expect("history a");
    let a_row = a
        .items
        .iter()
        .find(|item| item.id == "exe_iso_a")
        .expect("a row");
    let replayed = store
        .list_history(
            &scope_b(),
            &whole_history().with_cursor(ExecutionHistoryCursor::new(
                MicrosInstant::floor(a_row.created_at.to_datetime() + chrono::Duration::days(1)),
                "exe_zzz",
            )),
        )
        .await
        .expect("foreign cursor");
    assert!(
        replayed.items.iter().all(|item| item.id != "exe_iso_a"),
        "[{}] a cursor never crosses tenants",
        backend.name()
    );
}
