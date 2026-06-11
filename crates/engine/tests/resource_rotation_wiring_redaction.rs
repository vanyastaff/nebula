#![cfg(feature = "rotation")]

//! Redaction gate for the **wired** rotation fan-out path ( ,
//! PRODUCT_CANON ).
//!
//! `resource_rotation_redaction.rs` proves the fan-out *port*
//! (`dispatch_*` called directly) leaks no credential material. This
//! mirrors that gate one layer up: the secret travels the full
//! production wiring — a `CredentialEvent` / `LeaseEvent` emitted on a
//! real `nebula-eventbus` bus → the spawned [`ResourceFanoutDriver`]
//! subscriber → `dispatch_{refresh,revoke}` → the secret-bearing
//! resource hook. The driver adds its own credential-data-free
//! `tracing` event (`record`), so this gate also covers *that* surface,
//! not just the fan-out internals.
//!
//! Capture harness is the verbatim `CaptureBuf` + `tracing-subscriber`
//! `MakeWriter` shape from `crates/credential/tests/redaction.rs` and
//! `resource_rotation_redaction.rs` (reused, not re-invented, so the
//! span/event capture semantics are identical to the established
//! gate). Because the driver runs on a `tokio::spawn`ed
//! task, the capture subscriber is installed process-wide via
//! `tracing::subscriber::set_global_default` for the test so the
//! spawned task's spans/events are also captured.

