---
id: 0027
title: plugin-load-path-stable
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [plugin, trait, registry, metadata, wire-protocol, canon-3.5, canon-7.1, canon-13.1]
related:
  - docs/adr/0006-sandbox-phase1-broker.md
  - docs/adr/0018-plugin-metadata-to-manifest.md
  - docs/adr/0025-sandbox-broker-rpc-surface.md
  - docs/superpowers/specs/2026-04-20-plugin-load-path-stable-design.md
linear: []
---

# 0027. Plugin trait canonical; `ResolvedPlugin` wraps per plugin; `PluginManifest` in `nebula-metadata`; multi-version runtime dropped

## Context

ADR-0018 renamed `PluginMetadata` → `PluginManifest` and established the
`nebula-metadata` consolidation story. That PR was the first half; this ADR
records the second.

After ADR-0018 merged, three problems remained open:

**1. `Plugin` trait returned descriptors, not runnable objects.**
`Plugin::actions()` returned `Vec<ActionDescriptor>` — flat `{ key, name,
description }` structs in `nebula-plugin::descriptor`. Canon §3.5 is explicit:
`Plugin = [ registry: Actions + Resources + Credentials ]` where each leaf is a
runnable `*Metadata + Schema` object, not a descriptor. The trait honored canon
in prose but not in types.

**2. `PluginManifest` lived in `nebula-plugin`, unreachable by the plugin-author SDK.**
Canon §7.1 requires the plugin-author side (`nebula-plugin-sdk`) to carry zero
intra-workspace dependencies on engine-side crates. `nebula-plugin` is
Business-layer; `nebula-plugin-sdk` cannot depend on it. The SDK therefore
shipped its own `PluginMeta` builder — a parallel type for the same concept,
diverging with every structural change to `PluginManifest`.

**3. A runtime multi-version registry with zero production consumers.**
`nebula-plugin` contained `enum PluginType { Single(Arc<dyn Plugin>),
Versions(PluginVersions) }` and a `PluginVersions` struct modeling
n8n-style browser-side version selection. No production call site ever called
`::versioned()` or `add_version`. The abstraction made `PluginRegistry`'s
types opaque and complicated the entire registration path.

L2 §13.1 mandates that on load, Actions / Resources / Credentials from
`impl Plugin` appear in the catalog without a second manifest. That invariant
was honored end-to-end for in-process plugins only if you manually
coordinated the descriptor → metadata mapping yourself; there was no
enforcement surface that caught namespace violations or duplicate keys before
the registry.

The ADR-0018 follow-up entry in its own `Follow-ups` section called out the
`PluginManifest` move to `nebula-metadata` as the planned resolution.

## Decision

1. **`nebula-plugin::Plugin` trait returns runnable trait objects.**
   `fn actions() -> Vec<Arc<dyn Action>>`, `fn credentials() ->
   Vec<Arc<dyn AnyCredential>>`, `fn resources() -> Vec<Arc<dyn AnyResource>>`.
   The `descriptor` module (`ActionDescriptor`, `CredentialDescriptor`,
   `ResourceDescriptor`) is deleted entirely. `nebula-plugin` gains direct
   Business-layer sibling deps on `nebula-action`, `nebula-credential`,
   `nebula-resource`; no cycle exists (verified in the design spec's
   architectural-fit record — none of those three crates depend on
   `nebula-plugin`).

2. **`PluginManifest` moves from `nebula-plugin` to `nebula-metadata`**, alongside
   `Icon` / `MaturityLevel` / `DeprecationNotice` which already lived there.
   `nebula-plugin` keeps `pub use nebula_metadata::PluginManifest;` for source
   compatibility. `nebula-plugin-sdk` gains a single Core-layer dep on
   `nebula-metadata` — the one documented exception to canon §7.1's
   "zero intra-workspace deps" line for the SDK; `nebula-schema` accompanies it
   for wire `ValidSchema`. The SDK README's zero-deps line is updated to say
   "zero engine-side intra-workspace deps."

3. **`nebula-plugin-sdk::PluginMeta` is deleted.** `PluginHandler` exposes
   `manifest() -> &PluginManifest` and `actions() -> &[ActionDescriptor]`
   (wire-level descriptor, not the host-side trait object). Wire
   `PluginToHost::MetadataResponse` carries `manifest: PluginManifest` replacing
   the flat `plugin_key` / `plugin_version` fields, plus per-action
   `schema: ValidSchema`. `DUPLEX_PROTOCOL_VERSION` bumps 2 → 3. Old plugins
   emitting version-2 envelopes are rejected at the handshake; the schema field
   is required (not `#[serde(default)]`) because an action without a schema is
   a canon §11.6 false capability.

