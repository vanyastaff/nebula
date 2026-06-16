---
name: project-plugin-dependency-resolver-d6
description: ADR-0095 D6 plugin-dependency resolver — where the field/resolver/error live, and WHY wiring is a follow-up (no batch plugin-load call site exists)
metadata:
  type: project
---

ADR-0095 **D6** (RATIFIED, phase-1 unit): plugin-to-plugin dependency declaration + fail-closed topological resolver.

- **Manifest field**: `dependencies: Vec<PluginDependency>` on `PluginManifest` (`crates/metadata/src/manifest.rs`); `PluginDependency { key: PluginKey, req: semver::VersionReq }` lives in `nebula-metadata` next to the manifest. Reuses `PluginKey` (nebula-core) + `semver::VersionReq` — both already deps of nebula-metadata (semver) / precedent in `nebula-plugin` (`plugin_toml.rs` already parses `VersionReq` matched against `Version`). No native version type, no new crate dep.
- **Resolver**: `PluginRegistry::resolve_load_order(&self) -> Result<Vec<PluginKey>, PluginDependencyError>` method in `crates/plugin/src/` (new `dependency.rs` module), white/grey/black DFS over manifests already in the registry. Returns topo order (dependency-before-dependent).
- **Error**: new `PluginDependencyError` thiserror enum in nebula-plugin (MissingDependency / VersionMismatch{required,found} / Cycle{path}). Separate from `PluginError` (different failure domain — registry-graph, not single-plugin construction).

**Why wiring is a NAMED FOLLOW-UP, not done now (load-path finding, verified 2026-06-15):**
The engine holds ONE `PluginRegistry` and populates it one-plugin-at-a-time via `plugin_registry_mut()` + `register(Arc<ResolvedPlugin>)` (`engine.rs:371/466`). There is **no batch "load all plugins / compute load order" entrypoint anywhere** — not in engine, not in any binary. Only producers of registry contents are tests. The api holds it as read-only `Option<Arc<RwLock<PluginRegistry>>>` catalog (`api/src/state.rs:236`). A resolver that consumes a fully-populated registry has no clean non-latent call site; wiring it would mean inventing the batch-load/worker-build path, which is explicitly D1/orchestrator scope. So resolver ships unit-tested in nebula-plugin; **follow-up = call `resolve_load_order` at the future batch plugin-load seam (worker build / orchestrator) and register in returned order.**

**Why:** the registration model is incremental-register, not load-set; the topo resolver is a load-SET operation.
**How to apply:** when the worker-build / orchestrator batch-load unit lands, that is where the resolver wires in — register plugins in `resolve_load_order()` order and fail-closed on its error. Don't add a latent caller in the engine before then.

Related: [[project_inprocess_registry_pivot]], [[project_adr0095_spikes]].
