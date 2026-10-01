//! Engine-side [`ResourceAccessor`] implementation.
//!
//! [`EngineResourceAccessor`] bridges the engine's resource manager to the
//! [`ResourceAccessor`] capability trait consumed by actions. It serves each
//! row's per-unit checkout facade — a boxed
//! [`ResourceHandle<R>`](nebula_resource::call::ResourceHandle) for downcast
//! by action code (slot-identity-pinned, scope-aware) — and nothing else: a
//! raw lease ([`nebula_resource::ResourceGuard`]) bypasses the effect journal,
//! so it stays a host-only capability of the manager and never reaches an
//! action context.

use std::{any::Any, collections::HashMap, fmt, sync::Arc};

use nebula_core::{CoreError, ResourceKey, accessor::ResourceAccessor, scope::Scope};
use nebula_resource::{
    AcquireOptions, ErrorKind, Manager, ResourceContext, SlotIdentity, call::journal::EffectJournal,
};
use tokio_util::sync::CancellationToken;

/// Engine-side implementation of [`ResourceAccessor`].
///
/// Wraps an [`Arc<nebula_resource::Manager>`] and resolves rows using the
/// execution scope and optional per-key slot identities recorded at
/// activation. `resource_handle_any` hands out the row's per-unit checkout
/// facade, by default through
/// [`Manager::handle_any_read_only`](nebula_resource::Manager::handle_any_read_only):
/// its units are cancelled with the node's cancellation token until their
/// first grant and bounded by the execution deadline
/// ([`with_deadline`](Self::with_deadline)).
///
/// The handles it hands out are read-only unless the engine gave a node's
/// accessor that node's effect journal: its handles then drive every
/// `Idempotent` and `Write` unit through the journal
/// ([`Manager::handle_any_journaled`](nebula_resource::Manager::handle_any_journaled)).
#[derive(Clone)]
pub struct EngineResourceAccessor {
    manager: Arc<Manager>,
    scope: Scope,
    cancel: CancellationToken,
    slot_identities: Arc<HashMap<ResourceKey, SlotIdentity>>,
    /// The execution deadline bounding every managed row unit; `None` when
    /// the execution has no wall-clock budget.
    deadline: Option<std::time::Instant>,
    /// What the handed-out handles may do with effects.
    effects: HandleEffects,
}

/// The effect authority of an engine accessor's resource handles.
#[derive(Clone)]
enum HandleEffects {
    /// Reads only; an effect is refused with the detail, or the resource
    /// runtime's generic one.
    ReadOnly(Option<&'static str>),
    /// Effects are prepared, granted and recorded by the node's journal.
    Journaled(Arc<dyn EffectJournal>),
}

impl EngineResourceAccessor {
    /// Creates a new accessor backed by the given resource manager.
    #[must_use]
    pub fn new(manager: Arc<Manager>, scope: Scope, cancel: CancellationToken) -> Self {
        Self {
            manager,
            scope,
            cancel,
            slot_identities: Arc::new(HashMap::new()),
            deadline: None,
            effects: HandleEffects::ReadOnly(None),
        }
    }

    /// Hands out handles whose `Idempotent` and `Write` units `journal`
    /// prepares, grants and records, instead of read-only handles. The
    /// engine sets it only for a node whose admitted journaled action runs
    /// under that node attempt's journal.
    #[must_use]
    pub(crate) fn with_journal(mut self, journal: Arc<dyn EffectJournal>) -> Self {
        self.effects = HandleEffects::Journaled(journal);
        self
    }

    /// Keeps the handles read-only, refusing an effect with `detail` — why
    /// the node has no effect journal.
    #[must_use]
    pub(crate) fn with_read_only_detail(mut self, detail: &'static str) -> Self {
        self.effects = HandleEffects::ReadOnly(Some(detail));
        self
    }

    /// Bounds the units of every managed row this accessor hands out by
    /// `deadline` — the execution's wall-clock budget. A lease acquire is
    /// not bounded by it.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Option<std::time::Instant>) -> Self {
        self.deadline = deadline;
        self
    }

    /// Overrides the default slot-identity map (key → resolved
    /// **collision-free structural** credential identity).
    #[must_use]
    pub fn with_slot_identities(
        mut self,
        slot_identities: HashMap<ResourceKey, SlotIdentity>,
    ) -> Self {
        self.slot_identities = Arc::new(slot_identities);
        self
    }

    /// Like [`with_slot_identities`](Self::with_slot_identities) but shares an
    /// existing `Arc` (per-execution snapshot on the engine).
    #[must_use]
    pub fn with_slot_identities_arc(
        mut self,
        slot_identities: Arc<HashMap<ResourceKey, SlotIdentity>>,
    ) -> Self {
        self.slot_identities = slot_identities;
        self
    }

