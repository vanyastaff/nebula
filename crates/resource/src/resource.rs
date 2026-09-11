//! Core provider trait and supporting types.
//!
//! [`Provider`] is the central lifecycle trait: it describes how to create,
//! health-check, and tear down a single resource type. Implementors supply
//! three associated types (`Config`, `Instance`, `Topology`) and the lifecycle methods
//! (slot model).
//!
//! Per slot model (supersedes credential isolation) the singular `type Credential`
//! associated type was deleted in favor of typed credential **slot fields**
//! declared on the resource struct via `#[credential(key = "...")]` (the
//! `#[derive(Resource)]` macro emits a `DeclaresDependencies` impl that
//! enumerates them, plus slot accessors, plus `impl HasCredentialSlots`).
//!
//! `Provider::create(&self, ctx)` no longer takes an explicit
//! `scheme: &<R::Credential as Credential>::Scheme` argument: the framework
//! resolves every declared `#[credential]` slot **before** invoking
//! `create`. Each slot field is a `SlotCell<CredentialGuard<C>>` cell; the
//! implementation reads the resolved guard through the derive-emitted
//! `<field>_slot()` accessor (`Option<Arc<CredentialGuard<C>>>`).
//!
//! Per-credential rotation is exposed via
//! [`Provider::on_credential_refresh`], which receives the **slot name**
//! that rotated and the live `Instance` handle (so multi-credential
//! resources can choose to refresh only the affected pool, headers, etc.
//! via interior mutability). Revocation is signalled via
//! [`Provider::on_credential_revoke`].
//!
//! [`HasCredentialSlots`] is a separate trait (not on `Provider`) implemented
//! by `#[derive(Resource)]`. Resources with no credential slots need only
//! implement `Provider` — a blanket `impl HasCredentialSlots` for such types
//! is unnecessary since the epoch is structurally `0`.

use async_trait::async_trait;
use nebula_core::ResourceKey;
use nebula_metadata::{
    BaseCompatError, BaseMetadata, Metadata, MetadataDraft, RecordedBaseMetadata,
    validate_base_compat,
};
use nebula_schema::ValidSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::context::ResourceContext;

// `ResourceDescriptor` was retired in ADR-0095 D2 — replaced by the B+ merged
// `ResourceFactory` in `crate::factory`, which carries both introspection
// (key + metadata + validate) and construction (register). No shim, no alias.

/// Operational configuration for a resource. Contains NO secrets.
///
/// Implementors typically derive `serde::Deserialize` and hold fields like
/// host, port, pool size, timeouts, etc.
///
/// Must implement [`HasSchema`](nebula_schema::HasSchema) so the resource
/// metadata can auto-derive its configuration schema from the config type.
/// `()` declares a null root; primitives such as `bool` and `String` declare
/// their scalar kinds. An empty-braced struct declares an empty record, not
/// null. JSON-driven registration must use the declared serde wire shape.
/// The `ResourceConfig` derive requires `#[config(schema = external)]` plus a
/// derived or manual `HasSchema` for every nonempty or tuple config; it only
/// supplies automatic schemas for unit and empty-braced structs.
pub trait ResourceConfig: nebula_schema::HasSchema + Send + Sync + Clone + 'static {
    /// Validates the configuration, returning an error if invalid.
    ///
    /// The default implementation accepts all configurations.
    ///
    /// # Errors
    ///
    /// Implementors return [`Error::permanent`](crate::Error::permanent) for
    /// a malformed config (bad DSN, out-of-range size, missing required
    /// field) — this runs on [`Manager::register`](crate::Manager::register)
    /// and [`Manager::reload_config`](crate::Manager::reload_config), so the
    /// caller sees the rejection at registration/reload time rather than a
    /// later `create` failure. The default implementation never errors.
    fn validate(&self) -> Result<(), crate::Error> {
        Ok(())
    }

    /// Returns a fingerprint for change-detection during hot-reload.
    ///
    /// Two configs with equal fingerprints are treated as **identical** by the
    /// manager's hot-reload path: a reload where the old and new fingerprints
    /// match returns [`ReloadOutcome::NoChange`](crate::reload::ReloadOutcome)
    /// without swapping the live config or bumping the generation counter.
    ///
    /// **You MUST return a value that differs whenever any operationally-significant
    /// field differs.** Returning a constant from a struct that has fields is
    /// incorrect — it permanently disables hot-reload change-detection for that
    /// config type. Derive [`ResourceConfig`](nebula_resource_macros::ResourceConfig)
    /// for a correct structural default:
    ///
    /// ```
    /// use nebula_resource::{ResourceConfig, Schema};
    ///
    /// #[derive(ResourceConfig, Schema, Clone)]
    /// #[config(schema = external)]
    /// struct PgConfig {
    ///     url: String,
    ///     max_conns: u32,
    /// }
    ///
    /// // `fingerprint()` is emitted automatically — no manual impl needed — and it
    /// // changes whenever an operationally-significant field changes.
    /// let cfg = PgConfig { url: "postgres://db".to_owned(), max_conns: 8 };
    /// let resized = PgConfig { max_conns: 16, ..cfg.clone() };
    /// assert_ne!(cfg.fingerprint(), resized.fingerprint());
    /// assert_eq!(cfg.fingerprint(), cfg.clone().fingerprint());
    /// ```
    ///
    /// The only correct use of a constant fingerprint is for a **fieldless** config
    /// (unit struct or `()`), where all instances are structurally identical.
    fn fingerprint(&self) -> u64;
}