use std::io::{self, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialEvent, CredentialGuard, CredentialId, LeaseEvent};
use nebula_engine::credential::rotation::{ResourceFanoutDriver, ResourceFanoutIndex};
use nebula_eventbus::EventBus;
use nebula_resource::{
    AcquireOptions, Manager, Provider, RegistrationSpec, ResidentConfig, ResourceConfig,
    ResourceContext, SlotCell, SlotIdentity,
    error::Error as ResourceError,
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;
use zeroize::Zeroize;

/// Distinctive token planted in the rotated credential material + the
/// live runtime the hooks borrow. Long + structured so a substring
/// match cannot false-positive.
const SECRET: &str = "WIRED-ROTATION-SECRET-7b1e";

// ── CaptureBuf (verbatim shape from the established gate) ─────────

#[derive(Clone, Default)]
struct CaptureBuf(Arc<Mutex<Vec<u8>>>);

impl CaptureBuf {
    fn as_string(&self) -> String {
        let g = self.0.lock().expect("capture buffer poisoned");
        String::from_utf8_lossy(&g).into_owned()
    }
}

impl Write for CaptureBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(buf);
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

fn assert_no_secret(haystack: &str, surface: &str) {
    assert!(
        !haystack.contains(SECRET),
        "redaction gate violation: secret {SECRET:?} leaked into {surface} \
         (case-sensitive):\n---- {surface} ----\n{haystack}\n----"
    );
    assert!(
        !haystack.to_lowercase().contains(&SECRET.to_lowercase()),
        "redaction gate violation: secret {SECRET:?} leaked into {surface} \
         (case-insensitive):\n---- {surface} ----\n{haystack}\n----"
    );
}

// ── Secret-bearing resident resource ────────────────────────────────

struct SecretCred(String);
impl Zeroize for SecretCred {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
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

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct SecretRuntime {
    secret: String,
}

#[derive(Clone)]
struct SecretRes {
    #[allow(
        dead_code,
        reason = "models the author-declared resolved SlotCell; the hook borrows the runtime, not this cell — its presence makes the secret reachable through the resolved slot the fan-out rotates"
    )]
    db: Arc<SlotCell<CredentialGuard<SecretCred>>>,
    hook_entered: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for SecretRes {
    type Config = Cfg;
    type Instance = SecretRuntime;

    fn key() -> ResourceKey {
        resource_key!("wired-redaction-res")
    }

    async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<SecretRuntime, ResourceError> {
        Ok(SecretRuntime {
            secret: SECRET.to_owned(),
        })
    }

    async fn on_credential_refresh(
        &self,
        _s: &str,
        rt: &SecretRuntime,
    ) -> Result<(), ResourceError> {
        self.hook_entered.fetch_add(1, Ordering::SeqCst);
        // Genuinely handle the secret-bearing runtime on the rotation
        // path so redaction is not vacuously true.
        let _ = rt.secret.len();
        Ok(())
    }

    async fn on_credential_revoke(
        &self,
        _s: &str,
        rt: &SecretRuntime,
    ) -> Result<(), ResourceError> {
        self.hook_entered.fetch_add(1, Ordering::SeqCst);
        let _ = rt.secret.len();
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl nebula_resource::HasCredentialSlots for SecretRes {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl Resident for SecretRes {
    fn is_alive_sync(&self, _r: &SecretRuntime) -> bool {
        true
    }
}

// ── The gate ────────────────────────────────────────────────────────

/// Drive BOTH a refresh and a revoke through the wired driver against a
/// secret-bearing resource, capturing every span/event the spawned
/// driver + fan-out + resource side produce, then assert the secret
/// reached none of them — and that the capture is genuinely non-empty
/// (the driver's own completion event must be present).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wired_rotation_fanout_observability_is_redaction_clean() {
    // Process-wide capture: the driver runs on a spawned task, so a
    // thread-local subscriber would miss its spans. `set_global_default`
    // is fine here — this is a dedicated test binary target.
    let buf = CaptureBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("install global capture subscriber");

    let hook_entered = Arc::new(AtomicUsize::new(0));
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let mgr = Arc::new(Manager::new());
    let index = Arc::new(ResourceFanoutIndex::new());
    let cid = CredentialId::new();
    // The resolved-credential identity is the collision-free structural
    // key derived from the same `(slot, credential)` binding the resource
    // would resolve — used at register, acquire, and bind so all three
    // address the same registry row.
    let slot_identity = SlotIdentity::from_bindings([("db", "secret-cred")]);

    let slot: SlotCell<CredentialGuard<SecretCred>> = SlotCell::empty();
    slot.store(Arc::new(CredentialGuard::new(SecretCred(
        SECRET.to_owned(),
    ))));

    mgr.register(RegistrationSpec {
        resource: SecretRes {
            db: Arc::new(slot),
            hook_entered: Arc::clone(&hook_entered),
        },
        config: Cfg,
        scope: scope.clone(),
        slot_identity: slot_identity.clone(),
        topology: TopologyRuntime::resident(ResidentRuntime::<SecretRes>::new(
            ResidentConfig::default(),
        )),
        recovery_gate: None,
    })
    .expect("register resolved-credential row");

    let ctx = ResourceContext::minimal(
        Scope {
            org_id: Some(org),
            ..Default::default()
        },
        CancellationToken::new(),
    );
    let g = mgr
        .acquire_resident_for_identity::<SecretRes>(
            &ctx,
            &AcquireOptions::default(),
            &slot_identity,
        )
        .await
        .expect("warm secret-bearing runtime");
    drop(g);

    index.bind(
        cid,
        SecretRes::key(),
        scope.clone(),
        "db",
        slot_identity.clone(),
    );

    let cred_bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let lease_bus = Arc::new(EventBus::<LeaseEvent>::new(16));
    let _driver = ResourceFanoutDriver::spawn(
        Arc::clone(&index),
        Arc::clone(&mgr),
        Arc::clone(&cred_bus),
        Some(Arc::clone(&lease_bus)),
    );

    // Refresh via the credential bus.
    cred_bus.emit(CredentialEvent::Refreshed { credential_id: cid });
    // Revoke via the lease bus (the → path).
    lease_bus.emit(LeaseEvent::LeaseRevoked {
        credential_id: Some(cid),
        lease_id: "lease-redaction".to_owned(),
        provider: std::borrow::Cow::Borrowed("vault"),
    });

    // Wait until both hooks ran (refresh + revoke) — proof the wired
    // path handled the secret-bearing runtime on both directions.
    for _ in 0..3000 {
        if hook_entered.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        hook_entered.load(Ordering::SeqCst) >= 2,
        "both rotation + revoke hooks must have run through the wired driver \
         (handled the secret-bearing runtime), got {}",
        hook_entered.load(Ordering::SeqCst)
    );

    // Let the driver flush its credential-data-free completion events.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(2)).await;
        tokio::task::yield_now().await;
    }

    let logs = buf.as_string();

    // Capture-is-real guard: the driver's own completion event target
    // must be present, so an empty-capture false-clean is impossible.
    assert!(
        logs.contains("nebula_engine::credential::rotation"),
        "expected the wired driver's rotation log target in the capture — \
         capture-is-real guard, got:\n{logs}"
    );
    // And at least one fan-out span (proves the dispatch ran under
    // capture, not just the driver shell).
    assert!(
        logs.contains("nebula.credential.rotation.fanout_refresh")
            || logs.contains("nebula.credential.rotation.fanout_revoke")
            || logs.contains("nebula.resource.slot_refresh")
            || logs.contains("nebula.resource.slot_revoke"),
        "expected a fan-out / slot rotation span in the capture, got:\n{logs}"
    );

    // The invariant: no credential material on ANY captured surface
    // across the fully wired path (driver event + fan-out spans +
    // resource-side slot spans + any error string).
    assert_no_secret(&logs, "wired-path captured spans + events");
}

/// Load-bearing self-check: the absence assertion must actually fire on
/// a string that obviously contains the token (mirrors the negative
/// test in the established gates).
#[test]
#[should_panic(expected = "redaction gate violation")]
fn assert_no_secret_is_load_bearing() {
    assert_no_secret(&format!("leaked: {SECRET}"), "self-check");
}