    /// The resolved structural slot identity recorded for `key` at
    /// activation, or [`SlotIdentity::Unbound`] when the key resolved no
    /// credential slots (the historical single-row-per-`(key, scope)`
    /// behaviour).
    fn slot_identity_for(&self, key: &ResourceKey) -> SlotIdentity {
        self.slot_identities
            .get(key)
            .cloned()
            .unwrap_or(SlotIdentity::Unbound)
    }

    fn resource_ctx(&self) -> ResourceContext {
        ResourceContext::minimal(self.scope.clone(), self.cancel.clone())
    }

    fn map_err(_key: &ResourceKey, err: nebula_resource::Error) -> CoreError {
        err.to_core_error()
    }

    /// The resource handle of `key` under the accessor's effect authority.
    fn handle(
        &self,
        key: &ResourceKey,
    ) -> Result<Box<dyn Any + Send + Sync>, nebula_resource::Error> {
        let options = match self.deadline {
            Some(deadline) => AcquireOptions::default().with_deadline(deadline),
            None => AcquireOptions::default(),
        };
        let ctx = self.resource_ctx();
        let identity = self.slot_identity_for(key);
        match &self.effects {
            HandleEffects::Journaled(journal) => self.manager.handle_any_journaled(
                key,
                &ctx,
                &options,
                &identity,
                Arc::clone(journal),
            ),
            HandleEffects::ReadOnly(Some(detail)) => self
                .manager
                .handle_any_read_only_because(key, &ctx, &options, &identity, detail),
            HandleEffects::ReadOnly(None) => self
                .manager
                .handle_any_read_only(key, &ctx, &options, &identity),
        }
    }
}

impl fmt::Debug for EngineResourceAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineResourceAccessor")
            .field("manager", &"<Manager>")
            .field("scope", &self.scope)
            .field(
                "journaled",
                &matches!(self.effects, HandleEffects::Journaled(_)),
            )
            .finish()
    }
}

impl ResourceAccessor for EngineResourceAccessor {
    fn has(&self, key: &ResourceKey) -> bool {
        self.manager.has_registered_for_scope_identity(
            key,
            &self.scope,
            &self.slot_identity_for(key),
        )
    }

    fn resource_handle_any(
        &self,
        key: &ResourceKey,
    ) -> Result<Box<dyn Any + Send + Sync>, CoreError> {
        self.handle(key).map_err(|e| Self::map_err(key, e))
    }

    fn try_resource_handle_any(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<Box<dyn Any + Send + Sync>>, CoreError> {
        match self.handle(key) {
            Ok(row) => Ok(Some(row)),
            Err(error) if matches!(error.kind(), ErrorKind::NotFound) => Ok(None),
            Err(error) => Err(Self::map_err(key, error)),
        }
    }
}

/// Build slot identities for activation from resolved `(slot, credential)`
/// pairs, keyed by the **collision-free structural**
/// [`SlotIdentity`].
///
/// This is constructed via
/// [`SlotIdentity::from_bindings`](nebula_resource::SlotIdentity::from_bindings)
/// over the **same** `(slot, credential)` pairs the resource-side register
/// path hashes, so the accessor addresses the *exact* registry row
/// `Manager::register_resolved` created (byte-identical structural key).
#[must_use]
pub fn slot_identities_for_key(
    key: ResourceKey,
    pairs: &[(&str, &str)],
) -> HashMap<ResourceKey, SlotIdentity> {
    let id = SlotIdentity::from_bindings(pairs.iter().copied());
    let mut map = HashMap::new();
    map.insert(key, id);
    map
}

#[cfg(test)]
mod tests {
    use std::{
        fmt,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use nebula_resource::{
        Manager, RegistrationSpec, Resident, ResidentConfig, ResourceContext, ScopeLevel,
        SlotIdentity,
        error::Error,
        resource::{Provider, ResourceConfig, ResourceMetadataDraft},
        topology::resident::ResidentProvider,
    };

    use super::*;

    #[derive(Debug, Clone)]
    struct AccError(String);

    impl fmt::Display for AccError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for AccError {}

    impl From<AccError> for Error {
        fn from(e: AccError) -> Self {
            Error::permanent(e.0)
        }
    }

    #[derive(Clone, Debug, Default, nebula_schema::Schema)]
    struct AccConfig;

    impl ResourceConfig for AccConfig {
        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct AccResource;

    #[async_trait::async_trait]
    impl Provider for AccResource {
        type Config = AccConfig;
        type Instance = Arc<AtomicU64>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            ResourceKey::new("test.engine_accessor.acc").expect("valid resource key in test")
        }

        async fn create(
            &self,
            _config: &AccConfig,
            _ctx: &ResourceContext,
        ) -> Result<Arc<AtomicU64>, Error> {
            Ok(Arc::new(AtomicU64::new(42)))
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(
                Self::key(),
                nebula_resource::metadata_name!("test.engine_accessor.acc"),
                "",
            )
        }
    }

    nebula_resource::no_credential_slots!(AccResource);

    #[async_trait::async_trait]
    impl ResidentProvider for AccResource {
        fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
            true
        }
    }

