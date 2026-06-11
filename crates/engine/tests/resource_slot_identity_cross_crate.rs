//! Adversarial cross-crate byte-identity net for the structural slot
//! identity (R15 security residual).
//!
//! The U12 atomic-delete gate is `cargo check -p` per consumer crate. That
//! proves the engine and the resource crate *compile* against the
//! structural [`SlotIdentity`] — it does **not** prove the engine-recorded
//! structural key equals the value the resource-side register path filed
//! the registry row under. If those two construction sites ever diverged
//! (a different canonical sort, a different `CredentialKey` string
//! projection, an off-by-one in the pair shape) the engine would record /
//! fan-out / acquire under a key that no registry row carries — or, worse,
//! under a key that a *different tenant's* row carries (silent cross-tenant
//! re-aliasing, exactly the failure mode the collision-free structural
//! identity exists to make impossible).
//!
//! This test closes that residual at the value level, not the type level:
//!
//! 1. Register a resource **through the engine path**
//!    ([`ResourceRegistrarRegistry::register`]) with a non-empty
//!    `(slot, credential)` binding.
//! 2. Capture the [`SlotIdentity`] the manager-side `register_resolved`
//!    *returned* (propagated verbatim into
//!    [`ResourceRegistrationOutcome::slot_identity`]).
//! 3. **Independently** derive the resource-side identity for the *same*
//!    bindings via [`SlotIdentity::from_bindings`].
//! 4. Assert the two are byte-identical (`==`). A divergence here is a
//!    cross-tenant defect caught now, not in production.
//!
//! It also pins the negative: a *different* resolved credential yields a
//! *distinct* identity (so a digest collision could never re-merge them),
//! and the engine accessor's independent acquire-path derive
//! ([`slot_identities_for_key`]) agrees with the register-path value.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_core::{
    CredentialKey, DeclaresDependencies, Dependencies, ResourceKey,
    dependencies::{SlotField, SlotKind},
    resource_key,
};
use nebula_engine::{
    RegisterRequest, ResourceRegistrarRegistry, TypedResourceRegistrar,
    resource_accessor::slot_identities_for_key,
};
use nebula_expression::ExpressionEngine;
use nebula_resource::{
    Manager, ScopeLevel, SlotIdentity,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident,
};
use nebula_schema::HasSchema;

// ── A resource that declares one `#[credential]` slot ───────────────────────

#[derive(Debug, Clone)]
struct XError(String);

impl std::fmt::Display for XError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XError {}