4. **New `nebula-plugin::ResolvedPlugin` per-plugin wrapper.** `ResolvedPlugin::
   from<P: Plugin + 'static>(plugin: P)` eagerly calls the three component
   methods once, validates the namespace invariant (every component full key
   must start with `{plugin.key()}.`), catches within-plugin duplicate keys,
   and builds three flat `HashMap<FullKey, Arc<dyn …>>` caches.
   Namespace violations → `PluginError::NamespaceMismatch { plugin, offending_key,
   kind }`; duplicates → `PluginError::DuplicateComponent { plugin, key, kind }`.
   Both errors fire at construction — invariant violations can never reach the
   registry. The `Arc<dyn Plugin>` is kept inside `ResolvedPlugin` to preserve
   the `on_unload()` lifecycle hook.

5. **`enum PluginType { Single, Versions }`, `PluginVersions`, `ArcPlugin`, and
   the version-related `PluginError` variants are deleted.** YAGNI: zero
   production consumers. `::versioned()` / `add_version` appear only in their
   own unit tests. Nebula's deployment model pins versions at Cargo time;
   n8n-style runtime multi-version is a browser-first ergonomic, not applicable
   here. Runtime version pinning, if ever needed, opens a dedicated ADR against
   the now-clean registry surface.

6. **`PluginRegistry` becomes `HashMap<PluginKey, Arc<ResolvedPlugin>>`.**
   Public surface: `register(Arc<ResolvedPlugin>)` + `get` / `contains` /
   `remove` / `clear` / `iter` / `len` / `is_empty` (unchanged shape), plus new
   `all_actions` / `all_credentials` / `all_resources` flat iterators for
   bulk-registration into the engine's flat `ActionRegistry` at startup, and
   `resolve_action` / `resolve_credential` / `resolve_resource` full-key lookups
   that delegate into each `ResolvedPlugin`'s flat cache. No version
   negotiation; one `PluginKey`, one `ResolvedPlugin`.

7. **Sandbox discovery adopts `plugin.toml` parsing (canon §7.1).** A missing or
   invalid `plugin.toml`, missing `[nebula].sdk` field, or unsatisfied SDK
   `VersionReq` causes discovery to skip the binary with `tracing::warn!`.
   When `[plugin].id` is present, discovery uses it as the canonical
   `PluginKey`; if the `plugin.toml` id and the `MetadataResponse` manifest key
   disagree, discovery rejects with an explicit `PluginTomlError::KeyConflict`.
   `RemoteAction` wraps `ProcessSandboxHandler` to satisfy the `Action` trait.
   `DiscoveredPlugin: impl Plugin` is the host-side adapter over the wire
   manifest and `RemoteAction` children; it is registered into `PluginRegistry`
   by the same code path as in-process built-ins. Credentials and resources from
   out-of-process plugins are logged and skipped — gated on ADR-0025 slice 1d
   broker RPC; the wire protocol is not extended for them in this slice (doing
   so without a runtime delivery path would be the §11.6 mistake we are closing
   for actions).

## Consequences

**Positive.**
- Canon §3.5 is now honored in types, not just prose: `Plugin` is a sub-registry
  of runnable objects, each carrying its own `ActionMetadata` + `ValidSchema`.
- L2 §13.1 (plugin load → registry seam) honored end-to-end for in-process
  plugins.
- Namespace violations and within-plugin duplicate keys surface at
  `ResolvedPlugin::from` — before the registry, before dispatch.
- Single `PluginManifest` type shared by host, plugin-author SDK, and wire
  protocol.
- Wire protocol carries full manifest + per-action `ValidSchema`: catalog UI
  can render configuration forms for out-of-process actions (closes the
  §11.6 gap flagged by tech-lead review).
- `PluginRegistry` is simpler — one type, no wrapping enum, no version
  negotiation arithmetic.

**Negative.**
- Breaking public trait change on `nebula-plugin`. Blast radius was near-zero:
  the only production consumer was `crates/api/src/handlers/catalog.rs` (reads
  `manifest()` only; updated in PR 4 of the rollup).
- Drops `PluginType::versioned` which no consumer called; re-adding runtime
  multi-version requires a dedicated ADR.

**Neutral.**
- Out-of-process credentials / resources remain deferred to ADR-0025 slice 1d.
- SDK gains a single Core-layer dep on `nebula-metadata` — documented exception
  to canon §7.1's "zero intra-workspace deps" line.
- Transport protocol version bumps 2 → 3; plugin binaries built against the old
  SDK must be rebuilt. Migration note in `docs/UPGRADE_COMPAT.md`.