    fn make_accessor(manager: Arc<Manager>) -> EngineResourceAccessor {
        EngineResourceAccessor::new(manager, Scope::default(), CancellationToken::new())
    }

    fn rk(key: &str) -> ResourceKey {
        ResourceKey::new(key).expect("valid resource key in test")
    }

    #[tokio::test]
    async fn has_returns_false_for_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        assert!(!accessor.has(&rk("postgres")));
    }

    #[tokio::test]
    async fn resource_handle_any_refuses_an_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let result = accessor.resource_handle_any(&rk("postgres"));
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn try_resource_handle_any_returns_none_for_unregistered_key() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let result = accessor.try_resource_handle_any(&rk("postgres"));
        assert!(matches!(result, Ok(None)));
    }

    #[tokio::test]
    async fn resource_handle_any_serves_a_registered_row() {
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: AccResource,
                config: AccConfig,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<AccResource>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register");

        let accessor = make_accessor(Arc::clone(&manager));
        let row = row_of(&accessor);
        assert_eq!(row.submit(Read).await.expect("a read runs"), 42);
    }

    #[tokio::test]
    async fn debug_redacts_manager() {
        let accessor = make_accessor(Arc::new(Manager::new()));
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<Manager>"));
    }

    #[tokio::test]
    async fn handles_use_the_recorded_slot_identity_not_unbound() {
        let manager = Arc::new(Manager::new());
        let key = AccResource::key();
        let bound = SlotIdentity::from_bindings([("slot", "cred-a")]);

        manager
            .register(RegistrationSpec {
                resource: AccResource,
                config: AccConfig,
                scope: ScopeLevel::Global,
                slot_identity: bound.clone(),
                topology: Resident::<AccResource>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register cred-bound row");

        let accessor = make_accessor(Arc::clone(&manager))
            .with_slot_identities(HashMap::from([(key.clone(), bound)]));
        assert!(accessor.has(&key));
        let row = accessor
            .try_resource_handle_any(&key)
            .expect("lookup with matching slot identity")
            .expect("the cred-bound row is served");
        let row = row
            .downcast::<ResourceHandle<AccResource>>()
            .expect("ResourceHandle downcast");
        assert_eq!(row.submit(Read).await.expect("a read runs"), 42);

        // The unbound identity never reaches a credential-bound row.
        let unbound = make_accessor(Arc::clone(&manager));
        assert!(!unbound.has(&key));
        assert!(
            unbound
                .try_resource_handle_any(&key)
                .expect("lookup")
                .is_none()
        );

        let wrong = make_accessor(manager).with_slot_identities(HashMap::from([(
            key.clone(),
            SlotIdentity::from_bindings([("slot", "other")]),
        )]));
        assert!(
            !wrong.has(&key),
            "has must not see cred-bound row under a different slot identity"
        );
        let missing = wrong.try_resource_handle_any(&key).expect("try lookup");
        assert!(missing.is_none());
    }

    // ── managed rows ─────────────────────────────────────────────────────

    use nebula_resource::{
        call::{Cost, Effect, Operation, OperationCx, OperationError, ResourceHandle, SentState},
        rate_limit::{Rate, RowLimit},
    };

    /// Registers the fixture under `identity`, limited to `limit`.
    fn register_acc(manager: &Manager, identity: SlotIdentity, limit: Option<RowLimit>) {
        manager
            .register(RegistrationSpec {
                resource: AccResource,
                config: AccConfig,
                scope: ScopeLevel::Global,
                slot_identity: identity,
                topology: Resident::<AccResource>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: limit,
            })
            .expect("register");
    }

    fn row_of(accessor: &EngineResourceAccessor) -> ResourceHandle<AccResource> {
        *accessor
            .resource_handle_any(&AccResource::key())
            .expect("managed row")
            .downcast::<ResourceHandle<AccResource>>()
            .expect("ResourceHandle downcast")
    }

    /// Reads the instance's value in one attempt costing one permit.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Read;

    impl Operation<AccResource> for Read {
        type Output = u64;
        const KEY: &'static str = "acc.read";
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OperationCx<'_, AccResource>) -> Result<u64, OperationError> {
            cx.call(Cost::ONE, async |value, ()| {
                Ok(value.load(Ordering::Relaxed))
            })
            .await
        }
    }

    /// Records the unit's deadline.
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct UnitDeadline {
        #[serde(skip)]
        seen: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    }

    impl Operation<AccResource> for UnitDeadline {
        type Output = ();
        const KEY: &'static str = "acc.unit_deadline";
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OperationCx<'_, AccResource>) -> Result<(), OperationError> {
            *self.seen.lock().expect("deadline cell") = Some(cx.deadline());
            Ok(())
        }
    }

    /// The deadline a unit of `row` runs under.
    async fn unit_deadline(row: &ResourceHandle<AccResource>) -> std::time::Instant {
        let probe = UnitDeadline::default();
        let seen = Arc::clone(&probe.seen);
        row.submit(probe).await.expect("deadline");
        seen.lock().expect("deadline cell").expect("the unit ran")
    }

    #[tokio::test]
    async fn resource_handle_any_serves_the_row_of_the_recorded_slot_identity() {
        let manager = Arc::new(Manager::new());
        let key = AccResource::key();
        let bound = SlotIdentity::from_bindings([("slot", "cred-a")]);
        register_acc(&manager, bound.clone(), None);

        let accessor = make_accessor(Arc::clone(&manager))
            .with_slot_identities(HashMap::from([(key.clone(), bound)]));
        let row = row_of(&accessor);
        assert_eq!(row.submit(Read).await.expect("granted"), 42);

        let wrong = make_accessor(manager).with_slot_identities(HashMap::from([(
            key.clone(),
            SlotIdentity::from_bindings([("slot", "other")]),
        )]));
        let error = wrong
            .resource_handle_any(&key)
            .expect_err("another identity's row is not served");
        assert!(
            matches!(error, CoreError::CredentialNotFound { .. }),
            "not found, got {error:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_accessor_token_cancels_a_unit_queued_for_quota() {
        let manager = Arc::new(Manager::new());
        // One permit a minute: the second unit's slot lands inside its
        // deadline, so it waits rather than being refused `Exhausted`.
        let per_minute = Rate::new(std::num::NonZeroU32::MIN, std::time::Duration::from_mins(1))
            .expect("valid rate");
        register_acc(
            &manager,
            SlotIdentity::Unbound,
            Some(RowLimit::rate(per_minute)),
        );
        let token = CancellationToken::new();
        let accessor = EngineResourceAccessor::new(manager, Scope::default(), token.clone());
        let row = row_of(&accessor);

        row.submit(Read).await.expect("books the hour's permit");
        let queued = tokio::spawn(row.submit(Read));
        tokio::task::yield_now().await;
        assert!(!queued.is_finished(), "waiting for quota");
        token.cancel();
        let error = queued.await.expect("joined").expect_err("cancelled");
        assert_eq!(*error.kind(), ErrorKind::Cancelled);
        assert_eq!(error.sent(), SentState::NotSent);
    }

    /// A write the provider would apply.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Bump;

    impl Operation<AccResource> for Bump {
        type Output = u64;
        const KEY: &'static str = "acc.bump";
        const EFFECT: Effect = Effect::Write;

        async fn run(self, cx: &mut OperationCx<'_, AccResource>) -> Result<u64, OperationError> {
            cx.call(Cost::ONE, async |value, ()| {
                Ok(value.fetch_add(1, Ordering::Relaxed))
            })
            .await
        }
    }

    #[tokio::test]
    async fn read_only_handles_refuse_writes_with_the_accessors_detail() {
        let manager = Arc::new(Manager::new());
        register_acc(&manager, SlotIdentity::Unbound, None);

        let plain = row_of(&make_accessor(Arc::clone(&manager)));
        let refused = plain.submit(Bump).await.expect_err("read-only");
        assert_eq!(
            refused.detail(),
            "managed row effect requires execution-owner authority"
        );

        let storeless = row_of(
            &make_accessor(Arc::clone(&manager))
                .with_read_only_detail("journaled effects need execution stores"),
        );
        let refused = storeless.submit(Bump).await.expect_err("read-only");
        assert_eq!(refused.detail(), "journaled effects need execution stores");
        assert_eq!(refused.sent(), SentState::NotSent);
        assert_eq!(storeless.submit(Read).await.expect("a read runs"), 42);
    }

    #[tokio::test(start_paused = true)]
    async fn the_execution_deadline_bounds_every_unit() {
        let manager = Arc::new(Manager::new());
        register_acc(&manager, SlotIdentity::Unbound, None);
        let deadline = tokio::time::Instant::now().into_std() + std::time::Duration::from_secs(40);

        let bounded = make_accessor(Arc::clone(&manager)).with_deadline(Some(deadline));
        assert_eq!(unit_deadline(&row_of(&bounded)).await, deadline);

        let unbounded = make_accessor(manager);
        let capped = unit_deadline(&row_of(&unbounded)).await;
        assert_eq!(
            capped,
            tokio::time::Instant::now().into_std() + nebula_resource::call::OPERATION_DEADLINE_CAP
        );
    }
}
