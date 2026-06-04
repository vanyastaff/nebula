//! Registry CRUD: the single typed `register` funnel, the JSON-driven
//! `register_resolved` entry, the shared config-validation seam, typed
//! lookup, hot config reload, and removal.

use std::sync::{Arc, atomic::AtomicU64};

use nebula_core::{ResourceKey, ScopeLevel};
use tokio::sync::Notify;
use tracing::Instrument as _;

use super::{ErasedAcquireFn, Manager, RegistrationSpec, resolve_json_templates};
use crate::{
    error::Error,
    events::ResourceEvent,
    recovery::gate::RecoveryGate,
    reload::ReloadOutcome,
    resource::Resource,
    runtime::{TopologyRuntime, managed::ManagedResource},
};

impl Manager {
    /// Registers a resource from a fully-specified [`RegistrationSpec`].
    ///
    /// This is the **single registration funnel**: the former 3-deep
    /// `register` → `register_with_identity` → `register_with_slot_identity`
    /// → internal-row-builder chain and the ~17 per-topology
    /// `register_<topo>[_with]` shorthands all collapse onto this one
    /// method fed by one struct. Callers that only need the historical
    /// single-row-per-`(key, scope)` behaviour pass
    /// [`RegistrationSpec::slot_identity`] =
    /// [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity).
    ///
    /// Per slot model the `spec.resource` value is expected to have **all
    /// `#[credential]` slot fields already resolved and populated**.
    /// `Manager::register` does not itself resolve credential bindings —
    /// that is the responsibility of the caller (typically the engine
    /// dispatch layer that assembles `R` via the `FromConfig` trait emitted
    /// by `#[derive(Resource)]`).
    ///
    /// `spec.slot_identity` is the structural anti-bleed seam: two
    /// registrations of the same resource type at the same `spec.scope`
    /// whose resolved `(slot, credential)` bindings differ occupy
    /// **distinct** registry rows with **distinct** topology runtimes, so
    /// one tenant's runtime can never serve another tenant's resolved
    /// credential. Equality is exact and structural (no digest), so two
    /// distinct resolved binding sets can never alias.
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key, scope,
    /// and slot identity is already registered, it is silently replaced.
    /// The manager's internal [`ReleaseQueue`](crate::ReleaseQueue) is automatically shared with
    /// the managed resource — callers never need to create or manage it.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails on the provided config.
    pub fn register<R: Resource>(&self, spec: RegistrationSpec<R>) -> Result<(), Error> {
        use crate::resource::ResourceConfig as _;

        let RegistrationSpec {
            resource,
            config,
            scope,
            slot_identity,
            topology,
            acquire,
            recovery_gate,
        } = spec;

        config.validate()?;

        // #390 (pool min/max sanity) is enforced at `PoolRuntime`
        // construction, which the caller has already invoked to build the
        // `TopologyRuntime::Pool` handed in here. No separate
        // register-time pool-config check is needed: an invalid
        // `(min_size, max_size)` from operator/JSON config is rejected by
        // the fallible `PoolRuntime::try_new` (typed `Error::permanent`)
        // that the engine registrar uses to construct the runtime, so the
        // failure surfaces *before* this funnel as a registration error
        // rather than an abort. (The deleted `register_pooled[_with]`
        // shorthands re-validated the raw config only because they took
        // it *before* building the runtime.)

        let key = R::key();

        // Wire the manager's event bus into the optional recovery gate so its
        // state transitions emit `ResourceEvent::RecoveryGateChanged`.
        // Idempotent at the `RecoveryGate` end: a gate handed to a second
        // manager (test composition, scoped registry) keeps its first sink
        // and ignores this call. Cheap and lock-free — `OnceLock::set` is one
        // CAS over a cloned `Arc`.
        if let Some(gate) = recovery_gate.as_deref() {
            gate.set_event_sink(Arc::clone(&self.event_bus), key.clone());
        }

        let managed = Arc::new(ManagedResource {
            resource,
            config: arc_swap::ArcSwap::from_pointee(config),
            topology,
            release_queue: Arc::clone(&self.release_queue),
            generation: AtomicU64::new(0),
            status: arc_swap::ArcSwap::from_pointee(crate::state::ResourceStatus::new()),
            recovery_gate,
            tainted: std::sync::atomic::AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        });

        let type_id = std::any::TypeId::of::<ManagedResource<R>>();
        self.registry.register(
            key.clone(),
            type_id,
            scope,
            slot_identity,
            managed.clone(),
            acquire,
        );

        // #387: everything below this point is a single funnel — the
        // resource is installed, so advance its phase from `Initializing`
        // to `Ready`. Failures are surfaced by `config.validate()` above,
        // which aborts before we reach this line.
        managed.set_phase(crate::state::ResourcePhase::Ready);

        // Start the background idle/lifetime reaper for pools that expire
        // instances. No-op for non-pool topologies and for pools with no
        // TTL configured (zero background overhead in that case).
        self.spawn_pool_maintenance(&managed);

        if let Some(m) = &self.metrics {
            m.record_create();
        }
        self.emit(ResourceEvent::Registered { key: key.clone() });

        tracing::debug!(%key, "resource registered");
        Ok(())
    }

