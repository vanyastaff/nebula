#![cfg(feature = "rotation")]

//! Per-slot rotation observability redaction CI gate (PRODUCT_CANON ,
//! ).
//!
//! This is the resource-rotation analogue of the `token_refresh`
//! redaction gate in `credential_refresh_redaction.rs`: the same
//! inject-a-secret → capture-everything → assert-no-leak shape, applied to
//! the engine per-slot rotation fan-out
//! (`ResourceFanoutIndex::{dispatch_refresh, dispatch_revoke}` →
//! `Manager::{refresh_slot_for, revoke_slot_for}`).
//!
//! The capture buffer mirrors the `CaptureBuf` + `tracing-subscriber`
//! `MakeWriter` harness from `crates/credential/tests/redaction.rs`: a
//! thread-local subscriber records **every** span and event on the
//! calling thread at `TRACE`, so both the resource-side
//! `nebula.resource.slot_{refresh,revoke}` span (and its `error = %e` warn)
//! and the fan-out `nebula.credential.rotation.fanout_*` spans are
//! captured. On top
//! of the log buffer this test also drains the `ResourceEvent` broadcast
//! sink (Debug + Display of every emitted slot event) and renders the full
//! Prometheus text exposition (every `*_ATTEMPTS_TOTAL{outcome=…}` series
//! with its label set) so all four observability surfaces named in
//! — spans, domain events, metric labels, error strings — are
//! inspected against the same injected secret.
//!
//! A distinctive secret literal is planted both in the slot's
//! `CredentialGuard` material and in the live `Runtime` the rotation hooks
//! borrow, so if any of those surfaces were to render the credential the
//! capture would contain the secret substring. The invariant: only
//! key / slot / scope / slot_identity (a `u64`) / topology / durations /
//! counts reach observability — never credential material.
//!
//! The fan-out is driven through BOTH a successful row (→ `SlotRefreshed`
//! / `SlotRevoked`, success metric label, the slot-refresh + fan-out
//! spans) and a deliberately-failing row (→ `SlotRefreshFailed` /
//! `SlotRevokeFailed`, the `error = %e` span field, the failed metric
//! label). The failing hook's error message is credential-free by
//! construction — this gate proves the *rotation/revoke path* never
//! injects the credential it handled into any captured surface, not the
//! resource author's own error-string discipline.
//!
//! Single-threaded by construction (`#[tokio::test]` current-thread
//! runtime, no `tokio::spawn`): the capture subscriber is installed via
//! `tracing::subscriber::set_default` (the harness's `with_default`
//! variant would force a nested `block_on`, starving the fan-out's
//! `tokio::time::timeout` of a reactor), which is thread-local, and
//! `join_all` polls every per-row dispatch future on the calling thread,
//! so nothing escapes the capture.