/// `()` is the canonical no-config sentinel for resources that take no user configuration.
///
/// All `()` values are structurally identical, so fingerprint `0` is correct:
/// two unit configs are always the same, and a reload with `()` ↔ `()` is
/// always a no-op — which is exactly what you want.
impl ResourceConfig for () {
    fn fingerprint(&self) -> u64 {
        // Unit type: no fields, all instances identical — 0 is the correct constant.
        0
    }
}

/// Resource metadata authoring state before the canonical configuration schema is bound.
///
/// Authors return this type from [`Provider::metadata`]. The resource factory
/// derives the schema from `R::Config` and performs the only transition to
/// getter-only [`ResourceMetadata`]. A draft cannot supply or override a schema.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "resource metadata drafts are admitted by a resource factory"]
pub struct ResourceMetadataDraft {
    base: MetadataDraft<ResourceKey>,
}

impl ResourceMetadataDraft {
    /// Create a draft from an already checked display name.
    pub fn new(
        key: ResourceKey,
        name: nebula_metadata::MetadataName,
        description: impl Into<String>,
    ) -> Self {
        Self {
            base: MetadataDraft::new(key, name, description),
        }
    }

    /// Create a draft after checking a dynamic display name.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_metadata::MetadataError::BlankName`] for empty or
    /// whitespace-only text.
    pub fn try_new(
        key: ResourceKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, nebula_metadata::MetadataError> {
        Ok(Self {
            base: MetadataDraft::try_new(key, name, description)?,
        })
    }

    /// Create minimal author intent derived from a resource key.
    pub fn from_key(key: ResourceKey) -> Self {
        Self::new(key.clone(), key.into(), String::new())
    }

    /// Set the interface version.
    pub fn with_version(mut self, version: Version) -> Self {
        self.base = self.base.with_version(version);
        self
    }

    /// Set the catalog icon.
    pub fn with_icon(mut self, icon: nebula_metadata::Icon) -> Self {
        self.base = self.base.with_icon(icon);
        self
    }

    /// Set an inline-identifier icon.
    pub fn with_inline_icon(mut self, name: impl Into<String>) -> Self {
        self.base = self.base.with_inline_icon(name);
        self
    }

    /// Set a URL-backed icon.
    pub fn with_url_icon(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_url_icon(url);
        self
    }

    /// Set the documentation URL.
    pub fn with_documentation_url(mut self, url: impl Into<String>) -> Self {
        self.base = self.base.with_documentation_url(url);
        self
    }

    /// Replace all catalog tags.
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.base = self.base.with_tags(tags);
        self
    }

    /// Append one catalog tag.
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.base = self.base.add_tag(tag);
        self
    }

    /// Mark the resource experimental.
    pub fn mark_experimental(mut self) -> Self {
        self.base = self.base.mark_experimental();
        self
    }

    /// Mark the resource beta.
    pub fn mark_beta(mut self) -> Self {
        self.base = self.base.mark_beta();
        self
    }

    /// Mark the resource stable.
    pub fn mark_stable(mut self) -> Self {
        self.base = self.base.mark_stable();
        self
    }

    /// Attach a deprecation notice and mark the resource deprecated.
    pub fn with_deprecation(mut self, notice: nebula_metadata::DeprecationNotice) -> Self {
        self.base = self.base.with_deprecation(notice);
        self
    }

    pub(crate) fn admit(self, schema: ValidSchema) -> ResourceMetadata {
        ResourceMetadata {
            base: self.base.bind_schema(schema),
        }
    }
}

/// Immutable resource metadata admitted by a resource factory.
///
/// The schema is always derived from the resource's `R::Config` type. Fields
/// are private and exposed through getters; rebuild a fresh draft to change a
/// static definition. This admitted form serializes for catalogs but cannot be
/// deserialized from wire or persistence data.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceMetadata {
    #[serde(flatten)]
    base: BaseMetadata<ResourceKey>,
}

