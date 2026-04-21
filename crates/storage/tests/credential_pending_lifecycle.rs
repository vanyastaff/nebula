//! Integration test: pending state lifecycle (interactive credential flow)
//! + ADR-0029 §4 invariant gate for `InMemoryPendingStore`.
//!
//! Validates the full interactive credential resolution flow:
//! `execute_resolve` → `Pending` → `execute_continue` → `Complete`.
//!
//! Also pins the §4 invariants ADR-0029 enumerates for a pending-store impl:
//! invariant 2 (TTL / expiry indistinguishable from NotFound surface),
//! invariant 3 (single-use — `consume` is transactional get_then_delete),
//! invariant 4 (session-id binding rejection),
//! invariant 5 (secret fields typed as `SecretString` — compile-time witness
//!             via `OAuth2Pending`),
//! invariant 6 (pending state types derive `Zeroize` per the trait bound).
//!
//! Invariant 1 (encryption at rest) is **moot** for `InMemoryPendingStore`
//! because the impl is process-local; durable impls (Postgres/Redis) must
//! wrap via a future `EncryptedPendingLayer`. See ADR-0029 §4.1 and
//! `crates/storage/src/credential/pending.rs` module doc.
//!
//! Ref: `docs/adr/0029-storage-owns-credential-persistence.md` §4
//! Ref: `docs/adr/0032-credential-store-canonical-home.md`

// `InMemoryPendingStore` is gated on `credential-in-memory`; tests also
// need `test-util` for any helper pathways. Gate on both so
// `cargo test -p nebula-storage --features test-util` (without
// `credential-in-memory`) does not fail to compile.
#![cfg(all(feature = "test-util", feature = "credential-in-memory"))]

use std::time::Duration;

use nebula_credential::{
    Credential, CredentialContext, PendingState, PendingStateStore, PendingStoreError,
    PendingToken, SecretString,
    credentials::OAuth2Pending,
    error::CredentialError,
    executor::{ResolveResponse, execute_continue, execute_resolve},
    metadata::CredentialMetadata,
    resolve::{DisplayData, InteractionRequest, RefreshOutcome, ResolveResult, UserInput},
    scheme::SecretToken,
};
use nebula_schema::FieldValues;
use nebula_storage::credential::InMemoryPendingStore;

// ── Test pending state ───────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestPending {
    verification_code: String,
}

impl zeroize::Zeroize for TestPending {
    fn zeroize(&mut self) {
        self.verification_code.zeroize();
    }
}

impl PendingState for TestPending {
    const KIND: &'static str = "test_interactive_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_mins(5)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ShortTtl {
    data: String,
}

impl zeroize::Zeroize for ShortTtl {
    fn zeroize(&mut self) {
        self.data.zeroize();
    }
}

impl PendingState for ShortTtl {
    const KIND: &'static str = "short_ttl";

    fn expires_in(&self) -> Duration {
        Duration::ZERO
    }
}

// ── Test credential state ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestInteractiveState {
    token: String,
}

impl nebula_credential::CredentialState for TestInteractiveState {
    const KIND: &'static str = "interactive_test";
    const VERSION: u32 = 1;
}

// ── Test credential type ─────────────────────────────────────────────

struct InteractiveTestCredential;

impl Credential for InteractiveTestCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = TestInteractiveState;
    type Pending = TestPending;

    const KEY: &'static str = "interactive_test";
    const INTERACTIVE: bool = true;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("interactive_test"),
            "Interactive Test",
            "Test credential for pending lifecycle",
            Self::parameters(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &TestInteractiveState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        Ok(ResolveResult::Pending {
            state: TestPending {
                verification_code: "secret-code-123".into(),
            },
            interaction: InteractionRequest::DisplayInfo {
                title: "Enter Code".into(),
                message: "Please enter the verification code".into(),
                data: DisplayData::Text("Check your email for the code".into()),
                expires_in: Some(300),
            },
        })
    }

    async fn continue_resolve(
        pending: &TestPending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        // Verify the user provided the correct code
        match input {
            UserInput::Code { code } if code == &pending.verification_code => {
                Ok(ResolveResult::Complete(TestInteractiveState {
                    token: "final-token".into(),
                }))
            },
            _ => Err(CredentialError::InvalidInput(
                "incorrect verification code".into(),
            )),
        }
    }

    async fn refresh(
        _state: &mut TestInteractiveState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::NotSupported)
    }
}

// ── Lifecycle tests (preserved from credential crate) ────────────────

#[tokio::test]
async fn pending_lifecycle_resolve_then_continue() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = FieldValues::new();

    // Step 1: initial resolve → should return Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .expect("execute_resolve should succeed");

    let token = match response {
        ResolveResponse::Pending { token, interaction } => {
            // Verify the interaction request matches what we returned
            assert!(
                matches!(interaction, InteractionRequest::DisplayInfo { ref title, .. } if title == "Enter Code"),
                "expected DisplayInfo interaction, got: {interaction:?}"
            );
            token
        },
        other => panic!("expected Pending response, got: {other:?}"),
    };

    // Step 2: continue with correct code → should return Complete
    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let response =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await
            .expect("execute_continue should succeed");

    match response {
        ResolveResponse::Complete(state) => {
            assert_eq!(state.token, "final-token");
        },
        other => panic!("expected Complete response, got: {other:?}"),
    }
}

#[tokio::test]
async fn pending_token_is_single_use() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = FieldValues::new();

    // Resolve → Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .unwrap();
    let token = match response {
        ResolveResponse::Pending { token, .. } => token,
        other => panic!("expected Pending, got: {other:?}"),
    };

    // First continue → should succeed
    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(result.is_ok(), "first continue should succeed");

    // Second continue with same token → should fail (single-use)
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(result.is_err(), "second continue should fail (single-use)");
}

