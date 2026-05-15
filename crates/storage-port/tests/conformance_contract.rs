//! Trait-level contract guards.
//!
//! These pin the *contract* before any real backend exists: a stale fencing
//! token must yield `FencedOut`, and a cross-scope `get` must yield
//! `Ok(None)` (the row's existence never leaks across tenants). Real
//! backends are verified by the behavioral conformance matrix in
//! `crates/storage/tests/`.

use std::time::Duration;

use nebula_storage_port::dto::ExecutionRecord;
use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{FencingToken, Scope, StorageError, TransitionBatch, TransitionOutcome};

/// A reference store: every `commit` is fenced out, and `get` only returns a
/// row when the scope matches the one it was constructed for.
#[derive(Debug)]
struct StubExecutionStore {
    owner_scope: Scope,
}

#[async_trait::async_trait]
impl ExecutionStore for StubExecutionStore {
    async fn create(
        &self,
        _scope: &Scope,
        _id: &str,
        _workflow_id: &str,
        _initial_state: serde_json::Value,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError> {
        if scope != &self.owner_scope {
            // Cross-scope miss: never leak the row, never surface
            // ScopeViolation as an existence oracle.
            return Ok(None);
        }
        Ok(Some(ExecutionRecord {
            id: id.to_string(),
            workflow_id: "wf".into(),
            scope: self.owner_scope.clone(),
            version: 0,
            status: "Running".into(),
            state: serde_json::json!({}),
            lease_holder: None,
            fencing: None,
            created_at: "2026-05-15T00:00:00Z".into(),
            updated_at: "2026-05-15T00:00:00Z".into(),
        }))
    }

    async fn commit(&self, _batch: TransitionBatch) -> Result<TransitionOutcome, StorageError> {
        Ok(TransitionOutcome::FencedOut)
    }

    async fn acquire_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _holder: &str,
        _ttl: Duration,
    ) -> Result<Option<FencingToken>, StorageError> {
        Ok(None)
    }

    async fn renew_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _token: FencingToken,
        _ttl: Duration,
    ) -> Result<bool, StorageError> {
        Ok(false)
    }

    async fn release_lease(
        &self,
        _scope: &Scope,
        _id: &str,
        _token: FencingToken,
    ) -> Result<bool, StorageError> {
        Ok(false)
    }

    async fn list_running(&self, _scope: &Scope) -> Result<Vec<String>, StorageError> {
        Ok(vec![])
    }

    async fn list_running_for_workflow(
        &self,
        _scope: &Scope,
        _workflow_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        Ok(vec![])
    }

    async fn count(&self, _scope: &Scope, _workflow_id: Option<&str>) -> Result<u64, StorageError> {
        Ok(0)
    }
}

#[tokio::test]
async fn stale_fencing_token_commit_is_fenced_out() {
    let store = StubExecutionStore {
        owner_scope: Scope::new("ws_a", "org_a"),
    };
    let batch = TransitionBatch::builder()
        .scope(Scope::new("ws_a", "org_a"))
        .execution_id("exe_1")
        .expected_version(0)
        .fencing(FencingToken::from_generation(1))
        .new_state(serde_json::json!({"s": "running"}))
        .build()
        .expect("valid batch");
    let outcome = store.commit(batch).await.expect("commit returns");
    assert_eq!(outcome, TransitionOutcome::FencedOut);
}

#[tokio::test]
async fn cross_scope_get_returns_none() {
    let store = StubExecutionStore {
        owner_scope: Scope::new("ws_a", "org_a"),
    };
    // Same scope ⇒ row visible.
    let hit = store
        .get(&Scope::new("ws_a", "org_a"), "exe_1")
        .await
        .expect("get returns");
    assert!(hit.is_some());

    // Different scope ⇒ Ok(None), not the row, not an error.
    let miss = store
        .get(&Scope::new("ws_b", "org_b"), "exe_1")
        .await
        .expect("get returns");
    assert!(miss.is_none(), "cross-scope get must not leak the row");
}