impl ResourceMetadata {
    /// Shared admitted catalog metadata.
    #[must_use]
    pub const fn base(&self) -> &BaseMetadata<ResourceKey> {
        &self.base
    }

    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataCompatibilityError`] when the key changes, the
    /// version moves backward, or a schema change lacks a major-version bump.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        validate_base_compat(&self.base, &previous.base)?;
        Ok(())
    }
}

impl Metadata for ResourceMetadata {
    type Key = ResourceKey;

    fn base(&self) -> &BaseMetadata<ResourceKey> {
        &self.base
    }
}

/// Deserialized evidence of a previously admitted resource definition.
///
/// Recorded fields never become authority. Call [`Self::readmit_against`]
/// with metadata freshly admitted from the current static resource type.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedResourceMetadata {
    #[serde(flatten)]
    base: RecordedBaseMetadata<ResourceKey>,
}

impl RecordedResourceMetadata {
    /// Recorded shared metadata evidence.
    #[must_use]
    pub const fn base(&self) -> &RecordedBaseMetadata<ResourceKey> {
        &self.base
    }

    /// Re-admit an exact recorded definition against fresh static metadata.
    ///
    /// The returned value is cloned entirely from `fresh_definition`.
    ///
    /// # Errors
    ///
    /// Returns [`nebula_metadata::MetadataReadmissionError::DefinitionMismatch`]
    /// when any recorded field or the canonical schema differs.
    #[tracing::instrument(name = "resource.metadata.readmit_recorded", skip_all, err)]
    pub fn readmit_against(
        &self,
        fresh_definition: &ResourceMetadata,
    ) -> Result<ResourceMetadata, nebula_metadata::MetadataReadmissionError> {
        self.base.readmit_against(fresh_definition.base())?;
        Ok(fresh_definition.clone())
    }
}

/// A resource catalog definition failed factory admission.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataBuildError {
    /// Shared metadata or canonical schema construction failed.
    #[error(transparent)]
    Definition(#[from] nebula_metadata::MetadataBuildError),
    /// The authored metadata key differs from the provider's canonical key.
    #[error("resource metadata key `{actual}` does not match provider key `{expected}`")]
    KeyMismatch {
        /// Canonical key declared by [`Provider::key`].
        expected: ResourceKey,
        /// Key carried by the schema-free author draft.
        actual: ResourceKey,
    },
}

impl From<nebula_schema::ValidationReport> for MetadataBuildError {
    fn from(report: nebula_schema::ValidationReport) -> Self {
        Self::Definition(nebula_metadata::MetadataBuildError::from(report))
    }
}

impl nebula_error::Classify for MetadataBuildError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Definition(source) => nebula_error::Classify::category(source),
            Self::KeyMismatch { .. } => nebula_error::ErrorCategory::Validation,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Definition(source) => nebula_error::Classify::code(source),
            Self::KeyMismatch { .. } => {
                nebula_error::ErrorCode::new("RESOURCE:METADATA_KEY_MISMATCH")
            },
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Definition(source) => nebula_error::Classify::is_retryable(source),
            Self::KeyMismatch { .. } => false,
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::Definition(source) => nebula_error::Classify::retry_hint(source),
            Self::KeyMismatch { .. } => None,
        }
    }
}

/// Compatibility validation errors for resource metadata evolution.
///
/// Wraps [`BaseCompatError`] for parity with the action- and
/// credential-side error shapes. Resource has no entity-specific compat
/// rules today, so the enum currently has a single `Base` variant; new
/// variants will be added here (alongside `Base`) if `ResourceMetadata`
/// later gains entity-specific fields whose changes should break version
/// compatibility.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] BaseCompatError<ResourceKey>),
}

/// Why an instance is being torn down — lets a `destroy` impl adapt its
/// graceful-shutdown behavior (e.g. full flush on `Shutdown`, fast abandon on
/// `Revoked`). See ADR-0093.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownReason {
    /// Normal lease end (clean release).
    Released,
    /// Pool eviction (stale fingerprint / max-lifetime / idle / broken).
    Evicted,
    /// The instance's credential was revoked.
    Revoked,
    /// Graceful manager shutdown drain.
    Shutdown,
}

/// Framework-owned teardown context handed to [`Provider::destroy`].
///
/// `deadline` tells the author when the framework will abandon asynchronous
/// teardown. Bound graceful work to it with
/// `tokio::time::timeout_at(cx.deadline.into(), …)`. The public fields can be
/// changed locally, but the framework captures its deadline independently:
/// changing this context cannot extend or disarm that timeout.
///
/// The timeout is cooperative: it cannot preempt a blocking future poll or a
/// blocking destructor. Providers must keep both non-blocking.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct TeardownCx {
    /// The instant by which teardown must complete or be abandoned.
    pub deadline: std::time::Instant,
    /// Why this instance is being torn down.
    pub reason: TeardownReason,
}

impl TeardownCx {
    /// Constructs a teardown context. The framework builds these; exposed for
    /// tests and out-of-crate `Provider` impls.
    #[must_use]
    pub fn new(deadline: std::time::Instant, reason: TeardownReason) -> Self {
        Self { deadline, reason }
    }
}

/// Relative cost of a [`Provider::check`] probe — the framework maintenance
/// reaper uses it to space background health probes so an expensive check is
/// not run every sweep over a pool of idle instances.
///
/// Returned by [`Provider::check_cost`] (default [`Cheap`](Self::Cheap)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckCost {
    /// In-process / O(1) check (a liveness flag, a cached handle state).
    /// Probed every maintenance sweep.
    Cheap,
    /// A check with moderate cost (a local syscall, a cheap handshake).
    /// Probed less often than [`Cheap`](Self::Cheap).
    Moderate,
    /// A network round-trip / `SELECT 1` / remote ping. Probed rarely so idle
    /// instances are not hammered with probe traffic.
    Expensive,
}

impl CheckCost {
    /// How many maintenance sweeps elapse between background probes at this
    /// cost: `Cheap` every sweep, `Moderate` every 4th, `Expensive` every 16th.
    ///
    /// The reaper probes idle slots on sweep `n` iff
    /// `n.is_multiple_of(self.probe_every_n_sweeps())`, so the probe frequency
    /// falls as the cost rises.
    #[must_use]
    pub fn probe_every_n_sweeps(self) -> u64 {
        match self {
            Self::Cheap => 1,
            Self::Moderate => 4,
            Self::Expensive => 16,
        }
    }
}

/// Provider trait — 3 associated types + lifecycle methods (slot model).
///
/// Uses `#[async_trait]` to keep return types uniform with the blanket
/// `impl<R: Provider> ManagedHandle for ManagedResource<R>`, which
/// dispatches through `dyn ManagedHandle` (object-safe, boxed futures).
/// `Provider` itself is not object-safe (`fn key()` has no receiver,
/// `Sized` bound) — the attribute is for the blanket impl's convenience,
/// not for `dyn Provider` dispatch.
///
/// Per slot model (supersedes credential isolation) the singular `type Credential`
/// associated type was removed in favor of typed credential **slot
/// fields** on the resource struct (declared via `#[credential(...)]`
/// field attributes; the `#[derive(Resource)]` macro emits an impl of
/// [`nebula_core::DeclaresDependencies`] enumerating them). Each slot
/// field is a `SlotCell<CredentialGuard<C>>` cell. The framework
/// resolves slot fields **before** calling [`create`](Self::create) —
/// implementors read each resolved guard through the derive-emitted
/// `<field>_slot()` accessor, which returns
/// `Option<Arc<CredentialGuard<C>>>`, never off the raw cell field.
///
/// # Associated types
///
/// | Type | Purpose |
/// |------|---------|
/// | `Config` | Operational config (no secrets) |
/// | `Instance` | The live resource handle (connection, client, etc.) |
/// | `Topology` | The resource's framework-driven entry policy |
///
/// # Lifecycle
///
/// ```text
/// create() → Instance    (slot fields already resolved)
///   ↓
/// check()  → Ok(()) | Err
///   ↓
/// destroy() → flush, stop, close, and join (consumes Instance on final ownership release)
/// ```
// `Sized` is required so `type Topology: Topology<Self>` can name `Self` as the
// topology's `R` (which carries an implicit `Sized` bound). `Provider` is never
// object-safe regardless — `fn key() -> ResourceKey` has no receiver — so no
// `dyn Provider` usage is foreclosed by this.
//
// `HasCredentialSlots` supertrait: every `Provider` must state its
// credential-slot posture, either via `#[derive(Resource)]` (which emits the
// real fold) or the `no_credential_slots!` macro for slot-less types. This
// folds what used to be two independently-implementable traits (a resource
// could implement `Provider` and silently skip `HasCredentialSlots`, leaving
// the framework's rotation/revoke fan-out unable to see it) into one
// compile-time-enforced contract.
#[async_trait]
pub trait Provider: HasCredentialSlots + Send + Sync + Sized + 'static {
    /// Operational configuration type (no secrets).
    type Config: ResourceConfig;
    /// The live resource handle.
    type Instance: Send + Sync + 'static;
    /// The lease topology backing this resource — the framework dispatches
    /// acquire / release / admission through it.
    ///
    /// Pin this to a built-in framework topology
    /// ([`Pooled<Self>`](crate::topology::Pooled) /
    /// [`Resident<Self>`](crate::topology::Resident)) or a custom
    /// [`Topology`](crate::topology::Topology) implementation. Stable Rust has
    /// no per-resource associated-type defaults, so every `impl Provider` must
    /// spell this — `type Topology = Pooled<Self>;`, etc.
    ///
    /// The topology is keyed to the resource (`Topology<Self>`): every topology
    /// needs the R-aware entry lifecycle hooks (`create_entry` produces
    /// `R::Instance`), so the trait carries `R` rather than splitting an
    /// R-agnostic open trait from an R-aware bridge.
    type Topology: crate::topology::Topology<Self>;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new instance from config.
    ///
    /// Credential slot cells declared via `#[credential(key = "...")]`
    /// are already populated on `&self` by the framework before this
    /// call (per slot model). Implementations read each resolved guard
    /// through the derive-emitted `self.<field>_slot()` accessor
    /// (`Option<Arc<CredentialGuard<C>>>`) — handling the `None`
    /// (unbound) case explicitly — never off the raw cell field.
    ///
    /// # Errors
    ///
    /// Map driver errors to [`crate::ErrorKind`] via
    /// `#[derive(ClassifyError)]` so the manager can decide retry:
    ///
    /// - [`Transient`](crate::ErrorKind::Transient) — connect timeout, network blip.
    /// - [`Permanent`](crate::ErrorKind::Permanent) — auth failure, malformed config.
    /// - [`Exhausted { retry_after }`](crate::ErrorKind::Exhausted) — backend rate-limit;
    ///   drives backoff before the next acquire attempt.
    /// - [`Backpressure`](crate::ErrorKind::Backpressure) — your own quota saturated.
    /// - [`Cancelled`](crate::ErrorKind::Cancelled) — observed `ctx.cancel_token()` and aborted.
    ///
    /// # Cancel safety
    ///
    /// `create` MUST be cancel-safe: observing
    /// `ctx.cancel_token().cancelled()` MAY drop the future at any
    /// `.await` point. Any partially-allocated OS resource (socket, temp
    /// file, spawned task) MUST be released in the dropped path —
    /// typically via RAII (`AbortOnDrop` for `JoinHandle`,
    /// `tempfile::TempPath` for transient files).
    async fn create(
        &self,
        config: &Self::Config,
        ctx: &ResourceContext,
    ) -> Result<Self::Instance, crate::Error>;