## Alternatives considered

- **(a) Keep `descriptor` module + add a metadata adapter layer.** Rejected.
  A type-adapting shim between `ActionDescriptor` and `Arc<dyn Action>` is the
  exact shim-naming trap catalogued in `CLAUDE.md §"Quick Win trap catalog"` and
  `feedback_no_shims`. Fix the source; don't bridge two wrong types.

- **(b) Return `Vec<ActionMetadata>` without the runnable trait object.** Rejected.
  Canon §3.5 calls for a "registry of Actions" — the catalog handler needs to
  look up the live handler separately, which reintroduces the coupling the trait
  change is meant to remove. The schema is half the value.

- **(c) `TypedPlugin` / `PluginHost` split aggregator.** Rejected. A second
  top-level struct with the same purpose as `ResolvedPlugin` but at a different
  granularity adds abstraction without reducing complexity. One `ResolvedPlugin`
  per plugin with eager resolution gives O(1) lookup at no extra type cost.

- **(d) New `PluginV2` trait alongside the existing one.** Rejected. Two traits
  for the same role is the shim shape; `feedback_no_shims` is explicit: replace
  the wrong thing directly. `nebula-plugin` is `frontier`; the breaking change
  cost is low.

- **(e) Keep `PluginType` enum + `PluginVersions`.** Rejected. YAGNI — zero
  production consumers. Nebula's Cargo-deployment model already pins versions;
  n8n-style browser-centric runtime selection is not applicable. If a real
  workflow-level version-pinning requirement appears (canary rollout, node-version
  inlining), a dedicated ADR opens cleanly against the now-simplified surface.

- **(f) Keep `PluginManifest` in `nebula-plugin`; mirror a minimal `PluginMeta`
  in the SDK.** Rejected. Duplication of the same concept in two crates for no
  functional reason; the two types diverge silently with every manifest field
  addition. Moving the type to Core-layer `nebula-metadata` is the root-cause
  fix.

## Follow-ups

- ADR-0025 slice 1d lands broker RPC verbs → out-of-process credentials /
  resources flow over wire and register into the host `PluginRegistry`.
- If a workflow-level version-pinning requirement appears (canary rollout,
  node-version inlining), open a dedicated ADR — the clean `PluginRegistry`
  surface makes a wrapper or `(PluginKey, Version)`-keyed map straightforward.
- `docs/MATURITY.md` row for `nebula-plugin` flips engine-integration column
  `partial → stable` (PR 8 of the rollup).
- `docs/pitfalls.md` has the namespace-mismatch entry as of PR 4 of the rollup.

## Seam / verification

Files that carry the invariants:

- `crates/metadata/src/manifest.rs` — `PluginManifest`, `ManifestError`.
- `crates/plugin/src/plugin.rs` — the canonical `Plugin` trait returning
  `Vec<Arc<dyn Action|Credential|Resource>>`.
- `crates/plugin/src/resolved_plugin.rs` — namespace invariant + duplicate check
  at construction; `ResolvedPlugin::from` is the enforcement point.
- `crates/plugin/src/registry.rs` — `HashMap<PluginKey, Arc<ResolvedPlugin>>`
  with `register`, aggregate `all_*` iterators, and `resolve_*` lookups.
- `crates/plugin/src/error.rs` — pruned error set; `NamespaceMismatch`,
  `DuplicateComponent`, `InvalidManifest` variants added.
- `crates/plugin-sdk/src/lib.rs` + `crates/plugin-sdk/src/protocol.rs` —
  `PluginHandler::manifest()`, `DUPLEX_PROTOCOL_VERSION = 3`,
  `MetadataResponse.manifest`, `ActionDescriptor.schema`, `SDK_VERSION`.
- `crates/sandbox/src/plugin_toml.rs` — `plugin.toml` parser; SDK constraint
  check and optional `[plugin].id` guard.
- `crates/sandbox/src/remote_action.rs` — `impl Action` wrapper around
  `ProcessSandboxHandler`.
- `crates/sandbox/src/discovered_plugin.rs` — `impl Plugin` host-side adapter
  over wire manifest + `RemoteAction` children.
- `crates/sandbox/src/discovery.rs` — orchestration: parse → probe → construct
  → register.
- `crates/sandbox/tests/discovery_schema_roundtrip.rs` — end-to-end schema
  round-trip regression; asserts `ActionMetadata.schema` survives the wire hop
  with field types and validator annotations intact.
- `docs/pitfalls.md` — namespace-mismatch entry: plugin author declares a
  component whose full key does not start with the plugin's own prefix;
  `ResolvedPlugin::from` rejects with `PluginError::NamespaceMismatch`.