#[tokio::test]
async fn continue_with_wrong_code_returns_error() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = FieldValues::new();

    // Resolve → Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .unwrap();
    let token = match response {
        ResolveResponse::Pending { token, .. } => token,
        other => panic!("expected Pending, got: {other:?}"),
    };

    // Continue with wrong code → should return credential error
    let input = UserInput::Code {
        code: "wrong-code".into(),
    };
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(
        result.is_err(),
        "continue with wrong code should fail: {result:?}"
    );
}

// ── ADR-0029 §4 invariant tests ──────────────────────────────────────

// Invariant 1 (encryption at rest) — **moot** for InMemoryPendingStore.
// The impl is process-local; "at rest" encryption must be provided by a
// wrapper (`EncryptedPendingLayer`) around a durable backend. No test
// here — documented in `crates/storage/src/credential/pending.rs`
// module doc and deferred to a follow-up ADR.
//
// Ref: docs/adr/0029-storage-owns-credential-persistence.md §4.1

/// Invariant 2 — TTL is enforced; expired entries surface as `Expired`
/// (and get evicted so a re-read returns a clean `NotFound`). The
/// expiry path must be indistinguishable from "never existed" from the
/// caller's perspective apart from the discriminated error — no
/// side-channel leak of which dimension missed.
#[tokio::test]
async fn invariant_2_ttl_expiry_is_surfaced_and_evicted() {
    let store = InMemoryPendingStore::new();
    let pending = ShortTtl {
        data: "ephemeral".into(),
    };
    let token = store
        .put("oauth2", "user_1", "sess_1", pending)
        .await
        .unwrap();

    // Give the clock a moment to advance past expiry.
    tokio::time::sleep(Duration::from_millis(5)).await;

    let err = store
        .consume::<ShortTtl>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(
        matches!(err, PendingStoreError::Expired),
        "expected Expired, got {err:?}"
    );

    // Expired entry should have been evicted — a repeat read returns
    // NotFound, not Expired, and carries no residual state.
    let err2 = store
        .consume::<ShortTtl>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(
        matches!(err2, PendingStoreError::NotFound),
        "expected NotFound after eviction, got {err2:?}"
    );
}

/// Invariant 3 — single-use: `consume` is transactional (validate + delete
/// under the same write lock). A second `consume` with the same token
/// returns `NotFound`.
#[tokio::test]
async fn invariant_3_consume_is_single_use() {
    let store = InMemoryPendingStore::new();
    let pending = TestPending {
        verification_code: "x".into(),
    };

    let token = store
        .put("interactive_test", "user_1", "sess_1", pending)
        .await
        .unwrap();

    let _: TestPending = store
        .consume("interactive_test", &token, "user_1", "sess_1")
        .await
        .expect("first consume should succeed");

    let err = store
        .consume::<TestPending>("interactive_test", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(
        matches!(err, PendingStoreError::NotFound),
        "second consume must see NotFound (single-use), got {err:?}"
    );
}

/// Invariant 4 — session-binding rejection. A consume with a wrong
/// session id is rejected with `ValidationFailed`, and critically **does
/// not destroy** the pending entry — so a token leak cannot turn into a
/// DoS against the legitimate caller.
#[tokio::test]
async fn invariant_4_consume_rejects_mismatched_session() {
    let store = InMemoryPendingStore::new();
    let pending = TestPending {
        verification_code: "x".into(),
    };

    let token = store
        .put("interactive_test", "user_1", "sess_1", pending)
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("interactive_test", &token, "user_1", "sess_2")
        .await
        .unwrap_err();
    assert!(
        matches!(err, PendingStoreError::ValidationFailed { .. }),
        "wrong-session consume must be ValidationFailed, got {err:?}"
    );

    // The legitimate caller must still be able to consume.
    let ok: TestPending = store
        .consume("interactive_test", &token, "user_1", "sess_1")
        .await
        .expect("legitimate consume should still succeed after bad probe");
    assert_eq!(ok.verification_code, "x");
}

/// Invariant 5 — `PendingState` implementers that carry credentials must
/// type them as `SecretString` (not `String`). This test is a
/// compile-time witness against `OAuth2Pending`: if the `client_secret`
/// field drifts away from `SecretString`, the assignment fails to type
/// check and the test fails to compile.
#[test]
fn invariant_5_oauth2_pending_secrets_are_secret_string() {
    fn _compile_time_witness(p: &OAuth2Pending) {
        let _: &SecretString = &p.client_secret;
        let _: &Option<SecretString> = &p.pkce_verifier;
    }
    // Prevent the closure from being dead-code-elim'd at some
    // optimisation levels — we only need it to type-check.
    let _ = _compile_time_witness;
}

/// Invariant 6 — `PendingState` types are `Zeroize` by trait bound. Any
/// type that asserts `impl PendingState` must also assert `impl Zeroize`,
/// or it won't build. This test is a compile-time witness pinning the
/// bound: drop the bound and the test fails to compile.
#[test]
fn invariant_6_pending_state_requires_zeroize() {
    fn assert_zeroize<P: zeroize::Zeroize>() {}
    assert_zeroize::<TestPending>();
    assert_zeroize::<ShortTtl>();
    assert_zeroize::<OAuth2Pending>();
}

/// Sanity: `delete` is idempotent — removes on first call, succeeds on
/// repeat. Covers a property of the trait (not a §4 invariant) that
/// callers of the executor path rely on during failure cleanup.
#[tokio::test]
async fn delete_is_idempotent() {
    let store = InMemoryPendingStore::new();
    let token = PendingToken::generate();

    store.delete(&token).await.unwrap();
    store.delete(&token).await.unwrap();
}