    /// Called by the engine rotation fan-out after it has swapped the
    /// rotated credential into this resource's slot. `&self`: the resource
    /// impl is an immutable descriptor; blue-green / re-auth acts on
    /// `instance`'s own interior mutability. `slot_name` identifies which
    /// `#[credential]` slot rotated.
    ///
    /// Multi-credential resources can choose to refresh only the affected
    /// sub-system (e.g. swap a single pool, refresh a single header) rather
    /// than recycling the whole instance. Connection-bound resources (Pool,
    /// Service, Transport) typically override with the blue-green swap
    /// pattern: build a fresh pool from the rotated credential, atomically
    /// swap into an `Arc<RwLock<Pool>>`, let RAII drain old handles.
    ///
    /// **Invariant** per slot model §Seam: implementer must handle every
    /// declared credential slot name. `slot_name` is validated before this
    /// hook is ever dispatched: `Manager::refresh_slot` / `taint_slot`
    /// reject a slot name that does not match one of
    /// [`HasCredentialSlots::credential_slot_names`] with a typed
    /// [`Error::unknown_credential_slot`](crate::Error::unknown_credential_slot)
    /// — this hook never observes an undeclared slot.
    ///
    /// Cancellation safety: implementations MUST be cancel-safe — if
    /// the returned future is dropped mid-await, the resource MUST
    /// remain consistent.
    ///
    /// Default: no-op.
    async fn on_credential_refresh(
        &self,
        slot_name: &str,
        instance: &Self::Instance,
    ) -> Result<(), crate::Error> {
        let _ = (slot_name, instance);
        Ok(())
    }