    /// Spawns the background maintenance reaper for a freshly-registered
    /// pool — the **sole production driver of idle eviction**.
    ///
    /// [`PoolRuntime::run_maintenance`](crate::runtime::pool::PoolRuntime::run_maintenance)
    /// evicts idle-timed-out, max-lifetime-exceeded, stale-fingerprint, and
    /// revoked idle instances, but nothing calls it on its own. The return
    /// path enforces `max_lifetime` opportunistically, yet an instance that
    /// goes idle and simply ages out past `idle_timeout` is never swept
    /// unless this reaper runs — so without it `idle_timeout` is dead config.
    ///
    /// Spawned **only** for the [`Pool`](TopologyRuntime::Pool) topology and
    /// **only** when a TTL (`idle_timeout` or `max_lifetime`) is configured,
    /// so pools that never expire instances pay zero background cost (bb8's
    /// guard). The task:
    /// - holds only a [`Weak`](std::sync::Weak) to the
    ///   [`ManagedResource`], so it never keeps the resource alive — a
    ///   `remove()` drops the last strong ref and the reaper exits on its
    ///   next tick;
    /// - selects on the manager's cancellation token, so `shutdown` /
    ///   `graceful_shutdown` stops it promptly;
    /// - runs each sweep *outside* the cancel `select!` so a cancellation
    ///   cannot drop a maintenance future mid-eviction.
    ///
    /// Both the tick cadence and the TTL thresholds (`idle_timeout` /
    /// `max_lifetime`) are fixed at registration — they live on the pool's
    /// immutable topology [`Config`](crate::topology::pooled::config::Config),
    /// which [`reload_config`](Self::reload_config) does **not** touch (it
    /// swaps the resource-level `R::Config` and bumps the pool fingerprint,
    /// never the pool topology config). The stale-fingerprint and
    /// credential-revoke eviction arms *are* read live on every sweep (both
    /// consult atomics updated by `reload_config` / `revoke_slot`), so a
    /// reload still evicts stale-fingerprint instances and a revoke still
    /// evicts revoked ones on the next sweep — only the TTL *durations* are
    /// frozen for the pool's lifetime.
    fn spawn_pool_maintenance<R: Resource>(&self, managed: &Arc<ManagedResource<R>>) {
        let TopologyRuntime::Pool(pool) = &managed.topology else {
            return;
        };
        let cfg = pool.config();
        if cfg.idle_timeout.is_none() && cfg.max_lifetime.is_none() {
            return;
        }
        // `tokio::time::interval` panics on a zero period; clamp so an
        // operator-supplied `maintenance_interval` of zero degrades to a
        // tight-but-valid sweep rather than panicking the reaper task.
        let period = cfg
            .maintenance_interval
            .max(std::time::Duration::from_millis(1));
        let weak = Arc::downgrade(managed);
        let cancel = self.cancel.clone();
        let bus = Arc::clone(&self.event_bus);
        let key = R::key();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // The first tick fires immediately; consume it so the first real
            // sweep lands one full period after registration.
            ticker.tick().await;
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                // Upgrade to a strong ref only for the duration of the sweep.
                // If the registry has dropped the row (`remove`), the upgrade
                // fails and the reaper exits — it never extends the resource's
                // lifetime past removal. The sweep runs *after* the `select!`
                // so a cancellation cannot drop it mid-eviction (a finite,
                // bounded operation over the current idle set).
                let Some(managed) = weak.upgrade() else {
                    break;
                };
                if let TopologyRuntime::Pool(pool) = &managed.topology {
                    let span = tracing::debug_span!("pool_maintenance", %key);
                    let evicted = pool
                        .run_maintenance(&managed.resource)
                        .instrument(span)
                        .await;
                    if evicted > 0 {
                        let _ = bus.emit(ResourceEvent::MaintenanceEvicted {
                            key: key.clone(),
                            evicted,
                        });
                    }
                }
            }
        });
    }

    /// Schema-validate an **already-resolved** config JSON tree against
    /// `<R::Config as HasSchema>::schema()` *without* registering anything.
    ///
    /// This is the pure validation core shared with
    /// [`register_resolved`](Self::register_resolved): it runs exactly
    /// the schema pass, the closed-set guard, and the `R::Config`
    /// deserialize step that the live path runs *after* template
    /// resolution — but performs **no** `{{ … }}` resolution, **no**
    /// `Manager` mutation, and constructs **no** `resource: R` /
    /// `TopologyRuntime<R>`. It is the seam a config-CRUD writer uses to
    /// reject a bad `ResourceEntry.config` *before* persistence, keeping
    /// config validation strictly separate from engine-activation live
    /// registration (INTEGRATION_MODEL integration seam.1 — live registration happens
    /// at engine activation, never at config-create time).
    ///
    /// Template resolution is deliberately excluded: `{{ … }}` is resolved
    /// against the engine's expression context at activation, which does
    /// not exist at config-create time. A stored config may legitimately
    /// still carry unresolved templates; validating the *post-resolution*
    /// shape is an activation-time concern.
    ///
    /// On success returns the validated, deserialized `R::Config`: the
    /// closed-set guard and `serde_json::from_value::<R::Config>` already
    /// run here, so the live `register_resolved` path consumes this
    /// owned value directly instead of deserializing the same JSON twice.
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when the JSON is not a field tree, fails the
    ///   `R::Config` schema (missing/invalid declared fields, `#[validate]`
    ///   rules), or fails to deserialize into `R::Config`.
    /// - [`Error::permanent`] when the config carries a top-level field the
    ///   `R::Config` schema does not declare (closed-set guard):
    ///   `ResourceConfig` must carry no secrets, so an inlined
    ///   secret-shaped field is rejected here rather than silently ignored
    ///   (product credential boundary). The error names only the offending key,
    ///   never its value.
    pub fn validate_config_value<R>(config_json: serde_json::Value) -> Result<R::Config, Error>
    where
        R: Resource,
        R::Config: serde::de::DeserializeOwned,
    {
        // Schema-validate against <R::Config as HasSchema>::schema(). This is
        // independent of serde::Deserialize: it surfaces missing/invalid fields a
        // serde default impl would silently accept, and runs the schema's
        // `#[validate(...)]` rules (length, pattern, …). Schema check runs FIRST so
        // structural errors are reported as schema violations rather than
        // confusingly re-routed through serde.
        let schema = <R::Config as nebula_schema::HasSchema>::schema();
        let field_values =
            nebula_schema::FieldValues::from_json(config_json.clone()).map_err(|e| {
                Error::permanent(format!("validate_config_value: invalid field tree: {e}"))
            })?;
        if let Err(report) = schema.validate(&field_values) {
            return Err(Error::permanent(format!(
                "validate_config_value: schema validation failed: {report:?}"
            )));
        }

        // Closed-set guard: reject any config key the typed `R::Config` schema does
        // not declare. `nebula_schema::Schema::validate` only checks *declared*
        // fields and silently ignores unknown ones, so without this an operator
        // could inline a secret-shaped field (e.g. `password`) into
        // `ResourceConfig` and get no signal — `ResourceConfig` must carry no
        // secrets; secrets reach a resource ONLY via typed credential slots
        // (product credential boundary; slot model; engine credential orchestration redaction; credential isolation
        // isolation). The error names only the offending KEY, never its value, so
        // a mis-wired secret can never leak through the rejection message.
        //
        // Skipped when the schema declares no fields: an empty `ValidSchema` is
        // the "schema not yet declared" sentinel (`impl_empty_has_schema!`), and a
        // closed set over zero fields would reject every config — that gate
        // belongs to types that have opted into a real schema.
        let declared = schema.fields();
        if !declared.is_empty()
            && let Some((unknown, _)) = field_values
                .iter()
                .find(|(k, _)| !declared.iter().any(|f| f.key() == *k))
        {
            return Err(Error::permanent(format!(
                "validate_config_value: config field `{unknown}` is not declared by \
                 the `{ty}` schema; secrets must not be inlined into ResourceConfig \
                 — bind them through a typed credential slot instead \
                 (product credential boundary)",
                unknown = unknown.as_str(),
                ty = std::any::type_name::<R::Config>(),
            )));
        }

        // Deserialize R::Config from the JSON to surface any residual
        // type-shape mismatch the structural schema pass did not, and
        // return the parsed value: the live `register_resolved` path
        // consumes this owned `R::Config` directly, so the JSON is
        // deserialized exactly once across validation + typed dispatch.
        serde_json::from_value::<R::Config>(config_json).map_err(|e| {
            Error::permanent(format!(
                "validate_config_value: failed to deserialize {ty} config from JSON: {e}",
                ty = std::any::type_name::<R::Config>()
            ))
        })
    }

    /// JSON-driven registration keyed by the **collision-free structural**
    /// resolved-credential identity.
    ///
    /// The JSON-driven registration entry: it resolves `{{ … }}` templates,
    /// schema-validates, and dispatches into the single
    /// [`register`](Self::register) funnel. Phase order: slot-binding
    /// validation → `{{ … }}` template resolution → schema + closed-set
    /// guard + `R::Config` deserialize → dispatch into the single funnel.
    /// The registry row is keyed by the structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) derived from the
    /// resolved `(slot, credential)` bindings via
    /// [`SlotIdentity::from_bindings`](crate::dedup::SlotIdentity::from_bindings)
    /// — collision-free by exact string equality (no digest). Two
    /// registrations whose resolved bindings differ are distinct rows by
    /// construction, eliminating the cross-tenant-bleed failure mode a
    /// digest exposes rather than shrinking it.
    ///
    /// The derived structural identity is **returned** so the caller (the
    /// engine activation loop) records it for the acquire path and the
    /// rotation fan-out reverse index, addressing the *same* registry row
    /// this method created. The erased `acquire` hook is passed by value
    /// (not a `Fn(slot_id)` factory): the single-walk acquire resolution
    /// pins the row by the *caller's* runtime slot identity, so the
    /// registration-time identity no longer parameterises the hook.
    ///
    /// `nebula-resource → nebula-expression` is allowed under deny.toml's
    /// `[[bans]]` `nebula-resource` wrapper allowlist (Business → Core layer
    /// edge per typed ref fields / Phase 9, R-040 R8).
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when expression resolution, JSON
    ///   deserialization, or schema validation fails.
    /// - [`Error::permanent`] when the config carries a top-level field the
    ///   `R::Config` schema does not declare (closed-set guard):
    ///   `ResourceConfig` must carry no secrets, so an inlined secret-shaped
    ///   field is rejected here rather than silently ignored (product
    ///   credential boundary). The error names only the offending key, never
    ///   its value.
    /// - [`Error::permanent`] when a `slot_bindings` key does not correspond
    ///   to a declared credential slot on `R`.
    /// - Any [`Error`](Error) returned by the underlying typed
    ///   [`register`](Self::register).
    #[tracing::instrument(
        level = "debug",
        target = "nebula_resource::register_resolved",
        skip_all,
        fields(
            resource_key = %R::key(),
            slot_count = slot_bindings.len(),
        )
    )]
    // guard-justified: the production engine registrar dispatches into this positionally (config_json + expr_engine + slot_bindings + resource + scope + topology + acquire + recovery_gate), so the 8-param JSON-driven shape is the engine ABI — collapsing it into a struct would re-introduce the navigation hop the single funnel removed and is not warranted for the one erased call site.
    #[allow(
        clippy::too_many_arguments,
        reason = "engine-facing JSON-driven structural-identity entry: the production engine registrar calls register_resolved positionally; collapsing the 8-param shape into a struct would re-introduce a navigation hop for the one erased call site, and the body itself builds one RegistrationSpec and delegates to the single register() funnel"
    )]
    pub async fn register_resolved<R>(
        &self,
        config_json: serde_json::Value,
        expr_engine: &nebula_expression::ExpressionEngine,
        slot_bindings: std::collections::HashMap<String, nebula_core::CredentialKey>,
        resource: R,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        acquire: ErasedAcquireFn,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<crate::dedup::SlotIdentity, Error>
    where
        R: Resource + nebula_core::DeclaresDependencies,
        R::Config: serde::de::DeserializeOwned,
    {
        // 0. Validate that every binding matches a declared credential slot.
        //    Hard error on unknown slot — refuses to register a resource
        //    whose credential surface diverged from the one the workflow
        //    JSON specified, so misconfiguration surfaces at register time
        //    rather than as a confusing rotation no-op later.
        let deps = R::dependencies();
        for slot_name in slot_bindings.keys() {
            let known = deps.slot_fields().iter().any(|sf| {
                sf.slot_key == slot_name.as_str()
                    && matches!(
                        sf.kind,
                        nebula_core::dependencies::SlotKind::Credential { .. }
                    )
            });
            if !known {
                return Err(Error::permanent(format!(
                    "register_resolved: slot binding `{slot_name}` does not match any declared credential slot on `{}`",
                    std::any::type_name::<R>()
                )));
            }
        }

        // 1. Resolve `{{ … }}` templates inside the JSON tree.
        let ctx = nebula_expression::EvaluationContext::new();
        let resolved = resolve_json_templates(config_json, expr_engine, &ctx)?;

        // 2/2b/3. Schema pass + closed-set guard + `R::Config` deserialize.
        //    Shared verbatim with the config-CRUD validate seam via
        //    [`validate_config_value`](Self::validate_config_value) so the
        //    two paths cannot drift.
        let config: R::Config = Self::validate_config_value::<R>(resolved)?;

        // 4. Derive the **collision-free structural** slot identity from the
        //    resolved slot bindings. Equality is exact string equality over
        //    the canonical-sorted `(slot, credential)` pairs, so two
        //    registrations whose resolved credentials differ are distinct
        //    rows by construction (no digest, no collidable space). This is
        //    the structural barrier against cross-tenant runtime bleed
        //    (credential isolation, slot model). It carries no secret bytes
        //    — only a stable identity over the resolved binding *names*.
        let slot_identity = crate::dedup::SlotIdentity::from_bindings(
            slot_bindings
                .iter()
                .map(|(slot, cred)| (slot.as_str(), cred.as_str())),
        );

        // 5. Dispatch into the single typed register funnel via a
        //    `RegistrationSpec`. ResourceConfig::validate() runs inside
        //    `register`, so domain-level rules (PoolConfig sanity, host
        //    non-empty) are still enforced.
        tracing::debug!(
            target: "nebula_resource::register_resolved",
            ?slot_identity,
            "all pre-register checks passed; dispatching into typed register"
        );
        self.register(RegistrationSpec {
            resource,
            config,
            scope,
            slot_identity: slot_identity.clone(),
            topology,
            acquire,
            recovery_gate,
        })?;
        Ok(slot_identity)
    }

    /// Looks up a registered `ManagedResource<R>` by type and scope.
    ///
    /// This is the building block for acquire: callers retrieve the managed
    /// resource and then call the topology-specific acquire method directly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered for the given scope.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    pub fn lookup<R: Resource>(
        &self,
        scope: &ScopeLevel,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        Self::resolve_typed::<R>(self.registry.get_typed::<R>(scope))
    }

    /// Hot-reloads the configuration for a registered resource.
    ///
    /// Validates the new config, swaps it into the [`ArcSwap`](arc_swap::ArcSwap),
    /// increments the generation counter, and — for pool topologies — updates the
    /// fingerprint so idle instances with stale configs are evicted on next
    /// acquire or release.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered for the given scope.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if config validation fails.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shut down.
    pub fn reload_config<R: Resource>(
        &self,
        new_config: R::Config,
        scope: &ScopeLevel,
    ) -> Result<ReloadOutcome, Error> {
        use crate::resource::ResourceConfig as _;

        new_config.validate()?;

        let managed = self.lookup::<R>(scope)?;

        // Fingerprint comparison — bail early if nothing changed.
        let new_fp = new_config.fingerprint();
        let old_fp = managed.config.load().fingerprint();
        if new_fp == old_fp {
            return Ok(ReloadOutcome::NoChange);
        }

        // #387: visible `Reloading` phase for operators polling health
        // mid-swap.
        managed.set_phase(crate::state::ResourcePhase::Reloading);

        // Atomically swap the config.
        managed.config.store(Arc::new(new_config));

        // Update pool fingerprint so stale idle instances are evicted.
        if let TopologyRuntime::Pool(ref pool_rt) = managed.topology {
            pool_rt.set_fingerprint(new_fp);
        }

        // Bump generation — readers snapshot this to detect changes.
        managed
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);

        // #387: return to `Ready` after publishing the new atomic
        // generation so pollers see the phase transition alongside the
        // config change. `health_check` reads the atomic directly, but
        // `ResourceStatus.generation` is also refreshed by `set_phase`
        // so `status()` snapshots stay self-consistent.
        managed.set_phase(crate::state::ResourcePhase::Ready);

        self.emit(ResourceEvent::ConfigReloaded { key: R::key() });

        // Reload outcome. `reload_config` swaps the config `ArcSwap`
        // without rebuilding the caller-supplied live `Arc<R::Runtime>` for
        // *any* topology — only the Pool fingerprint is updated, above. So
        // the honest outcome is `SwappedImmediately` for every variant: the
        // config is swapped, the live runtime is not rebuilt. The genuine
        // "drain + rebuild the live runtime on reload" behavior is the
        // separately-tracked deferred `reload_config` redesign ([#712]).
        let outcome = ReloadOutcome::SwappedImmediately;

        tracing::info!(key = %R::key(), ?outcome, "resource config reloaded");
        Ok(outcome)
    }

    /// Removes a resource from the registry by key.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if
    /// the key is not registered.
    pub fn remove(&self, key: &ResourceKey) -> Result<(), Error> {
        if !self.registry.remove(key) {
            return Err(Error::not_found(key));
        }

        if let Some(m) = &self.metrics {
            m.record_destroy();
        }
        self.emit(ResourceEvent::Removed { key: key.clone() });
        tracing::debug!(%key, "resource removed");
        Ok(())
    }
}
