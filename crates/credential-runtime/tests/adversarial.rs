//! Adversarial integration suite — one test per abuse invariant from the
//! credential-runtime subsystem design spec §6, each asserted **end-to-end
//! through the public [`CredentialService`] surface** (no crate internals).
//!
//! The spec's §6 fixes are *structural*; these tests are the behavioural
//! proof that the structure holds at the facade boundary. Where an abuse
//! case is enforced one layer down (the `PendingStateStore` 4-D binding,
//! the engine `LeaseLifecycle` scan), the test asserts what the facade
//! *can* prove and the doc comment states what is delegated and where it
//! is covered — kept honest about the boundary.

use nebula_credential::store::{CredentialStore, StoreError};
use nebula_credential_runtime::test_support::{
    in_memory_service, service_and_raw_store_with_audit_sink,
};
use nebula_credential_runtime::{CredentialServiceError, TenantScope};
use nebula_storage::credential::{AuditEvent, AuditSink};
use serde_json::json;
use std::sync::Arc;

/// Abuse #1 — confused deputy / cross-tenant access.
///
/// A credential created under scope A must be **completely invisible** to
/// scope B on every read/mutate/lifecycle op: each returns `NotFound`,
/// never an error that reveals the row exists in another tenant (no
/// existence leak — the composite owner-scoped key makes a foreign id
/// indistinguishable from a missing one).
#[tokio::test]
async fn abuse1_cross_tenant_is_uniformly_not_found_no_existence_leak() {
    let svc = in_memory_service();
    let a = TenantScope::new("orgA", "wsA");
    let b = TenantScope::new("orgB", "wsB");

    svc.create(
        &a,
        "bearer_token",
        json!({ "token": "sk-tenant-A" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create under A");
    let id = svc.list(&a).await.expect("list A")[0].id.clone();

    // get / update / delete / test / refresh / revoke under B: every one
    // is NotFound (not VersionConflict, not CapabilityUnsupported — those
    // would each leak that the id exists).
    assert!(matches!(
        svc.get(&b, &id).await.expect_err("get B denied"),
        CredentialServiceError::NotFound { .. }
    ));
    assert!(matches!(
        svc.update(
            &b,
            &id,
            Some(json!({ "token": "z" })),
            Some(1),
            nebula_credential::CredentialDisplay::default()
        )
        .await
        .expect_err("update B denied"),
        CredentialServiceError::NotFound { .. }
    ));
    assert!(matches!(
        svc.test(&b, &id).await.expect_err("test B denied"),
        CredentialServiceError::NotFound { .. }
    ));
    assert!(matches!(
        svc.refresh(&b, &id).await.expect_err("refresh B denied"),
        CredentialServiceError::NotFound { .. }
    ));
    assert!(matches!(
        svc.revoke(&b, &id).await.expect_err("revoke B denied"),
        CredentialServiceError::NotFound { .. }
    ));
    assert!(matches!(
        svc.delete(&b, &id).await.expect_err("delete B denied"),
        CredentialServiceError::NotFound { .. }
    ));

    // B's list never sees A's credential; A still owns it.
    assert!(svc.list(&b).await.expect("list B").is_empty());
    assert_eq!(svc.list(&a).await.expect("list A").len(), 1);
}

/// Abuse #2 — schema-bypass / `$expr` injection (credential secrecy).
///
/// A `{"$expr": ..}` envelope survives schema validation but the typed
/// `serde_json::from_value` round-trip refuses it, so a credential secret
/// can never be made to depend on workflow expression state. The
/// well-formed control proves the pipeline accepts a legitimate payload.
#[tokio::test]
async fn abuse2_expr_injection_is_validation_failed_control_succeeds() {
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");

    let err = svc
        .create(
            &scope,
            "bearer_token",
            json!({ "token": { "$expr": "{{ $execution.id }}" } }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect_err("$expr envelope must be refused");
    assert!(
        matches!(err, CredentialServiceError::ValidationFailed { .. }),
        "expected ValidationFailed, got {err:?}"
    );

    // Control: a well-formed create on the same type succeeds.
    let head = svc
        .create(
            &scope,
            "bearer_token",
            json!({ "token": "sk-well-formed" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("well-formed create succeeds");
    assert_eq!(head.credential_key, "bearer_token");
}

/// Abuse #3 — secret echo in responses.
///
/// The facade's management-plane response type is `CredentialHead`, which
/// is secret-free **by construction**: it is projected from the stored row
/// without ever reading `StoredCredential::data`, so there is no secret
/// field to redact — `Debug` and `Serialize` structurally cannot echo the
/// material. (The scheme-bearing `CredentialSnapshot` stays on the
/// execution plane and deliberately does not implement `Serialize`; that
/// property is asserted by the `tests/compile_fail` probe
/// `snapshot_not_serialize.rs`.)
#[tokio::test]
async fn abuse3_no_secret_in_head_debug_or_serialize_on_create_and_get() {
    const SECRET: &str = "sk-do-not-leak-7f3a";
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");

    let created = svc
        .create(
            &scope,
            "bearer_token",
            json!({ "token": SECRET }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect("create ok");
    let created_dbg = format!("{created:?}");
    assert!(
        !created_dbg.contains(SECRET),
        "created head Debug leaked the secret"
    );
    let created_json = serde_json::to_string(&created).expect("head serializes");
    assert!(
        !created_json.contains(SECRET),
        "created head Serialize leaked the secret"
    );

    let fetched = svc.get(&scope, &created.id).await.expect("get ok");
    let fetched_dbg = format!("{fetched:?}");
    assert!(
        !fetched_dbg.contains(SECRET),
        "fetched head Debug leaked the secret"
    );
    let fetched_json = serde_json::to_string(&fetched).expect("head serializes");
    assert!(
        !fetched_json.contains(SECRET),
        "fetched head Serialize leaked the secret"
    );
}

/// Abuse #4 — capability-gated dispatch (no SSRF via test/refresh on a
/// type that does not implement the capability).
///
/// The three first-party builtins are static (no `Testable`/`Refreshable`/
/// `Revocable` impl), so closure absence *is* capability absence: `test`/
/// `refresh`/`revoke` are refused with `CapabilityUnsupported` and
/// `continue_resolve` (no `Interactive` impl) likewise — the dispatch
/// never reaches a provider call, so there is no request to forge.
#[tokio::test]
async fn abuse4_static_type_capability_ops_are_unsupported() {
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");
    svc.create(
        &scope,
        "bearer_token",
        json!({ "token": "sk-cap" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create ok");
    let id = svc.list(&scope).await.expect("list")[0].id.clone();

    for (op_name, res) in [
        ("test", svc.test(&scope, &id).await.err()),
        ("refresh", svc.refresh(&scope, &id).await.err()),
        ("revoke", svc.revoke(&scope, &id).await.err()),
    ] {
        match res {
            Some(CredentialServiceError::CapabilityUnsupported { capability, .. }) => {
                assert_eq!(capability, op_name, "wrong capability name for {op_name}");
            },
            other => panic!("expected CapabilityUnsupported for {op_name}, got {other:?}"),
        }
    }

    // continue_resolve is gated session-first: a session-less scope is
    // refused with SessionRequired *before* the capability check — the
    // pending-store (kind, owner, session, token) binding makes a
    // continuation structurally impossible without a session, so the gap
    // is surfaced explicitly rather than collapsing into a misleading
    // ValidationFailed deep in the executor.
    let no_session = svc
        .continue_resolve(
            &scope,
            "bearer_token",
            "irrelevant-token",
            nebula_credential::resolve::UserInput::Poll,
        )
        .await
        .expect_err("session-less continue must be refused");
    assert!(
        matches!(
            no_session,
            CredentialServiceError::SessionRequired { capability } if capability == "continue"
        ),
        "expected SessionRequired(continue), got {no_session:?}"
    );

    // With a session the session gate is passed; the non-interactive
    // builtin then fails the capability gate (no continuation closure).
    let with_session = scope.clone().with_session("sess-cap");
    let cont = svc
        .continue_resolve(
            &with_session,
            "bearer_token",
            "irrelevant-token",
            nebula_credential::resolve::UserInput::Poll,
        )
        .await
        .expect_err("non-interactive continue must be refused");
    assert!(
        matches!(
            cont,
            CredentialServiceError::CapabilityUnsupported { ref capability, .. }
                if capability == "interactive"
        ),
        "expected CapabilityUnsupported(interactive), got {cont:?}"
    );
}

/// Abuse #5 — cross-tenant lease replay / revoke scan.
///
/// `revoke_for_credential` scans namespaced (owner-scoped) lease ids, so a
/// cross-tenant revoke cannot release another tenant's leases. At the
/// facade level the builtins are static + leaseless, so the provable slice
/// is: a cross-tenant `revoke` is `NotFound` (it never reaches the lease
/// scan because the owner check rejects first — same guarantee as #1).
/// The lease-scan namespacing itself is exercised by the engine
/// `LeaseLifecycle` tests; here we pin the facade contract that ties the
/// revoke entry point to the owner-scoped id.
#[tokio::test]
async fn abuse5_cross_tenant_revoke_is_not_found_before_lease_scan() {
    let svc = in_memory_service();
    let owner = TenantScope::new("orgX", "wsX");
    let attacker = TenantScope::new("orgY", "wsY");

    svc.create(
        &owner,
        "bearer_token",
        json!({ "token": "sk-lease" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create ok");
    let id = svc.list(&owner).await.expect("list")[0].id.clone();

    // The attacker's revoke is NotFound — the owner gate runs before the
    // capability gate and before any lease release, so a foreign caller
    // cannot drive `revoke_for_credential` against this id at all.
    assert!(matches!(
        svc.revoke(&attacker, &id).await.expect_err("denied"),
        CredentialServiceError::NotFound { .. }
    ));
}

/// Abuse #6 — pending-token hijack.
///
/// The general `PendingStateStore` inherits the OAuth pending-token
/// guarantees: unguessable + single-use + TTL + bound to the 4-D
/// `(kind, owner, session, token)` tuple in `PendingStateStore::consume`
/// (covered by nebula-credential / storage tests). The invariant under
/// test at the facade is: **a forged continuation token never yields an
/// `Acquisition::Complete`**, on both sides of the session gate —
///
/// - session-less scope → refused with `SessionRequired` *before* the
///   token is ever consulted (the continuation is structurally
///   impossible without the session half of the 4-D binding);
/// - with a session, for the non-interactive builtin the next gate is
///   `CapabilityUnsupported` (no continuation closure) — still never a
///   `Complete`. (A forged token reaching a genuinely interactive type's
///   `PendingStateStore::consume` is rejected one layer down.)
#[tokio::test]
async fn abuse6_forged_pending_token_never_resolves() {
    let svc = in_memory_service();
    let no_session = TenantScope::new("org1", "ws1");
    let with_session = no_session.clone().with_session("sess-6");

    for forged in ["", "garbage", "{\"not\":\"a token\"}", "../../etc/passwd"] {
        // Session-less: the session gate refuses before the token matters.
        match svc
            .continue_resolve(
                &no_session,
                "bearer_token",
                forged,
                nebula_credential::resolve::UserInput::Poll,
            )
            .await
        {
            Err(CredentialServiceError::SessionRequired { capability }) => {
                assert_eq!(capability, "continue");
            },
            Ok(acq) => panic!("forged token {forged:?} must never resolve, got {acq:?}"),
            other => panic!("session-less forged {forged:?}: expected SessionRequired, {other:?}"),
        }

        // With a session: past the session gate, the non-interactive
        // builtin has no continuation closure → CapabilityUnsupported.
        // Either way, never an `Acquisition::Complete`.
        match svc
            .continue_resolve(
                &with_session,
                "bearer_token",
                forged,
                nebula_credential::resolve::UserInput::Poll,
            )
            .await
        {
            Err(
                CredentialServiceError::CapabilityUnsupported { .. }
                | CredentialServiceError::ValidationFailed { .. }
                | CredentialServiceError::PendingExpired,
            ) => {},
            Ok(acq) => panic!("forged token {forged:?} must never resolve, got {acq:?}"),
            other => panic!("unexpected error class for forged token {forged:?}: {other:?}"),
        }
    }
}

/// Abuse #7 — plaintext-at-rest impossible / raw store uncomposable.
///
/// This invariant's primary proof is the **compile-fail probe**
/// (`tests/compile_fail/raw_store_without_layers.rs`): `CredentialService`
/// cannot be constructed bypassing the builder, so the
/// `Audit(Cache(Encryption(raw)))` composition (Encryption adjacent to the
/// backend ⇒ ciphertext at rest) is the only construction path. The stored
/// bytes are not assertable through the public API by design — `get`
/// returns a secret-free [`CredentialSnapshot`], never the row — so the
/// runtime slice here is the behavioural complement: a created credential
/// is retrievable and correctly projected (the encryption layer round-trips
/// transparently), and its Debug never carries the plaintext. The
/// "ciphertext at rest" guarantee itself is structural (compile-fail probe
/// + the crate-private `LayeredStore` type), not facade-observable.
#[tokio::test]
async fn abuse7_layered_store_roundtrips_without_exposing_plaintext() {
    const SECRET: &str = "sk-at-rest-c0ffee";
    let svc = in_memory_service();
    let scope = TenantScope::new("org1", "ws1");

    svc.create(
        &scope,
        "bearer_token",
        json!({ "token": SECRET }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create ok");
    let id = svc.list(&scope).await.expect("list")[0].id.clone();

    // Round-trips through Encryption(raw) transparently and the projected
    // snapshot never carries the plaintext. (Ciphertext-at-rest itself is
    // proven structurally by the compile-fail probe — the raw store can
    // never be composed without the EncryptionLayer.)
    let got = svc.get(&scope, &id).await.expect("get ok");
    assert_eq!(got.credential_key, "bearer_token");
    assert!(
        !format!("{got:?}").contains(SECRET),
        "head must not carry the plaintext secret"
    );
}

/// Audit sink that refuses every event — drives the fail-closed path.
#[derive(Debug)]
struct FailingAuditSink;

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: &AuditEvent) -> Result<(), StoreError> {
        Err(StoreError::AuditFailure("audit sink down".to_owned()))
    }
}

/// Abuse #8 — audit fail-closed.
///
/// A refusing `AuditSink` makes the wrapping `AuditLayer` fail the whole
/// store operation with `StoreError::AuditFailure` (no discard-and-log),
/// surfaced by the facade as `CredentialServiceError::Store`. The write
/// must **not partially land** — fail-closed, never log-and-continue.
///
/// Note on what the facade can prove here: with a hard-down sink *every*
/// store op (`put`, `get`, `list`) through the layered stack also fails
/// closed, so the row cannot be read back *through the facade* — that is
/// itself the fail-closed property, but it means "row absent" must be
/// asserted against the raw inner store, which the `test_support`
/// read-back seam exposes (a `Clone` sharing the service's backing map,
/// bypassing the poisoned `AuditLayer`). This proves the `AuditLayer`
/// `CreateOnly` rollback (delete-on-sink-refusal) actually executed.
#[tokio::test]
async fn abuse8_audit_refusal_fails_closed_no_partial_write() {
    let (svc, raw_store) = service_and_raw_store_with_audit_sink(Arc::new(FailingAuditSink));
    let scope = TenantScope::new("org1", "ws1");

    let err = svc
        .create(
            &scope,
            "bearer_token",
            json!({ "token": "sk-audit" }),
            nebula_credential::CredentialDisplay::default(),
        )
        .await
        .expect_err("create must fail when the audit sink refuses");
    assert!(
        matches!(err, CredentialServiceError::Store(_)),
        "expected Store (audit fail-closed), got {err:?}"
    );

    // The credential row did not land — the audit refusal triggered the
    // AuditLayer CreateOnly rollback, so the raw inner store (read
    // directly, bypassing the poisoned audit layer) holds no row.
    let raw_ids = raw_store
        .list(None)
        .await
        .expect("raw inner store list (no audit layer) ok");
    assert!(
        raw_ids.is_empty(),
        "audit-refused create must not leave a partial row in the raw store, found {raw_ids:?}"
    );

    // And the facade itself is fully fail-closed: a read through the
    // poisoned audit layer also errors (never silently succeeds).
    assert!(
        matches!(
            svc.list(&scope).await,
            Err(CredentialServiceError::Store(_))
        ),
        "list through a poisoned audit sink must also fail closed"
    );
}