    /// Called by the engine fan-out when a slot's credential is revoked.
    /// Post-invocation invariant (slot model): the resource emits no further
    /// authenticated traffic on the revoked credential. Default: no-op
    /// (the engine still taints + drains the instance around this call).
    ///
    /// # Errors
    ///
    /// Returns `crate::Error` if the instance cannot stop emitting
    /// authenticated traffic on the revoked credential. The manager
    /// surfaces the error as
    /// [`SlotRevokeFailed`](crate::ResourceEvent::SlotRevokeFailed) on
    /// the event channel and emits an inline
    /// [`HealthChanged { healthy: false }`](crate::ResourceEvent::HealthChanged)
    /// so subscribers see the failure even if they filter slot events.
    ///
    /// # Security
    ///
    /// On `Ok(())` the resource guarantees no subsequent traffic uses
    /// the revoked credential. On `Err(_)` the manager treats the
    /// instance as compromised; the row stays tainted until a fresh
    /// credential is bound.
    async fn on_credential_revoke(
        &self,
        slot_name: &str,
        instance: &Self::Instance,
    ) -> Result<(), crate::Error> {
        let _ = (slot_name, instance);
        Ok(())
    }

    /// Health-checks an existing instance.
    ///
    /// The default implementation always succeeds.
    ///
    /// The framework also calls this from its background maintenance probe
    /// (spaced by [`check_cost`](Self::check_cost)). The idle lock is **NOT**
    /// held while `check` runs: the probe briefly locks the idle queue
    /// only to drain it, runs every check concurrently outside that lock,
    /// then returns each survivor through the framework's epoch-fenced
    /// return path (re-checking the revoke epoch under the re-taken lock —
    /// an entry whose credential was revoked mid-probe is destroyed, never
    /// silently re-admitted). A
    /// `check` impl still MUST NOT re-enter the resource manager for the
    /// same resource (acquire / return through a captured `Manager`
    /// handle): this is a **policy** boundary now (an author hook must stay
    /// read-only and side-effect-free), not a lock-reentrancy hazard the
    /// framework depends on to avoid deadlock. Read instance state only.
    ///
    /// # Errors
    ///
    /// Returns `crate::Error` classified as
    /// [`Transient`](crate::ErrorKind::Transient) for a recoverable health
    /// failure (the manager will tear the instance down and let the next
    /// acquire rebuild it) or
    /// [`Permanent`](crate::ErrorKind::Permanent) for a misconfiguration
    /// that no retry will fix.
    async fn check(&self, _instance: &Self::Instance) -> Result<(), crate::Error> {
        Ok(())
    }