impl From<XError> for ResourceError {
    fn from(e: XError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
struct XConfig {
    #[serde(default)]
    label: String,
}

nebula_schema::impl_empty_has_schema!(XConfig);

impl ResourceConfig for XConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.label.is_empty() {
            return Err(ResourceError::permanent("label must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone)]
struct XResource {
    create_counter: Arc<AtomicU64>,
}

impl XResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Provider for XResource {
    type Config = XConfig;
    type Instance = Arc<AtomicU64>;

    fn key() -> ResourceKey {
        resource_key!("xcross.widget")
    }

    fn create(
        &self,
        _config: &XConfig,
        _ctx: &nebula_resource::ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, nebula_resource::Error>> + Send {
        let counter = self.create_counter.clone();
        async move {
            let id = counter.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(AtomicU64::new(id)))
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(
            <Self as Provider>::key(),
            "xcross.widget".to_owned(),
            String::new(),
            <XConfig as HasSchema>::schema(),
        )
    }
}

/// The slot key the resource declares (and the binding key the test uses).
const SLOT_KEY: &str = "auth";

impl DeclaresDependencies for XResource {
    fn dependencies() -> Dependencies {
        // One `#[credential]` slot named `auth`. `register_resolved`'s
        // slot-binding validation requires every `slot_bindings` key to
        // match a declared credential slot, so a non-empty binding is only
        // accepted because this slot is declared.
        Dependencies::new().slot_field(SlotField {
            slot_key: SLOT_KEY,
            default_id: SLOT_KEY,
            kind: SlotKind::Credential {
                type_id: std::any::TypeId::of::<()>(),
                type_name: "test-credential",
                key: CredentialKey::new("auth").expect("valid credential key"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

impl nebula_resource::HasCredentialSlots for XResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl resident::Resident for XResource {
    fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
        runtime.load(Ordering::Relaxed) < u64::MAX
    }
}

fn registrars() -> ResourceRegistrarRegistry {
    let mut registrars = ResourceRegistrarRegistry::new();
    registrars.insert(
        "xcross.widget",
        Arc::new(TypedResourceRegistrar::<XResource, _, _, _>::new(
            XResource::new,
            || {
                TopologyRuntime::Resident(ResidentRuntime::<XResource>::new(
                    resident::config::Config::default(),
                ))
            },
            || Manager::erased_acquire_resident_for::<XResource>(),
        )),
    );
    registrars
}

fn request<'a>(expr: &'a ExpressionEngine, bindings: &[(&str, &str)]) -> RegisterRequest<'a> {
    let slot_bindings: HashMap<String, CredentialKey> = bindings
        .iter()
        .map(|(slot, cred)| {
            (
                (*slot).to_owned(),
                CredentialKey::new(*cred).expect("valid credential key"),
            )
        })
        .collect();
    RegisterRequest {
        config_json: serde_json::json!({ "label": "x" }),
        expr_engine: expr,
        slot_bindings,
        credential_ids: HashMap::new(),
        scope: ScopeLevel::Global,
        recovery_gate: None,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// The headline residual: the structural [`SlotIdentity`] the engine
/// records (the value `Manager::register_resolved` returned, propagated
/// through [`ResourceRegistrationOutcome`]) is **byte-identical** to an
/// independent resource-side derive over the same `(slot, credential)`
/// bindings. A divergence between the two construction sites would
/// silently re-alias tenants in production; this asserts it cannot.
#[tokio::test]
async fn engine_recorded_identity_is_byte_identical_to_resource_side_derive() {
    let manager = Manager::new();
    let expr = ExpressionEngine::with_cache_size(16);
    let reg = registrars();

    let bindings = [(SLOT_KEY, "cred-tenant-a")];

    // (1)+(2) Register through the engine path; capture the structural
    // identity the manager-side register path derived and returned.
    let outcome = reg
        .register("xcross.widget", &manager, request(&expr, &bindings))
        .await
        .expect("engine-path registration of a credential-bound resource");

    // (3) Independently derive the resource-side identity for the SAME
    // bindings. This is the exact constructor `register_resolved` uses
    // internally — driven here from a separate call site.
    let resource_side = SlotIdentity::from_bindings(bindings.iter().copied());

    // (4) Byte-identical or the cross-crate seam is broken.
    assert_eq!(
        outcome.slot_identity, resource_side,
        "engine-recorded structural slot identity diverged from an \
         independent resource-side derive for the same resolved bindings — \
         this is a cross-tenant re-aliasing defect, not a cosmetic mismatch"
    );

    // It is a real `Structural` identity (a credential WAS bound), not the
    // `Unbound` no-slots sentinel — so the equality above is non-trivial.
    assert!(
        !outcome.slot_identity.is_unbound(),
        "a bound credential must yield a Structural identity, not Unbound"
    );
    assert!(
        matches!(outcome.slot_identity, SlotIdentity::Structural(_)),
        "the recorded identity must be the collision-free Structural form"
    );

    // The row is resolvable in the manager under exactly that identity
    // (the engine recorded the key the registry row is actually filed
    // under — not some other digest).
    assert!(
        manager.has_registered_for_identity(
            &<XResource as Provider>::key(),
            &ScopeLevel::Global,
            &outcome.slot_identity,
        ),
        "the registry row must be resolvable under the recorded structural \
         identity"
    );
}

/// The engine accessor's *independent* acquire-path derive
/// ([`slot_identities_for_key`], used to populate the per-execution
/// slot-identity map) must agree with the register-path value. These are
/// two genuinely separate construction sites in the engine; they must
/// converge on the same structural key or acquire would miss the row the
/// register path created.
#[tokio::test]
async fn accessor_acquire_path_derive_agrees_with_register_path() {
    let manager = Manager::new();
    let expr = ExpressionEngine::with_cache_size(16);
    let reg = registrars();

    let bindings = [(SLOT_KEY, "cred-tenant-a")];

    let outcome = reg
        .register("xcross.widget", &manager, request(&expr, &bindings))
        .await
        .expect("engine-path registration");

    // The accessor builds its acquire-path identity map from the resolved
    // `(slot, credential)` pairs via `slot_identities_for_key` — a separate
    // construction site from the register path.
    let acquire_map = slot_identities_for_key(<XResource as Provider>::key(), &bindings);
    let acquire_side = acquire_map
        .get(&<XResource as Provider>::key())
        .expect("accessor acquire-path identity is present for the key");

    assert_eq!(
        *acquire_side, outcome.slot_identity,
        "the accessor acquire-path structural identity must equal the \
         register-path value, or action-time acquire would address a \
         different (or no) registry row than the one registration created"
    );
}

/// The negative: a *different* resolved credential for the same slot
/// yields a *distinct* structural identity end to end, so a digest
/// collision could never silently re-merge two tenants. Both sides
/// (engine-recorded and resource-side derive) must reflect the
/// distinction.
#[tokio::test]
async fn distinct_resolved_credentials_never_alias_cross_crate() {
    let manager = Manager::new();
    let expr = ExpressionEngine::with_cache_size(16);
    let reg = registrars();

    let bindings_a = [(SLOT_KEY, "cred-tenant-a")];
    let bindings_b = [(SLOT_KEY, "cred-tenant-b")];

    let outcome_a = reg
        .register("xcross.widget", &manager, request(&expr, &bindings_a))
        .await
        .expect("tenant-a registration");
    // A second registration at the same (key, scope) but a different
    // resolved credential is a DISTINCT registry row (structural
    // anti-bleed) — it does not replace tenant-a's row.
    let outcome_b = reg
        .register("xcross.widget", &manager, request(&expr, &bindings_b))
        .await
        .expect("tenant-b registration");

    assert_ne!(
        outcome_a.slot_identity, outcome_b.slot_identity,
        "two different resolved credentials must have distinct structural \
         identities — equality here would be cross-tenant bleed"
    );

    // The resource-side independent derive agrees on the distinction.
    let resource_a = SlotIdentity::from_bindings(bindings_a.iter().copied());
    let resource_b = SlotIdentity::from_bindings(bindings_b.iter().copied());
    assert_eq!(outcome_a.slot_identity, resource_a);
    assert_eq!(outcome_b.slot_identity, resource_b);
    assert_ne!(resource_a, resource_b);

    // Both rows are independently resolvable under their own identity, and
    // neither is visible under the other's — fail-closed isolation.
    let key = <XResource as Provider>::key();
    assert!(manager.has_registered_for_identity(
        &key,
        &ScopeLevel::Global,
        &outcome_a.slot_identity
    ));
    assert!(manager.has_registered_for_identity(
        &key,
        &ScopeLevel::Global,
        &outcome_b.slot_identity
    ));
}