use std::io::{self, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialGuard, CredentialId};
use nebula_engine::credential::rotation::{ResourceFanoutIndex, RotationOutcome};
use nebula_resource::{
    AcquireOptions, Manager, ManagerConfig, RegistrationSpec, ResidentConfig, Resource,
    ResourceConfig, ResourceContext, SlotCell, SlotIdentity,
    error::Error as ResourceError,
    events::ResourceEvent,
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;
use zeroize::Zeroize;

/// Distinctive, unlikely-to-coincide token planted in the rotated
/// credential material. Long and structured so a substring match cannot
/// false-positive on incidental output.
const SECRET: &str = "SUPER-SECRET-TOKEN-9d3f";

// ---------------------------------------------------------------------
// Capture buffer + MakeWriter plumbing
//
// Verbatim shape of the `CaptureBuf` harness in
// `crates/credential/tests/redaction.rs` — reused (not re-invented) so
// the span/event capture semantics are identical to the established
// gate.
// ---------------------------------------------------------------------

/// Shared buffer every captured span/event is appended to.
#[derive(Clone, Default)]
struct CaptureBuf(Arc<Mutex<Vec<u8>>>);

impl CaptureBuf {
    fn as_string(&self) -> String {
        let guard = self.0.lock().expect("capture buffer poisoned");
        String::from_utf8_lossy(&guard).into_owned()
    }
}

impl Write for CaptureBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.0.lock().expect("capture buffer poisoned");
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureBuf {
    type Writer = CaptureBuf;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Case-sensitive **and** case-insensitive substring absence assertion.
/// requires both: a leak that lower/upper-cases the token in
/// transit is still a leak.
fn assert_no_secret(haystack: &str, surface: &str) {
    assert!(
        !haystack.contains(SECRET),
        "redaction gate violation: secret {SECRET:?} leaked into {surface} \
         (case-sensitive):\n---- {surface} ----\n{haystack}\n------------------"
    );
    assert!(
        !haystack.to_lowercase().contains(&SECRET.to_lowercase()),
        "redaction gate violation: secret {SECRET:?} leaked into {surface} \
         (case-insensitive):\n---- {surface} ----\n{haystack}\n------------------"
    );
}

// ---------------------------------------------------------------------
// Secret-bearing test resource
//
// The rotated credential material lives in BOTH:
//   * the `SlotCell<CredentialGuard<SecretCred>>` field — the resolved
//     credential the engine fan-out conceptually rotates; and
//   * the live `Runtime` the `&self` rotation hooks borrow.
// So every observability surface on the rotation/revoke path handled the
// secret and would contain it if it rendered the credential.
// ---------------------------------------------------------------------

/// Credential plaintext under test. `Zeroize` so it can sit inside a
/// `CredentialGuard` exactly like a real resolved secret.
struct SecretCred(String);

impl Zeroize for SecretCred {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

/// Hook outcome selected per resolved `slot_identity`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Hook returns `Ok` after reading the secret-bearing runtime —
    /// drives `SlotRefreshed`/`SlotRevoked`, the success metric label,
    /// and the slot-refresh + fan-out spans.
    Ok,
    /// Hook returns `Err` with a credential-free message — drives
    /// `SlotRefreshFailed`/`SlotRevokeFailed`, the `error = %e` span
    /// field, and the failed metric label. The message intentionally
    /// carries NO secret: this gate proves the rotation path never
    /// injects the credential it handled, not author error hygiene.
    Err,
}

#[derive(Debug)]
struct HookError(&'static str);
impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for HookError {}
impl From<HookError> for ResourceError {
    fn from(e: HookError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone)]
struct Cfg;
nebula_schema::impl_empty_has_schema!(Cfg);
impl ResourceConfig for Cfg {
    fn validate(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// Live runtime handed by `&Runtime` to the rotation hooks. Holds the
/// secret so the hook genuinely *handles* credential material on the
/// rotation path (a hook that touches the secret then succeeds/fails
/// exercises the real code path that could leak it).
#[derive(Clone)]
struct SecretRuntime {
    secret: String,
}

#[derive(Clone)]
struct SecretBearingResource {
    behaviour: Behaviour,
    /// `#[credential]`-shaped resolved slot, pre-populated with the
    /// secret guard (mirrors the `manager_refresh_slot.rs` fixture).
    /// The dispatch borrows the runtime, not this cell, so it is not
    /// read directly — its presence makes the secret reachable through
    /// the resolved slot the engine rotates.
    #[allow(
        dead_code,
        reason = "models the author-declared resolved SlotCell; rotation dispatch borrows the runtime, not this cell"
    )]
    db: Arc<SlotCell<CredentialGuard<SecretCred>>>,
    /// Bumped whenever a rotation hook actually ran (proves the capture
    /// covers a path that handled the secret, not a no-op).
    hook_entered: Arc<AtomicUsize>,
}

impl Resource for SecretBearingResource {
    type Config = Cfg;
    type Runtime = SecretRuntime;
    type Lease = SecretRuntime;
    type Error = HookError;

    fn key() -> ResourceKey {
        resource_key!("rotation-redaction-res")
    }

    async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<SecretRuntime, HookError> {
        Ok(SecretRuntime {
            secret: SECRET.to_owned(),
        })
    }

    async fn on_credential_refresh(
        &self,
        _slot: &str,
        rt: &SecretRuntime,
    ) -> Result<(), HookError> {
        self.hook_entered.fetch_add(1, Ordering::SeqCst);
        // Genuinely touch the secret-bearing runtime on the rotation
        // path so this is not a no-op the redaction is vacuously true
        // for. The value is intentionally unused beyond the read.
        let _ = rt.secret.len();
        match self.behaviour {
            Behaviour::Ok => Ok(()),
            // Credential-free message by construction.
            Behaviour::Err => Err(HookError("refresh hook rejected: simulated upstream 503")),
        }
    }

    async fn on_credential_revoke(&self, _slot: &str, rt: &SecretRuntime) -> Result<(), HookError> {
        self.hook_entered.fetch_add(1, Ordering::SeqCst);
        let _ = rt.secret.len();
        match self.behaviour {
            Behaviour::Ok => Ok(()),
            Behaviour::Err => Err(HookError("revoke hook rejected: simulated upstream 503")),
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for SecretBearingResource {
    fn is_alive_sync(&self, _rt: &SecretRuntime) -> bool {
        true
    }
}

/// Drains the `ResourceEvent` broadcast sink, returning every event's
/// Debug **and** Display rendering joined into one haystack. Asserts the
/// expected slot variants were observed so the event surface is proven
/// non-empty (not trivially redaction-clean by emitting nothing).
struct EventSink {
    rx: nebula_eventbus::Subscriber<ResourceEvent>,
}

#[derive(Default)]
struct DrainedEvents {
    rendered: String,
    count: usize,
    saw_success_variant: bool,
    saw_failed_variant: bool,
}

impl EventSink {
    fn drain(mut self, want_revoke: bool) -> DrainedEvents {
        let mut d = DrainedEvents::default();
        // The drain MUST be loud on a dropped event: a silent stop on
        // `Lagged` would skip an event that may have carried credential
        // material, yielding a false-clean from this security gate. The
        // broadcast channel is a fixed 256-slot buffer (constructed
        // internally by `Manager`; capacity is not caller-controllable
        // here) and this gate emits ≤2 events per direction, so `Lagged`
        // cannot trigger under load — the `Subscriber` wrapper
        // automatically skips lagged events and continues, so we drain
        // until `None` (empty or closed).
        while let Some(evt) = self.rx.try_recv() {
            // `ResourceEvent` derives `Debug` (no `Display`); the
            // Debug rendering expands every struct field name +
            // value, so a credential in any field — including the
            // `error: String` on the `*Failed` variants — would
            // surface here verbatim.
            d.rendered.push_str(&format!("{evt:?}\n"));
            match (&evt, want_revoke) {
                (ResourceEvent::SlotRefreshed { .. }, false)
                | (ResourceEvent::SlotRevoked { .. }, true) => {
                    d.saw_success_variant = true;
                },
                (ResourceEvent::SlotRefreshFailed { .. }, false)
                | (ResourceEvent::SlotRevokeFailed { .. }, true) => {
                    d.saw_failed_variant = true;
                },
                _ => {},
            }
            d.count += 1;
        }
        assert_eq!(
            self.rx.lagged_count(),
            0,
            "event lag detected — security gate may have missed events"
        );
        d
    }
}

/// Registers `ok_id` (→ `Behaviour::Ok`) and `err_id`
/// (→ `Behaviour::Err`) as two resolved rows of ONE `(key, scope)` with
/// distinct `slot_identity`, warms each resident runtime, binds both
/// into a fresh fan-out index under one `cid`, and returns everything
/// the drive step needs. A real `MetricsRegistry` is wired so the
/// Prometheus snapshot reflects the `outcome`-labeled series.
async fn setup(
    ok_id: SlotIdentity,
    err_id: SlotIdentity,
) -> (
    ResourceFanoutIndex,
    Arc<Manager>,
    CredentialId,
    Arc<AtomicUsize>,
    Arc<nebula_metrics::MetricsRegistry>,
) {
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let mgr = Arc::new(Manager::with_config(ManagerConfig {
        metrics_registry: Some(Arc::clone(&registry)),
        ..ManagerConfig::default()
    }));
    let idx = ResourceFanoutIndex::new();
    let cid = CredentialId::new();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let hook_entered = Arc::new(AtomicUsize::new(0));

    for (id, behaviour) in [(ok_id, Behaviour::Ok), (err_id, Behaviour::Err)] {
        let slot: SlotCell<CredentialGuard<SecretCred>> = SlotCell::empty();
        slot.store(Arc::new(CredentialGuard::new(SecretCred(
            SECRET.to_owned(),
        ))));

        mgr.register(RegistrationSpec {
            resource: SecretBearingResource {
                behaviour,
                db: Arc::new(slot),
                hook_entered: Arc::clone(&hook_entered),
            },
            config: Cfg,
            scope: scope.clone(),
            slot_identity: id.clone(),
            topology: TopologyRuntime::Resident(ResidentRuntime::<SecretBearingResource>::new(
                ResidentConfig::default(),
            )),
            acquire: Manager::erased_acquire_resident_for::<SecretBearingResource>(),
            recovery_gate: None,
        })
        .expect("register resolved-credential row");

        // Resident materializes its shared runtime lazily on first
        // acquire — warm it so the rotation hook has a live `&Runtime`
        // (carrying the secret) to borrow.
        let ctx = ResourceContext::minimal(
            Scope {
                org_id: Some(org),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        let _g = mgr
            .acquire_resident_for_identity::<SecretBearingResource>(
                &ctx,
                &AcquireOptions::default(),
                &id,
            )
            .await
            .expect("warm tenant runtime");

        idx.bind(cid, SecretBearingResource::key(), scope.clone(), "db", id);
    }

    (idx, mgr, cid, hook_entered, registry)
}

/// One direction (refresh or revoke) of the gate: inject the secret,
/// capture every span/event/metric/error produced by driving the
/// fan-out across an OK row and a failing row, then assert the secret
/// leaked into none of them — and that the capture is genuinely
/// non-empty.
async fn run_redaction_gate(want_revoke: bool) {
    // Two distinct resolved-credential rows under one cid — distinct
    // structural identities so the fan-out routes each to its own row.
    let ok_id = SlotIdentity::from_bindings([("db", "cred-ok")]);
    let err_id = SlotIdentity::from_bindings([("db", "cred-err")]);
    let (idx, mgr, cid, hook_entered, registry) = setup(ok_id, err_id).await;

    // Subscribe to the event broadcast *before* driving so no slot
    // event is missed.
    let sink = EventSink {
        rx: mgr.subscribe_events(),
    };

    // Install the thread-local capturing subscriber for the whole drive
    // (same `CaptureBuf` + `tracing-subscriber` shape as
    // `assert_no_secret_in_logs` in the credential redaction harness):
    // TRACE level so DEBUG-level slot/fan-out spans + the `error = %e`
    // warn are all captured.
    //
    // `set_default` (vs the harness's `with_default`) keeps the normal
    // `.await` flow: this is a current-thread `#[tokio::test]` with no
    // `tokio::spawn`, and `join_all` inside `dispatch_*` polls every
    // per-row future on this one thread, so the thread-local subscriber
    // covers every span/event — while the Tokio reactor (needed by the
    // fan-out's `tokio::time::timeout`) stays available (no nested
    // `block_on`). The guard is dropped at end of scope.
    let buf = CaptureBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_target(false)
        .with_level(true)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let capture_guard = tracing::subscriber::set_default(subscriber);

    let outcome: RotationOutcome = if want_revoke {
        idx.dispatch_revoke(cid, &mgr, Duration::from_secs(5)).await
    } else {
        idx.dispatch_refresh(cid, &mgr, Duration::from_secs(5))
            .await
    };

    // Stop capturing once the drive is done; everything below inspects
    // the already-captured buffers.
    drop(capture_guard);

    // Both rows dispatched: the OK row succeeded, the failing row
    // failed. This pins that the gate actually exercised the
    // success-event path AND the failure-event/error path (not a
    // degenerate all-skip).
    assert_eq!(
        outcome,
        RotationOutcome {
            success: 1,
            failed: 1,
            timed_out: 0,
        },
        "gate must drive one successful and one failing resolved row"
    );
    assert_eq!(
        outcome.dispatched(),
        2,
        "both bound resolved rows must be accounted for"
    );
    assert_eq!(
        hook_entered.load(Ordering::SeqCst),
        2,
        "both rotation hooks must have run (and handled the secret-bearing runtime)"
    );

    let logs = buf.as_string();
    let events = sink.drain(want_revoke);
    let metrics = nebula_metrics::snapshot(&registry);

    // ---- Mandatory "captured something" guard ----
    // A redaction test that captures nothing passes trivially and is
    // worthless. Prove every inspected surface is genuinely non-empty
    // and carries the rotation signal before asserting absence.
    // Assert the EXACT fan-out span for the direction under test (or the
    // matching resource-side slot span — `slot_revoke` for revoke,
    // `slot_refresh` for refresh), so a one-sided rename of either
    // direction's span is caught instead of being masked by the other.
    let (expected_fanout_span, expected_resource_span) = if want_revoke {
        (
            "nebula.credential.rotation.fanout_revoke",
            "nebula.resource.slot_revoke",
        )
    } else {
        (
            "nebula.credential.rotation.fanout_refresh",
            "nebula.resource.slot_refresh",
        )
    };
    assert!(
        logs.contains(expected_fanout_span) || logs.contains(expected_resource_span),
        "expected rotation span `{expected_fanout_span}` (or the resource-side \
         `{expected_resource_span}` span) in captured logs — capture-is-real \
         guard, got:\n{logs}"
    );
    assert!(
        events.count >= 1,
        "capture guard: expected ≥1 ResourceEvent, captured none"
    );
    assert!(
        events.saw_success_variant,
        "capture guard: expected the success slot event \
         ({}) — the success-path observability was not exercised",
        if want_revoke {
            "SlotRevoked"
        } else {
            "SlotRefreshed"
        }
    );
    assert!(
        events.saw_failed_variant,
        "capture guard: expected the failure slot event \
         ({}) — the failure-path error/event observability was not exercised",
        if want_revoke {
            "SlotRevokeFailed"
        } else {
            "SlotRefreshFailed"
        }
    );
    // The failing row records the `failed` outcome label and the OK row
    // the `success` label — proves the metric label surface under
    // inspection is non-empty and actually carries rotation series.
    assert!(
        metrics.contains("outcome=\"failed\"") && metrics.contains("outcome=\"success\""),
        "capture guard: expected success+failed outcome label series in the \
         Prometheus snapshot, got:\n{metrics}"
    );

    // ---- The invariant: no credential material on any surface ----
    assert_no_secret(&logs, "captured spans + events (tracing log buffer)");
    assert_no_secret(&events.rendered, "emitted ResourceEvent renderings");
    assert_no_secret(&metrics, "Prometheus metric label set");
}

/// Refresh fan-out: drive `dispatch_refresh` across a secret-bearing OK
/// row and a failing row; assert no secret in any captured span, emitted
/// `ResourceEvent`, metric label, or error string.
#[tokio::test]
async fn refresh_fanout_observability_is_redaction_clean() {
    // Capture is installed inside run_redaction_gate via set_default (NOT
    // with_default) — see the comment there before changing.
    run_redaction_gate(false).await;
}

/// Revoke fan-out: same gate over `dispatch_revoke`
/// (`SlotRevoked` / `SlotRevokeFailed`, revoke `outcome` series, the
/// taint→drain→hook path's spans).
#[tokio::test]
async fn revoke_fanout_observability_is_redaction_clean() {
    // Capture is installed inside run_redaction_gate via set_default (NOT
    // with_default) — see the comment there before changing.
    run_redaction_gate(true).await;
}

/// Load-bearing self-check: if the secret-absence assertion were vacuous
/// it would pass on a string that obviously contains the token. Feeding
/// the raw secret to `assert_no_secret` MUST panic — proof the gate's
/// core assertion actually fires (mirrors the negative test in
/// `crates/credential/tests/redaction.rs`).
#[test]
#[should_panic(expected = "redaction gate violation")]
fn assert_no_secret_is_load_bearing() {
    assert_no_secret(
        &format!("leaked the raw credential: {SECRET}"),
        "self-check",
    );
}