    /// Relative cost of a [`check`](Self::check) probe, used by the framework
    /// maintenance reaper to space background health probes.
    ///
    /// A [`Cheap`](CheckCost::Cheap) check (an in-process liveness flag, a
    /// cached handle state) is probed every maintenance sweep; an
    /// [`Expensive`](CheckCost::Expensive) one (a network round-trip, a
    /// `SELECT 1`) is probed far less often, so a pool of idle connections is
    /// not hammered with probe traffic. Advisory only — `check` is still run on
    /// demand wherever correctness requires it (post-checkout validation,
    /// recovery). Default [`Cheap`](CheckCost::Cheap).
    fn check_cost(&self) -> CheckCost {
        CheckCost::Cheap
    }

    /// Budget for [`destroy`](Self::destroy) to flush, drain, stop, and join
    /// one instance's owned work. The framework composes the actual
    /// teardown deadline from this budget and the reason; a `Revoked` teardown
    /// is additionally capped short. Default 30s. See ADR-0093.
    fn teardown_budget(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    /// Optional maximum lease-hold duration — leak / hang detection.
    ///
    /// When `Some(d)`, the framework arms a background watchdog on every
    /// acquired [`ResourceGuard`](crate::guard::ResourceGuard): if the lease is
    /// still held `d` after acquisition, it emits a
    /// [`ResourceEvent::HoldDeadlineExceeded`](crate::events::ResourceEvent::HoldDeadlineExceeded)
    /// and a `WARN` span. The lease is **not** forcibly released — this is an
    /// observability signal (the HikariCP `leakDetectionThreshold` equivalent)
    /// that surfaces a node which acquired a bounded-exclusive lease and hung,
    /// pinning the slot while siblings back up on the semaphore.
    ///
    /// The default is `None` (no watchdog, zero cost). Receiver-less because
    /// the deadline is a resource-type policy, not a per-instance value.
    fn max_hold_duration() -> Option<std::time::Duration> {
        None
    }

    /// The single terminal hook — consumes the instance on final ownership release.
    ///
    /// Put all asynchronous flush, drain, stop, close, and worker-join work here.
    /// Shared Resident leases do not trigger physical teardown until the retained
    /// owner and every lease have released ownership. The framework dispatches
    /// this hook once for the owned instance and never retries it.
    ///
    /// The default implementation only drops the instance. It is appropriate
    /// when synchronous RAII cleanup is sufficient; it performs no asynchronous
    /// shutdown. `Drop` and this process-local hook do not guarantee cleanup
    /// after a process crash.
    ///
    /// # Errors
    ///
    /// Return a typed error if cleanup cannot complete (for example, a worker
    /// failed to join). The instance remains consumed on error: its ownership
    /// is not returned and there is no terminal retry, regardless of the error's
    /// retry classification. The framework logs failures and exposes them through
    /// awaited release or shutdown where that path reports the result.
    ///
    /// # Teardown context
    ///
    /// `cx.deadline` is the instant by which teardown must finish or be
    /// abandoned — an author doing graceful work (`flush`/`drain`/`close`)
    /// should bound it via `tokio::time::timeout_at(cx.deadline.into(), …)` so it
    /// composes with the framework's per-resource backstop (derived from
    /// [`teardown_budget`](Self::teardown_budget)). `cx.reason` says why the
    /// instance is going away ([`TeardownReason`]), letting an impl adapt
    /// (full flush on `Shutdown`, fast abandon on `Revoked`).
    ///
    /// # Cancel safety
    ///
    /// `destroy` typically runs through
    /// [`ReleaseQueue`](crate::ReleaseQueue) so caller-side `Drop` is
    /// non-blocking. It MUST tolerate running after the manager's cancel
    /// token has fired; do not abort if you observe cancellation.
    ///
    /// The future may be dropped at any await when its budget expires or its
    /// runtime stops. Owned tasks and handles therefore need a synchronous
    /// drop fallback (for example, abort-on-drop ownership for spawned tasks).
    /// Dropping a bare task handle does not stop its task. Keep polls and Drop
    /// non-blocking: the cooperative timeout cannot preempt either.
    async fn destroy(&self, instance: Self::Instance, cx: TeardownCx) -> Result<(), crate::Error> {
        let _ = (instance, cx);
        Ok(())
    }

    /// Returns schema-free author intent for UI and diagnostics.
    ///
    /// The resource factory derives and binds the canonical configuration
    /// schema from [`Self::Config`]. Authors cannot supply an arbitrary schema.
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
}

/// Credential-slot epoch provider — implemented by `#[derive(Resource)]`.
///
/// An order-sensitive positional fold over every credential slot's generation.
/// `0` = no slot ever bound (also the only value for slot-less resources).
///
/// **Contract:** the returned value **changes whenever ANY slot's generation
/// changes** — not just the slot with the largest generation. It is compared
/// **only for equality** by the create-vs-rotate reconcile (built-epoch vs
/// live-epoch), never by magnitude, so it is a change-token rather than a
/// monotone counter.
///
/// `#[derive(Resource)]` emits the real implementation: an
/// order-sensitive positional fold
/// (`acc = acc * K + slot.generation()`, fixed odd `K`) over every
/// declared `#[credential]` field's
/// [`SlotCell::generation`](crate::SlotCell::generation). A plain
/// `max` would be wrong here — an instance built at
/// `(slot_a=5, slot_b=10)` then rotated `slot_a→6` still maxes to
/// `10`, so the reconcile would miss the now-stale instance; the
/// positional fold changes on every slot transition regardless of
/// which slot moved. Slot-less resources always return `0`.
pub trait HasCredentialSlots {
    /// Order-sensitive positional fold over every credential slot's generation.
    /// `0` = no slot ever bound (also the only value for slot-less resources).
    fn credential_slot_epoch(&self) -> u64;

    /// Whether this resource TYPE declares any `#[credential]` slot field.
    ///
    /// Distinct from [`credential_slot_epoch`](Self::credential_slot_epoch),
    /// which is `0` both for a slot-less resource and for a declared-but-unbound
    /// slot and so cannot answer this at the type level. The derive emits `true`
    /// when the struct has at least one `#[credential]` field. **Required, no
    /// default:** a slot-less hand-written impl must say `false`
    /// explicitly — use [`no_credential_slots!`](crate::no_credential_slots) for
    /// the honest one-line zero impl rather than silently inheriting a default,
    /// so a resource can never implement [`Provider`] without having stated its
    /// credential-slot posture. The framework uses it to nudge credentialed
    /// Pooled resources toward a session-state-wiping `recycle` (see ADR-0093
    /// foolproofing Tier-3).
    fn declares_credential_slots() -> bool;

    /// Names of every declared `#[credential]` slot field, for unknown-slot
    /// validation.
    ///
    /// `#[derive(Resource)]` emits the real list — the `key = "..."` (or
    /// field name) of every `#[credential]` field, in declaration order.
    /// Hand-written impls default to empty.
    ///
    /// [`Manager::refresh_slot`](crate::Manager::refresh_slot) /
    /// [`taint_slot`](crate::Manager::taint_slot) (and their `_for_identity`
    /// / `revoke_slot*` counterparts) validate every incoming slot-name
    /// argument against this list, unconditionally — a slot-less type (empty
    /// list, `declares_credential_slots() -> false`) therefore rejects every
    /// slot name with [`Error::unknown_credential_slot`](crate::Error::unknown_credential_slot);
    /// there is no name that reaches the dispatch hook unchecked. A type
    /// that declares credential slots must declare their names here too —
    /// leaving this at the empty default while overriding
    /// `declares_credential_slots() -> true` makes every rotation/revoke
    /// call against it fail closed the same way.
    fn credential_slot_names() -> &'static [&'static str] {
        &[]
    }
}

/// Emits the honest zero [`HasCredentialSlots`] impl for a resource with no
/// `#[credential]` slot fields.
///
/// `Provider` requires `HasCredentialSlots`, and
/// [`declares_credential_slots`](HasCredentialSlots::declares_credential_slots)
/// has no default — every hand-written `impl Provider` must state its
/// credential-slot posture. `#[derive(Resource)]` does this for you when the
/// struct has `#[credential]` fields; for a resource with none, call this
/// macro once instead of hand-rolling the same three-line impl. Because
/// [`credential_slot_names`](HasCredentialSlots::credential_slot_names) also
/// defaults to empty, every rotation/revoke entry point on the resulting
/// type rejects every slot name with
/// [`Error::unknown_credential_slot`](crate::Error::unknown_credential_slot) —
/// there is no slot to rotate, so there is no name that should ever be
/// accepted:
///
/// ```
/// use nebula_resource::no_credential_slots;
///
/// struct MyResource;
///
/// no_credential_slots!(MyResource);
/// ```
///
/// expands to:
///
/// ```
/// # struct MyResource;
/// impl nebula_resource::HasCredentialSlots for MyResource {
///     fn credential_slot_epoch(&self) -> u64 {
///         0
///     }
///     fn declares_credential_slots() -> bool {
///         false
///     }
/// }
/// ```
#[macro_export]
macro_rules! no_credential_slots {
    ($ty:ty) => {
        impl $crate::HasCredentialSlots for $ty {
            fn credential_slot_epoch(&self) -> u64 {
                0
            }
            fn declares_credential_slots() -> bool {
                false
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use nebula_core::resource_key;
    use nebula_metadata::BaseCompatError;
    use nebula_schema::ValidSchema;
    use semver::Version;

    use super::{
        MetadataCompatibilityError, RecordedResourceMetadata, ResourceMetadata,
        ResourceMetadataDraft,
    };

    fn empty_schema() -> ValidSchema {
        ValidSchema::empty()
    }

    fn md(major: u64, minor: u64) -> ResourceMetadata {
        ResourceMetadataDraft::new(resource_key!("postgres"), crate::metadata_name!("pg"), "d")
            .with_version(Version::new(major, minor, 0))
            .admit(empty_schema())
    }

    #[test]
    fn metadata_equality_tracks_version() {
        let a = md(1, 2);
        let b = md(1, 2);
        assert_eq!(a, b);

        let c = md(1, 3);
        assert_ne!(a, c, "different minor version must break equality");
    }

    #[test]
    fn recorded_metadata_roundtrip_requires_fresh_readmission() {
        let original = md(2, 1);

        let json = serde_json::to_string(&original).expect("serialization succeeds");
        let json_value: serde_json::Value =
            serde_json::from_str(&json).expect("serialized metadata is valid JSON");
        assert!(
            json_value.get("base").is_none(),
            "base metadata must be flattened"
        );
        assert_eq!(
            json_value.get("key").and_then(serde_json::Value::as_str),
            Some("postgres")
        );

        let recorded: RecordedResourceMetadata =
            serde_json::from_str(&json).expect("recorded evidence deserializes");
        let readmitted = recorded
            .readmit_against(&original)
            .expect("matching evidence readmits against the fresh definition");
        assert_eq!(original, readmitted);
    }

    #[test]
    fn full_version_preserves_all_semver_components() {
        let version = Version::parse("2.1.3-alpha.1+build.7").expect("valid test version");
        let metadata =
            ResourceMetadataDraft::new(resource_key!("postgres"), crate::metadata_name!("pg"), "d")
                .with_version(version.clone())
                .admit(empty_schema());

        assert_eq!(metadata.base().version(), &version);
    }

    #[test]
    fn version_monotonic_accepted() {
        let prev = md(1, 0);
        let next = md(1, 1);
        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn version_regression_rejected() {
        let prev = md(2, 1);
        let next = md(2, 0);
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(
            matches!(
                err,
                MetadataCompatibilityError::Base(BaseCompatError::VersionRegressed { .. })
            ),
            "version regression must retain its typed compatibility failure"
        );
    }
}
