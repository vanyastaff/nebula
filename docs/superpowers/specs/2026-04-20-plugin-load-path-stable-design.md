---
title: Plugin load path stabilization (A + B slices)
date: 2026-04-20
status: draft
related:
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
  - docs/PRODUCT_CANON.md#71-plugin-packaging
  - docs/PRODUCT_CANON.md#131-plugin-load--registry
  - docs/INTEGRATION_MODEL.md
  - docs/MATURITY.md
  - docs/adr/0006-sandbox-phase1-broker.md
  - docs/adr/0018-plugin-metadata-to-manifest.md
  - docs/adr/0025-sandbox-broker-rpc-surface.md
---

# Plugin load path stabilization (A + B slices)

## Executive summary

`nebula-plugin` is blocked from `stable` on its engine-integration column
(`docs/MATURITY.md`) because L2 §13.1 — *a plugin loads; Actions / Resources /
Credentials from `impl Plugin` appear in the catalog without a second manifest*
— is honored only partially today: actions flow through out-of-process
discovery, credentials and resources do not; `nebula-plugin::descriptor::*`
duplicates the canonical `*Metadata` shape (canon §3.5) left over from before
the `nebula-metadata` consolidation (ADR-0018); the `Plugin` trait returns
flat descriptor structs instead of the runnable trait objects canon §3.5
calls for (`Plugin = [ registry: Actions + Resources + Credentials ]`).

This spec covers two tightly-coupled slices that together move
`nebula-plugin` to `stable` and close two §4.5 false-capability notes on
`nebula-sandbox`:

- **Slice A — `plugin.toml` parsing at discovery.** Pre-spawn validation of
  SDK compatibility constraint and optional stable plugin id. Updates the
  sandbox isolation roadmap to reflect ADR-0025 (capabilities come from
  workflow-config, not `plugin.toml`).

- **Slice B — `Plugin` trait returns runnable trait objects + `ResolvedPlugin`
  per-plugin wrapper + `PluginManifest` shared via `nebula-metadata` +
  schema-over-wire for actions + drop unused multi-version registry.**
  - Delete `nebula-plugin::descriptor` module; `Plugin::actions/credentials/
    resources()` returns `Vec<Arc<dyn Action|Credential|Resource>>`.
  - **Move `PluginManifest` from `nebula-plugin` to `nebula-metadata`** so
    both `nebula-plugin` (host) and `nebula-plugin-sdk` (plugin-author)
    import the canonical type. Follow-up note on ADR-0018's frontmatter
    ("moved to nebula-metadata in slice B for cross-side reuse").
  - **Delete `nebula_plugin_sdk::PluginMeta`.** `PluginHandler::metadata`
    returns `&PluginManifest` instead; wire `MetadataResponse` carries
    `manifest: PluginManifest` directly.
  - **New `ResolvedPlugin`** — per-plugin resolved wrapper. Holds
    `Arc<dyn Plugin>` + three `HashMap<FullKey, Arc<dyn …>>` indices
    eagerly built at construction. Validates namespace invariant
    ("every action/credential/resource full key starts with the plugin's
    own prefix") once, at `ResolvedPlugin::from(plugin)`. One plugin → one
    `ResolvedPlugin`.
  - **Drop `enum PluginType`, `PluginVersions`, `ArcPlugin`, and the whole
    runtime multi-version story.** Zero production consumers — YAGNI from
    a months-old design that nothing depends on. `PluginRegistry` becomes
    `HashMap<PluginKey, Arc<ResolvedPlugin>>` directly. Workflow-level
    version pinning, if ever needed, opens its own ADR; today's Cargo-
    deployment pinning covers every real case.
  - `PluginRegistry` surface: `register(Arc<ResolvedPlugin>)` plus the
    existing `get` / `contains` / `remove` / `clear` / `iter` / `len` /
    `is_empty` (unchanged shape), with new `all_*()` flat iterators and
    `resolve_*()` lookups that walk each `ResolvedPlugin`'s flat cache by
    full key.
  - Wire protocol (`nebula-plugin-sdk::protocol::MetadataResponse`) gains
    `manifest: PluginManifest` (replaces flat `plugin_key`/`plugin_version`
    fields) and per-action `schema: ValidSchema` so out-of-process actions
    register with a real schema (eliminates the §11.6 gap flagged by
    tech-lead review).
  - Out-of-process **credentials and resources are explicitly deferred** to
    sandbox broker RPC slice 1d (ADR-0025). Wire protocol for those fields
    is not extended in this slice — adding them without the broker would
    advertise capabilities the code does not deliver.

Everything in this spec is blast-radius-bounded: the only production
consumer of `Plugin` component methods today is
`crates/api/src/handlers/catalog.rs`, and it touches `manifest()` only —
the trait-signature change is effectively costless at the call-site level.

Deliverable accompanying this spec: **ADR-0027 — "Plugin trait returns
runnable traits"** (follow-up to ADR-0018).

## Context

### Canon requirements

- **§3.5 — integration model.**
  `Plugin = [ registry: Actions + Resources + Credentials ]`. Plugin is the
  *container* for leaf concepts; each leaf is `*Metadata + Schema` (a
  schematized object, not a descriptor).
- **§7.1 — plugin packaging.** Three sources of truth:
  1. `Cargo.toml` — Rust package identity + dependency graph.
  2. `plugin.toml` — trust + compatibility boundary (SDK constraint, stable
     id, signing when enabled); parsed without compiling.
  3. `impl Plugin + PluginManifest` — runtime registration source of truth.
- **§13.1 — plugin load → registry seam.** `PluginRegistry::register` is the
  seam; on load, Actions / Resources / Credentials from `impl Plugin` appear
  in the catalog. Test: unit tests in `crates/plugin/`.

### Current state (HEAD, commit `9aa2b62d`)

**`nebula-plugin::Plugin` trait.**

```rust
// crates/plugin/src/plugin.rs
pub trait Plugin: Send + Sync + Debug + 'static {
    fn manifest(&self) -> &PluginManifest;
    fn actions(&self)     -> Vec<ActionDescriptor>     { vec![] }
    fn credentials(&self) -> Vec<CredentialDescriptor> { vec![] }
    fn resources(&self)   -> Vec<ResourceDescriptor>   { vec![] }
    fn on_load(&self)   -> Result<(), PluginError>     { Ok(()) }
    fn on_unload(&self) -> Result<(), PluginError>     { Ok(()) }
}
```

`nebula-plugin::descriptor::{Action,Credential,Resource}Descriptor` are
three flat `{ key, name, description }` structs (ActionDescriptor also has
`version: Version`). Last meaningful commit: `902af033 feat(plugin): v2`,
predating both ADR-0018 (`PluginMetadata` → `PluginManifest`) and the
`nebula-metadata` consolidation.

**Canonical `*Metadata` types** (canon §3.5) already exist and compose
`nebula_metadata::BaseMetadata<K>` — they are what every other crate uses:

- `nebula_action::ActionMetadata` — `base: BaseMetadata<ActionKey>` + ports
  + `ValidSchema` parameters + `IsolationLevel` + `ActionCategory` +
  `CheckpointPolicy` (planned).
- `nebula_credential::CredentialMetadata` — `base: BaseMetadata<CredentialKey>`
  + pattern classifier.
- `nebula_resource::ResourceMetadata` — `base: BaseMetadata<ResourceKey>`.

**Consumers of the `Plugin::components` methods today.**
Only `crates/api/src/handlers/catalog.rs`, and it reads `manifest()` only —
no consumer reads the three component vectors in production code. Doc-tests
and unit tests in `plugin.rs` + unimplemented sketches in
`crates/resource/plans/06-action-integration.md` are the other references.
**Blast radius of the signature change is near-zero.**

**Out-of-process discovery**
(`crates/sandbox/src/discovery.rs`). Scans a directory for executables
matching `nebula-plugin-*`, spawns each with `PluginCapabilities::none()`
for the metadata probe, sends a `MetadataRequest`, and for each action in
the response builds `ActionMetadata::new(key, name, description)
.with_version_full(plugin_version)` — an `ActionMetadata` with an **empty
`ValidSchema`**. The host-side `ActionHandler::Stateless(Arc<
ProcessSandboxHandler>)` is produced per action. Credentials and resources
do not flow over the wire and are never registered.

Consequences of the empty-schema state:
- The catalog UI cannot render a configuration form for an out-of-process
  action — user must hand-write JSON input.
- Flipping `nebula-plugin` to `stable` while advertising "plugin
  registration works" while UI forms are empty violates canon §11.6 for
  out-of-process actions.

**`plugin.toml` status.** Canon §7.1 mandates a minimal `plugin.toml` at
crate root; none of the workspace tree contains a real `plugin.toml`, and
`nebula-sandbox/src/discovery.rs` does not read one. The sandbox README
isolation roadmap step #1 names this as a TODO but references the old
"capabilities from `plugin.toml`" mental model that ADR-0025 (accepted
2026-04-20, one day ago) explicitly supersedes — capabilities are sourced
from workflow-config, not from the plugin author's manifest.

**ADR-0025 interaction.** Broker RPC slice 1d (8 verbs including
`credentials.get`, `network.http_request`) is *not yet landed*. The broker
is the only path by which an out-of-process plugin can offer a credential
or resource at runtime — the secret never materializes in plugin memory
(CredentialRef model). Until slice 1d ships, registering plugin-declared
credentials/resources from out-of-process binaries would produce catalog
entries whose runtime operations are undeliverable.

### Architectural-fit record

Decision-gate check (CLAUDE.md §"Decision gate"):

- Q1 *strengthens golden path* — yes. Aligns `Plugin` trait with canon §3.5
  (Plugin is a sub-registry of runnable objects).
- Q2 *new public surface not honored end-to-end* — no. In-process E2E
  works after refactor; out-of-process credentials/resources are explicitly
  **not exposed** — the opposite of a false capability.
- Q3 *L2 invariant change without an ADR* — **yes, this triggers the gate.**
  The Plugin trait signature is public and §13.1-adjacent. An ADR
  (proposed **ADR-0027**, follow-up to ADR-0018) ships alongside the
  implementation PR series.
- Q4 *upward dep* — no. `nebula-plugin` gains sibling Business-layer deps
  on `nebula-action`/`nebula-credential`/`nebula-resource`. None of those
  crates depend on `nebula-plugin` (verified: no `use nebula_plugin` in
  `crates/{action,credential,resource}/src/`). No cycle.
- Q5 *implicit durable backbone* — no. `PluginRegistry` is explicitly in
  memory; persistence is elsewhere per the plugin crate's own README.
- Q6 *advertises capability code does not deliver* — no. We tighten docs
  (sandbox isolation roadmap updated, MATURITY.md truthful, out-of-process
  credentials/resources not silently stubbed).

Bounded context: **Business** (plugin, with new sibling deps), plus
**Exec** spillover (sandbox/discovery). Concept promotion: **🔴** — public
trait signature change + new cross-crate deps + L2-adjacent. ADR required.

## Scope

### In scope (this spec)

**Slice A — `plugin.toml` parsing at discovery time.**

1. New module `crates/sandbox/src/plugin_toml.rs`. Parses the minimal
   canon §7.1 shape:
   ```toml
   [nebula]
   sdk = "^0.8"       # required; semver constraint on the plugin SDK

   [plugin]
   id = "com.author.slack"   # optional stable plugin id
   ```
2. Discovery reads `plugin.toml` from the plugin's crate root (sibling of
   the binary: same directory, name `plugin.toml`) **before** `ProcessSandbox`
   spawns the binary for the metadata probe. Mismatched SDK constraint
   causes discovery to skip the binary with `tracing::warn!` (consistent
   with current invalid-key handling). A missing `plugin.toml` is a warn +
   skip; missing `[nebula].sdk` is a warn + skip.
3. When `[plugin].id` is present, discovery uses it as the canonical
   `PluginKey` (instead of the one returned by `MetadataResponse`). If
   both are present and disagree, discovery rejects with an explicit error
   — neither source silently wins.
4. Update `crates/sandbox/README.md` isolation roadmap to reflect
   ADR-0025: remove the "capabilities from `plugin.toml`" bullet, replace
   with "capabilities from workflow-config (ADR-0025 D4)". Remove the
   §4.5 false-capability marker on `discovery.rs:117` — after slice A the
   remaining gap is the workflow-config integration, which belongs to
   slice 1d and is documented there, not here.

**Slice B — `Plugin` trait refactor + `ResolvedPlugin` per-plugin wrapper + wire schema.**

5. Delete `crates/plugin/src/descriptor.rs` (entire module). Remove
   `pub use descriptor::*` from `lib.rs`. Remove all local `{Action,
   Credential,Resource}Descriptor` mentions from the plugin crate.

5a. **Move `PluginManifest` from `nebula-plugin` to `nebula-metadata`.**
    Today `PluginManifest` lives in `crates/plugin/src/manifest.rs`.
    Both host (`nebula-plugin`) and plugin-author side
    (`nebula-plugin-sdk`) need to work with bundle metadata; with the
    type parked in `nebula-plugin`, the SDK cannot see it (canon §7.1
    "zero intra-workspace dependencies for the plugin-side crate" blocks
    a direct dep on `nebula-plugin`). Putting `PluginManifest` alongside
    `Icon` / `MaturityLevel` / `DeprecationNotice` in `nebula-metadata`
    solves this without breaking the layering rule — `nebula-metadata`
    is a Core-layer crate, safely depended-on by both host and SDK.
    `nebula-plugin` re-exports `PluginManifest` from `nebula-metadata`
    for source-compatibility of existing call sites. Frontmatter of
    ADR-0018 gains a `superseded_by: []` stays empty plus a short note
    in its `Follow-ups` section pointing to this slice; the body stays
    immutable per ADR convention.

5b. **Delete `nebula-plugin-sdk::PluginMeta`.** Remove the builder
    type at `crates/plugin-sdk/src/lib.rs` (lines 138–174). Change
    `PluginHandler::metadata` to return `&PluginManifest` (imported
    from `nebula-metadata`). `nebula-plugin-sdk` gains a workspace
    dep on `nebula-metadata` — permitted because `nebula-metadata`
    is Core-layer, not engine-side; plugin-sdk README's "zero
    intra-workspace deps" line is updated to say "zero *engine-side*
    intra-workspace deps, with a single Core-layer exception for
    `nebula-metadata` (canon §3.5 / ADR-0018)."

6. Change `Plugin` trait signature — no helpers, just canon-level methods:
   ```rust
   pub trait Plugin: Send + Sync + Debug + 'static {
       fn manifest(&self) -> &PluginManifest;
       fn actions(&self)     -> Vec<Arc<dyn nebula_action::Action>>         { vec![] }
       fn credentials(&self) -> Vec<Arc<dyn nebula_credential::Credential>> { vec![] }
       fn resources(&self)   -> Vec<Arc<dyn nebula_resource::Resource>>     { vec![] }
       fn on_load(&self)   -> Result<(), PluginError> { Ok(()) }
       fn on_unload(&self) -> Result<(), PluginError> { Ok(()) }
   }
   ```
   Convenience lookups (`action(key)`, etc.) do **not** live on the trait —
   forcing every plugin author to deal with defaults for an ergonomic
   helper is API-surface pollution. Lookups live on `ResolvedPlugin` where
   cached indices make them O(1) anyway.

7. Add deps in `crates/plugin/Cargo.toml` on `nebula-action`,
   `nebula-credential`, `nebula-resource`. Required because `Plugin`
   trait signature itself references `Arc<dyn Action>` etc. Verified no
   cycle (section "Architectural-fit record" above).

8. **New `ResolvedPlugin`** in `crates/plugin/src/resolved_plugin.rs` —
   per-plugin resolved wrapper. One plugin → one `ResolvedPlugin`.
   ```rust
   pub struct ResolvedPlugin {
       plugin: Arc<dyn Plugin>,
       // Eagerly resolved and validated at construction. Keyed by
       // full (namespaced) key — no prefix-math at lookup time.
       actions:     HashMap<ActionKey, Arc<dyn Action>>,
       credentials: HashMap<CredentialKey, Arc<dyn Credential>>,
       resources:   HashMap<ResourceKey, Arc<dyn Resource>>,
   }

   impl ResolvedPlugin {
       /// Eagerly calls plugin.actions()/credentials()/resources() once,
       /// asserts every metadata.key starts with `{plugin.key()}.` —
       /// namespace invariant captured at construction. Also catches
       /// within-plugin duplicate keys up front.
       pub fn from<P: Plugin + 'static>(plugin: P) -> Result<Self, PluginError>;

       pub fn plugin(&self)   -> &Arc<dyn Plugin>;
       pub fn manifest(&self) -> &PluginManifest;
       pub fn key(&self)      -> &PluginKey;
       pub fn version(&self)  -> &Version;

       /// O(1) by full key.
       pub fn action(&self, key: &ActionKey)         -> Option<&Arc<dyn Action>>;
       pub fn credential(&self, key: &CredentialKey) -> Option<&Arc<dyn Credential>>;
       pub fn resource(&self, key: &ResourceKey)     -> Option<&Arc<dyn Resource>>;

       pub fn actions(&self)     -> impl Iterator<Item = (&ActionKey, &Arc<dyn Action>)>;
       pub fn credentials(&self) -> impl Iterator<Item = (&CredentialKey, &Arc<dyn Credential>)>;
       pub fn resources(&self)   -> impl Iterator<Item = (&ResourceKey, &Arc<dyn Resource>)>;
   }
   ```
   Why `Arc<dyn Plugin>` is kept inside: gives `ResolvedPlugin::manifest()`
   /`key()` a single source of truth (the plugin's own `manifest()`) and
   preserves the `on_unload()` lifecycle hook for when plugins are
   removed from the registry.

9. **Delete multi-version registry machinery.** Remove
   `crates/plugin/src/plugin_type.rs` (the `enum PluginType` with its
   `Single` / `Versions` variants plus `ArcPlugin` helper) and
   `crates/plugin/src/versions.rs` (the `PluginVersions` struct).
   Remove `PluginError::VersionAlreadyExists`, `VersionNotFound`,
   `NoVersionsAvailable`, `KeyMismatch` variants — they only existed
   for `PluginVersions` and have no other callers.
   Rationale: zero production consumers today (`::versioned()` /
   `add_version` only appear in their own unit tests). Multi-version
   runtime registries model n8n's browser-centric story; Nebula's
   Rust-native story pins versions at Cargo-deployment time. If
   workflow-level version pinning arrives later, it opens its own ADR
   — likely a wrapper type rather than this enum shape.

10. `crates/plugin/src/registry.rs` — simplify `PluginRegistry`
    dramatically. Inner map becomes `HashMap<PluginKey, Arc<ResolvedPlugin>>`
    (no wrapping enum).
    ```rust
    pub struct PluginRegistry {
        plugins: HashMap<PluginKey, Arc<ResolvedPlugin>>,
    }

    impl PluginRegistry {
        pub fn new() -> Self;

        /// Register a resolved plugin. Errors if `world.key()` is
        /// already taken (`PluginError::AlreadyExists`). No version
        /// negotiation — one `PluginKey`, one world.
        pub fn register(&mut self, world: Arc<ResolvedPlugin>) -> Result<(), PluginError>;

        pub fn get(&self, key: &PluginKey)      -> Option<Arc<ResolvedPlugin>>;
        pub fn contains(&self, key: &PluginKey) -> bool;
        pub fn remove(&mut self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>>;
        pub fn clear(&mut self);
        pub fn iter(&self) -> impl Iterator<Item = (&PluginKey, &Arc<ResolvedPlugin>)>;
        pub fn len(&self) -> usize;
        pub fn is_empty(&self) -> bool;

        /// Flat iterators used by `nebula-runtime::ActionRegistry` at
        /// startup to bulk-register handlers into the engine's flat
        /// dispatch map. Walk every plugin's `ResolvedPlugin` cache.
        pub fn all_actions(&self)     -> impl Iterator<Item = (&PluginKey, &Arc<dyn Action>)>;
        pub fn all_credentials(&self) -> impl Iterator<Item = (&PluginKey, &Arc<dyn Credential>)>;
        pub fn all_resources(&self)   -> impl Iterator<Item = (&PluginKey, &Arc<dyn Resource>)>;

        /// Lookup by full key — O(plugins) to find the owning
        /// `ResolvedPlugin`, then O(1) in its inner map. Used for
        /// introspection; not on engine dispatch hot path.
        pub fn resolve_action(&self, full: &ActionKey)         -> Option<Arc<dyn Action>>;
        pub fn resolve_credential(&self, full: &CredentialKey) -> Option<Arc<dyn Credential>>;
        pub fn resolve_resource(&self, full: &ResourceKey)     -> Option<Arc<dyn Resource>>;
    }
    ```
    Existing `PluginRegistry::register(PluginType)` surface goes away —
    callers change to `register(Arc<ResolvedPlugin>)`. Existing unit tests
    get ported (most still make sense at the Registry layer; the
    version-specific ones delete with `PluginVersions`).

11. `crates/plugin/macros/src/plugin.rs` — `#[derive(Plugin)]`. Per
    tech-lead review, the macro today does not emit `actions()` /
    `credentials()` / `resources()` method bodies (authors hand-write
    them); only the manifest plumbing emits. The macro change is
    therefore: ensure the emitted code still compiles under the new
    trait signature (hand-written `fn actions() -> Vec<Arc<dyn Action>>`
    already matches). No macro surface change.

12. **Wire protocol — schema for actions.**
    `crates/plugin-sdk/src/protocol.rs`:
    ```rust
    pub struct ActionDescriptor {
        pub key: String,
        pub name: String,
        #[serde(default)]
        pub description: String,
        pub schema: ValidSchema,   // NEW — tech-lead-mandated (§11.6)
    }
    ```
    Requires `nebula_schema::ValidSchema: Serialize + Deserialize`. If
    missing, this spec's implementation plan allocates a preparatory PR
    to nebula-schema that derives the impls (thin — `ValidSchema`
    composes serde-friendly primitives per canon §3.5's "typed,
    validated configuration").

    Protocol version bump: `DUPLEX_PROTOCOL_VERSION: u32 = 2` → `3`. Old
    plugins emitting version-2 envelopes are rejected at the handshake
    with a clear error. Rationale: the schema field is required, not
    `#[serde(default)]`, because an action without a schema is a §11.6
    false capability — tolerating it via `default` would re-open the
    gap the bump is closing. Short migration note lands in
    `docs/UPGRADE_COMPAT.md`.

13. **`RemoteAction` wrapper** in `crates/sandbox/src/remote_action.rs`
    (new):
    ```rust
    pub struct RemoteAction {
        metadata: ActionMetadata,
        handler:  Arc<ProcessSandboxHandler>,
    }
    impl nebula_action::Action for RemoteAction {
        fn metadata(&self) -> &ActionMetadata { &self.metadata }
        /* Self: Sized guards exclude runtime dependency methods from the
         * vtable — consistent with ADR-0025 RPC path, nothing more to
         * impl. Dispatch goes through ProcessSandboxHandler elsewhere. */
    }
    ```
    `discovery.rs::create_handlers` constructs `RemoteAction` instances
    with `metadata` assembled from wire
    `{key, name, description, schema}` + host-synthesized
    `IsolationLevel::Isolated`, `ActionCategory::Data`, default ports.
    Synthesized defaults are documented in ADR-0027 as deliberately
    conservative (§11.6 permits conservative defaults; it forbids
    advertising capabilities not delivered).

14. **Credentials / Resources from out-of-process plugins — not
    registered in this slice.** Discovery logs each via
    `tracing::info!(binary = %path, "out-of-process plugin declared \
     <n> credentials / <m> resources; skipped — gated on ADR-0025 slice \
     1d broker RPC")` but does not attempt to construct or register
    proxy objects. Wire protocol's `MetadataResponse` is **not**
    extended with credential / resource descriptor lists in this slice —
    extending the protocol without a way to deliver the runtime is the
    exact §11.6 mistake we are closing for actions. The protocol
    extension ships with slice 1d.

15. **`DiscoveredPlugin: impl Plugin`** in `crates/sandbox/src/
    discovered_plugin.rs` (new). Wraps a `PluginManifest` +
    `RemoteAction` children. `actions()` returns the `Arc<dyn Action>`
    vector; `credentials()` / `resources()` return empty vectors.
    `manifest()` returns the `PluginManifest` received over wire (with
    `plugin.toml`'s `[plugin].id` applied if present). This is the
    host-side impl that gets registered into the `PluginRegistry` from
    out-of-process discovery — same registration code path as in-process
    built-ins.

    The current legacy `DiscoveredPlugin` struct in
    `crates/sandbox/src/discovery.rs:20` (fields `key: String, version:
    String, actions: Vec<ActionDescriptor>`) is **inlined** — after the
    wire protocol bump carries the full `PluginManifest`, the
    intermediate DTO carries no information that isn't already in the
    wire envelope. Discovery constructs `DiscoveredPlugin: impl Plugin`
    directly from the `MetadataResponse`.

16. **ADR-0027 draft** (deliverable; see "Open artifacts" below).

17. **Docs sync (per canon §17 Definition of Done):**
    - `docs/MATURITY.md` — `nebula-plugin` engine-integration column
      `partial → stable`.
    - `crates/plugin/README.md` — update trait signature snippet,
      descriptor language removed, `ResolvedPlugin` documented, registry
      lookup/iterator methods added.
    - `crates/sandbox/README.md` — isolation roadmap per ADR-0025 (slice
      A item 4 above); appendix `Discovery TODO` reframed to reflect
      closed SDK-constraint gap + still-open workflow-config integration.
    - `crates/plugin-sdk/README.md` — protocol version bump note,
      `MetadataResponse` now carries `manifest: PluginManifest` and
      per-action `schema: ValidSchema`; zero-deps line updated with
      the `nebula-metadata` Core-layer exception.
    - `docs/UPGRADE_COMPAT.md` — protocol-version bump entry.
    - `docs/pitfalls.md` — namespace-mismatch entry: a plugin author
      declares an action / credential / resource whose full key does
      not start with the plugin's own prefix (e.g. plugin keyed
      `slack` returning an action keyed `api.foo`);
      `ResolvedPlugin::from` rejects with
      `PluginError::NamespaceMismatch`. Error message names the
      offending key and the expected prefix.

### Out of scope (explicit non-goals)

- **Out-of-process credential / resource runtime.** Blocked by ADR-0025
  slice 1d (broker RPC verbs). Spec is aware; wire protocol extension for
  these fields ships with slice 1d.
- **`plugin.toml` signing verification.** Canon §7.1 marks it `planned`;
  lives on the sandbox isolation roadmap per canon §12.6. Separate ADR
  when tooling (`cargo-nebula`) is ready.
- **Cross-plugin dependency activation-time check.** Separate concern
  (plugin README calls it out as sandbox-owned); different scope.
- **`cargo-nebula` tooling.** Pre-compile discovery helper referenced in
  canon §7.1 is not on this slice — tooling follow-up.
- **In-process built-in plugin bootstrap wiring.** The refactor makes
  the trait shape correct; actually wiring built-ins (e.g. a `core`
  plugin bundling `ControlAction` nodes) is a separate PR series on
  the `nebula-action` + `nebula-engine` side. This spec leaves the
  `PluginRegistry` with room for built-ins; it does not add them.
- **Runtime `ActionRegistry` redesign.** Flat dispatch stays (canon
  §10). `nebula-runtime::ActionRegistry` receives bulk-registration
  from `PluginRegistry::all_actions()` at startup; that's the only new
  connective tissue.
- **`ActionKey::split_plugin_prefix` (and equivalents).** A previous
  design revision proposed adding a typed split helper on `nebula-core`.
  With `ResolvedPlugin`'s flat-by-full-key cache, `resolve_*` no longer
  needs to split — it just probes every `ResolvedPlugin` with the full key.
  Dropped.

## Architecture

### Three types, two lookup paths

**Types:**

- **`Plugin`** (trait, `nebula-plugin`) — what the plugin author
  impls. Returns `Vec<Arc<dyn Action|Credential|Resource>>` +
  `manifest()` + lifecycle hooks. No helpers — the trait stays
  minimal.
- **`ResolvedPlugin`** (struct, `nebula-plugin`, new) — per-plugin
  resolved wrapper. Holds `Arc<dyn Plugin>` + three
  `HashMap<FullKey, Arc<dyn …>>` caches eagerly built at
  construction. Validates namespace invariant once. One plugin ↔
  one `ResolvedPlugin`.
- **`PluginRegistry`** (struct, `nebula-plugin`) — top-level
  registration + lookup entry point:
  `HashMap<PluginKey, Arc<ResolvedPlugin>>`. No enum wrapping, no
  version negotiation. One `PluginKey`, one `ResolvedPlugin`.
- **`PluginManifest`** (struct, moved in this slice from
  `nebula-plugin` to `nebula-metadata`) — canonical bundle
  descriptor. Used by host, by plugin-author SDK, and on-wire in
  `MetadataResponse`.

**Paths:**

1. **Flat dispatch (unchanged).**
   `nebula-runtime::ActionRegistry` is a flat `DashMap<ActionKey,
   Vec<ActionEntry>>`. Engine `dispatch(action_key) → handler` is O(1).
   14+ call sites. This is canon §10 golden path and we do not divert it.

2. **Registry lookup by full key (new, this slice).**
   `PluginRegistry::resolve_action(full) → Option<Arc<dyn Action>>`
   walks registered plugins, probing each `ResolvedPlugin`'s flat
   `HashMap<ActionKey, Arc<dyn Action>>` by the full key. O(plugins)
   + O(1). Used for authoring, introspection, catalog UI — **not**
   on engine dispatch hot path. Credentials / resources identical.

Both paths are populated from the same source: at plugin-load time the
engine walks `PluginRegistry::all_actions()` and bulk-registers into
`ActionRegistry`. Source of truth is each `ResolvedPlugin`'s own flat
map (built from `Plugin::actions()` / `credentials()` / `resources()`
exactly once at `ResolvedPlugin::from`); the `ActionRegistry` flat cache
is pure dispatch. No divergence risk — the handle stored in both is
the same `Arc<dyn Action>`.

### Composition rule and collision handling

A plugin with `PluginKey("http")` provides an action whose
`ActionMetadata.base.key` is `ActionKey("http.send_request")`. The
rule from `discovery.rs:158` becomes explicit and enforced at
`ResolvedPlugin::from`:

> **Every action / credential / resource key a plugin provides must
> live inside the plugin's own namespace prefix** — i.e. start with
> `{plugin_key}.`. Wire-level `ActionDescriptor.key` is accepted
> either as a short local name (host prepends the namespace) or as a
> fully-qualified key already starting with `{plugin_key}.` (host
> validates). Cross-namespace wire keys are rejected at discovery.

Collision handling has two layers:

1. **Within one `ResolvedPlugin`** — `ResolvedPlugin::from` rejects a
   plugin that declares two actions with the same full key (or two
   credentials / resources with the same key). The plugin never gets
   registered.
2. **Across plugins, at flat `ActionRegistry`** — if plugin `a`
   declares `a.b.c` and plugin `a.b` declares `a.b.c`, the engine's
   flat `ActionRegistry::register` rejects the second one at bulk-
   register time. Consistent with today's behaviour.

`PluginRegistry::resolve_action` is free of prefix-matching: it
probes each registered `ResolvedPlugin`'s flat cache directly by the
full key. No `ActionKey::split_plugin_prefix` helper is needed on
`nebula-core` (previous design artefact, now dropped — kept here as
explicit non-goal because an earlier revision of this spec proposed
adding it).

### Wire protocol shape after the bump

`DUPLEX_PROTOCOL_VERSION = 3`:

```rust
pub enum PluginToHost {
    // ... existing variants unchanged ...
    MetadataResponse {
        id: u64,
        protocol_version: u32,
        manifest: nebula_metadata::PluginManifest,  // NEW — replaces flat
                                                     // plugin_key/plugin_version
                                                     // fields; single source of
                                                     // truth for bundle info
        actions: Vec<ActionDescriptor>,             // descriptor now carries schema
        // credentials/resources fields intentionally omitted until slice 1d
    },
}

pub struct ActionDescriptor {
    pub key:    String,
    pub name:   String,
    #[serde(default)]
    pub description: String,
    pub schema: nebula_schema::ValidSchema,  // NEW, required
}
```

Host handshake: on `MetadataResponse`, verify `protocol_version == 3`;
reject otherwise with a clear error. Version-2 plugins re-built against
the new SDK pick up the new fields automatically (SDK version bumps
alongside).

### Data flow — in-process built-in plugin

```
host binary start
  └─ BuiltinCorePlugin::new()         // hand-written impl Plugin
     ├─ manifest(): PluginManifest{ key: "core", ... }
     ├─ actions():  vec![Arc::new(IfAction), Arc::new(SwitchAction), ...]
     │   // each IfAction: impl Action, metadata().base.key == "core.if"
     ├─ credentials(): vec![]
     └─ resources():   vec![]

  let world = ResolvedPlugin::from(BuiltinCorePlugin::new())?;
    // eager resolution:
    //   ├─ validates every Action/Credential/Resource key starts with "core."
    //   ├─ builds flat caches:
    //   │    actions:     { ActionKey("core.if") -> Arc<IfAction>, ... }
    //   │    credentials: { }
    //   │    resources:   { }
    //   └─ returns ResolvedPlugin

  registry.register(Arc::new(world))?;
    // PluginKey("core") -> Arc<ResolvedPlugin> lands in the registry HashMap;
    // PluginError::AlreadyExists if "core" was already registered.

  engine startup
    └─ for (pk, action) in registry.all_actions():
         ActionRegistry::register(action.metadata().base.key.clone(), action.handler())
       // flat DashMap populated for dispatch

  catalog UI
    └─ for (pk, world) in registry.iter():
         for (full_key, action) in world.actions():
             render_catalog_entry(world.manifest(), action.metadata())
             // full manifest + full schema available

  runtime dispatch of "core.if"
    └─ ActionRegistry::get("core.if") -> handler (O(1) flat)

  introspection lookup of "core.if"
    └─ registry.resolve_action(&ActionKey::new("core.if")?) -> Arc<dyn Action>
       // O(plugins) walk over ResolvedPlugins; each probes its cache by full key
```

### Data flow — out-of-process discovered plugin

```
discover_directory(dir, timeout, default_caps)
  └─ for entry in dir:
       if !is_executable(entry): skip
       else:
         read plugin.toml (slice A):
           if missing or [nebula].sdk missing: warn, skip
           if SDK constraint unsatisfied: warn, skip
           plugin_id_override = [plugin].id (optional)

         ProcessSandbox::new(binary, timeout, PluginCapabilities::none())
         send MetadataRequest -> MetadataResponse @ protocol v3
           {
             manifest: PluginManifest { key, version, maturity, ... },  // full bundle descriptor
             actions: [ActionDescriptor { key, name, description, schema }, ...]
           }

         canonical plugin key = plugin_id_override.or(manifest.key())
         // conflict: plugin_id_override != manifest.key() => reject

         for each action in response:
           build ActionMetadata {
             base: BaseMetadata { key: "{plugin_key}.{action.key_local}", ... },
             schema: action.schema,          // real schema now!
             isolation: IsolationLevel::Isolated,
             category:  ActionCategory::Data,
             ports:     default_ports(),
           }
           wrap as RemoteAction { metadata, handler: ProcessSandboxHandler::new(...) }
           collect into actions vector

         construct DiscoveredPlugin {  // new impl Plugin wrapper
           manifest: manifest (from wire, with plugin_id_override applied if set),
           actions:  Vec<Arc<dyn Action>>,
           credentials: vec![],   // slice 1d
           resources:   vec![],   // slice 1d
         }

       let world = ResolvedPlugin::from(handle)?;
         // validates + caches actions by full key
       registry.register(Arc::new(world))?;
         // plugin entry lands in the registry HashMap
```

### Error model

New typed errors:

- `nebula_sandbox::PluginTomlError`:
  - `Missing { path: PathBuf }`
  - `InvalidToml { path: PathBuf, source: toml::de::Error }`
  - `MissingSdkConstraint { path: PathBuf }`
  - `IncompatibleSdk { required: VersionReq, actual: Version, plugin: String }`
  - `InvalidPluginId { raw: String, source: KeyError }`
  - `KeyConflict { toml_id: String, metadata_key: String }`
- `SandboxError::Discovery(PluginTomlError)` wraps the above for the
  existing discovery surface.
- `nebula_plugin::PluginError::ProtocolVersionMismatch { required: u32,
  actual: u32 }` for the wire-version-bump case (graduated from
  today's ad-hoc string error).
- `nebula_plugin::PluginError::NamespaceMismatch { plugin: PluginKey,
  offending_key: String, kind: ComponentKind }` surfaced by
  `ResolvedPlugin::from` when a plugin declares an action/credential/
  resource whose full key does not start with `{plugin.key()}.`.
- `nebula_plugin::PluginError::DuplicateComponent { plugin: PluginKey,
  key: String, kind: ComponentKind }` surfaced by `ResolvedPlugin::from`
  when a plugin declares two components of the same kind with the
  same full key — caught inside the single plugin before it even
  reaches the registry.

All are `warn + skip` at the discovery-directory boundary — a bad plugin
must not poison the scan — but surface as typed `Result::Err` at the
per-plugin function level so tests can assert exact shape.

## Acceptance criteria

After this slice is merged:

1. `cargo nextest run -p nebula-plugin` green; doc-tests for `Plugin`
   trait methods use `Arc<dyn Action>` etc. and compile.
2. `cargo nextest run -p nebula-sandbox` green; new tests cover
   `plugin.toml` parsing (happy path + each error variant) and
   `RemoteAction` metadata round-trip.
3. `rg 'nebula_plugin::descriptor' crates/` → no matches. `rg
   '{Action,Credential,Resource}Descriptor' crates/plugin/` → no
   matches. The module is gone.
4. `rg 'use nebula_plugin' crates/{action,credential,resource}/src/`
   → no matches (cycle guard).
5. `docs/MATURITY.md` — `nebula-plugin` row engine-integration column
   reads `stable`. Last-reviewed date bumped.
6. ADR-0027 merged alongside the implementation PR series, status
   `accepted`, referenced from ADR-0018 frontmatter's `related:`.
7. Knife scenario (`crates/api/tests/knife.rs`) passes unchanged — flat
   `ActionRegistry` dispatch path unaffected.
8. `crates/api/src/handlers/catalog.rs` compiles without changes
   (manifest-only consumer, signature change is at orthogonal methods).
9. `crates/sandbox/README.md` isolation roadmap lists ADR-0025 as the
   source of the capability model; the §4.5 note on `discovery.rs:117`
   is removed.
10. Out-of-process round-trip — **unconditional**: a new fixture
    plugin binary (`crates/plugin-sdk/src/bin/schema_fixture.rs` or
    similar) declares one action with a two-field schema; an
    integration test under `crates/sandbox/tests/` spawns it through
    `discover_directory`, asserts the host-side `ActionMetadata.schema`
    round-trips with both fields and their validator annotations intact,
    and asserts the registered `RemoteAction` dispatches through the
    sandbox. Fails the acceptance gate if the fixture is absent or the
    test is `#[ignore]`d.
11. `ResolvedPlugin::from` rejects a plugin declaring an action with a
    key outside its namespace (`PluginError::NamespaceMismatch`) and a
    plugin declaring two actions with identical full keys
    (`PluginError::DuplicateComponent`). Same for credentials and
    resources. Namespace invariants never reach `PluginRegistry`.

## Migration and sequencing (implementation plan preview)

Detailed plan is produced by `writing-plans` skill as the next step.
High-level PR shape this spec anticipates (each PR reviewable in
isolation; strictly sequential):

1. **Prep PR — schema serde.** `nebula-schema::ValidSchema` derives
   `Serialize + Deserialize` if not already present. Round-trip unit
   test. Needed for the wire-schema change.
2. **Manifest move PR.** Move `PluginManifest` from
   `crates/plugin/src/manifest.rs` to `crates/metadata/src/` (new
   `manifest.rs`). `nebula-plugin` re-exports for source compatibility.
   Update ADR-0018 frontmatter `Follow-ups` with a one-line note. Add
   `nebula-plugin-sdk → nebula-metadata` workspace dep; update
   plugin-sdk README's "zero intra-workspace deps" line to mention the
   Core-layer exception.
3. **Delete PluginMeta + wire v3 bump PR.** Remove `PluginMeta` from
   `crates/plugin-sdk/src/lib.rs`. `PluginHandler::metadata` returns
   `&PluginManifest`. Wire `MetadataResponse` carries `manifest:
   PluginManifest` (replaces flat `plugin_key` / `plugin_version`
   fields). `DUPLEX_PROTOCOL_VERSION = 3` bump. **Downstream file
   updates in the same PR** (otherwise CI / newly-generated plugins
   break):
   - `crates/plugin-sdk/src/bin/echo_fixture.rs` — flip
     `PluginMeta::new(...)` to `PluginManifest::builder(...)`
   - `crates/plugin-sdk/src/bin/counter_fixture.rs` — same
   - `crates/plugin-sdk/src/lib.rs` doc comments + `README.md`
   - `crates/plugin-sdk/tests/broker_smoke.rs` — re-verify fixture
     build still smoke-tests green
   - `apps/cli/src/commands/plugin_new.rs:155-163` — scaffolding
     template emits `PluginManifest::builder(...)`, not `PluginMeta`
4. **Plugin refactor PR (atomic, was "PR 4 + PR 5").** Tech-lead
   review flagged a 2-PR split here as a CI-red mainline window that
   blocks the whole workspace (the `PluginRegistry` holds
   `PluginType::Single(Arc<dyn Plugin>)` today; flipping the trait
   without replacing `PluginType` leaves the registry storing
   old-trait objects). Merged as one cohesive story — *"make the
   `Plugin` trait canonical":*
   - Delete `crates/plugin/src/descriptor.rs` module.
   - Flip `Plugin` trait signature to return
     `Vec<Arc<dyn Action|Credential|Resource>>`.
   - Update `crates/plugin/Cargo.toml` with new sibling deps on
     `nebula-action` / `nebula-credential` / `nebula-resource`.
   - Delete `crates/plugin/src/plugin_type.rs` (enum) and
     `crates/plugin/src/versions.rs` (struct) entirely + drop
     `VersionAlreadyExists` / `VersionNotFound` / `NoVersionsAvailable`
     / `KeyMismatch` error variants.
   - Add `crates/plugin/src/resolved_plugin.rs` — `ResolvedPlugin`
     with namespace-invariant check at `::from`; add
     `PluginError::NamespaceMismatch` and `DuplicateComponent`.
   - Simplify `PluginRegistry` to
     `HashMap<PluginKey, Arc<ResolvedPlugin>>` with the slimmed
     surface.
   - `crates/engine/src/lib.rs:70` — remove the `PluginType`
     re-export.
   - `crates/engine/README.md:64` — update the lingering reference.
   - Fix the four doc-tests in `plugin.rs`; update
     `crates/plugin/README.md` for new trait signature, descriptor
     deletion, `ResolvedPlugin` glossary entry (tech-lead note).
   - `docs/pitfalls.md` — add entry: *"Plugin author declares an
     action key outside the plugin's own namespace (e.g. plugin
     keyed `slack` returns action `api.foo`); `ResolvedPlugin::from`
     rejects with `PluginError::NamespaceMismatch` — confusing to
     read if the author didn't know the rule. The rule lives in
     canon §7.1 / ADR-0027 and on `ResolvedPlugin`'s docs."*
5. **Registry aggregate PR.** Add `PluginRegistry::all_*()` iterators
   and `resolve_*()` lookups. Thin delegations into each registered
   `ResolvedPlugin`'s flat cache. Tests: cross-plugin resolve,
   namespace-violation register-path, `all_actions` bulk export shape.
6. **Wire schema + discovery PR.** Extend wire `ActionDescriptor`
   with `schema: ValidSchema`; `plugin.toml` parser; SDK constraint
   check in discovery; `RemoteAction` wrapper; `DiscoveredPlugin`
   impl Plugin (legacy `DiscoveredPlugin` DTO inlined); discovery
   skips credentials/resources with the info-log. Updates
   `crates/sandbox/README.md` and `docs/UPGRADE_COMPAT.md`.
7. **ADR-0027 PR.** Merged before or with PR 4 (so the public trait
   change has a written rationale citable from `CHANGELOG`).
   Frontmatter `related: [0018]` plus a symmetric
   `related: [0027]` addition to ADR-0018's frontmatter
   `related:` (ADR-0018 body stays immutable per convention; only
   frontmatter `related:` is maintained).
8. **MATURITY.md PR.** `nebula-plugin` row to `stable`, last-reviewed
   date bumped. Merged last, after all acceptance-criteria checks pass.

Engine bulk-register (`ActionRegistry::bulk_register` from
`PluginRegistry::all_actions()`) is touched lightly in PR 5 if a
wiring point is needed; otherwise deferred.

## Open artifacts accompanying this spec

- **ADR-0027 — "Plugin trait returns runnable traits; `ResolvedPlugin`
  resolves per plugin; `PluginManifest` moves to `nebula-metadata`;
  multi-version registry dropped" (proposed).**
  Outline:
  - Context: ADR-0018 context + canon §3.5 requirement + descriptor
    duplication + `PluginMeta`/`PluginManifest` duplication across
    host/plugin-author sides + L2 §13.1 partial honor today + a
    multi-version runtime story (`PluginType` enum) with zero
    production consumers.
  - Decision:
    (1) descriptor module deleted; `Plugin` trait flipped to
    `Vec<Arc<dyn …>>`;
    (2) `PluginManifest` moved from `nebula-plugin` to
    `nebula-metadata` so both host and plugin-author SDK import the
    canonical type (plugin-SDK gains a single Core-layer exception
    to its zero-dep rule);
    (3) `PluginMeta` deleted from SDK; `PluginHandler::metadata`
    returns `&PluginManifest`; wire `MetadataResponse` carries
    `manifest: PluginManifest`;
    (4) new `ResolvedPlugin` per-plugin wrapper enforces namespace
    invariant at `ResolvedPlugin::from`;
    (5) `enum PluginType` + `PluginVersions` + `ArcPlugin` deleted;
    `PluginRegistry` becomes
    `HashMap<PluginKey, Arc<ResolvedPlugin>>` directly (no wrapping
    enum; one `PluginKey`, one plugin — runtime multi-version was
    unused YAGNI);
    (6) `PluginRegistry` gains `all_*` iterators + `resolve_*`
    lookups delegating into `ResolvedPlugin` caches.
  - Consequences: positive (canon alignment, L2 §13.1 honored
    end-to-end in-process, namespace violations caught at register
    rather than at dispatch, single `PluginManifest` type, simpler
    registry surface); negative (breaking public trait — near-zero
    blast radius per tech-lead review, only `api/handlers/catalog.rs`
    touches manifest; drops `PluginType::versioned` which no one
    calls); neutral (out-of-process credentials/resources remain
    deferred to slice 1d per ADR-0025).
  - Alternatives considered:
    (a) keep descriptors + metadata adapter (rejected: shim-naming
    trap per `CLAUDE.md §"Quick Win trap catalog"`);
    (b) return `Vec<ActionMetadata>` without the runnable trait
    object (rejected: doesn't honor §3.5 "registry of Actions",
    catalog still needs to look up handlers separately);
    (c) `TypedPlugin` / `PluginHost` split aggregator layer
    (rejected in favour of single `ResolvedPlugin` per plugin —
    simpler, no new top-level type, eager resolution still gives
    O(1) lookup);
    (d) new `PluginV2` trait alongside (rejected: two traits for
    the same role, violates no-shims feedback);
    (e) keep `PluginType` enum + `PluginVersions` (rejected: YAGNI
    — zero production consumers; n8n-style runtime multi-version is
    browser-first ergonomics, Rust-native deployment pins versions at
    Cargo level; re-opening it later is a clean new ADR, not this
    one's problem);
    (f) keep `PluginManifest` in `nebula-plugin` and mirror a
    minimal `PluginMeta` in SDK (rejected: duplication of the same
    concept in two crates for no functional reason; moving the type
    to Core-layer `nebula-metadata` is a straightforward fix).
  - Follow-ups: slice 1d broker RPC lands credentials/resources
    counterpart over wire; MATURITY.md row stable; a future ADR
    revisits runtime multi-version if a real consumer appears
    (n8n-style workflow pinning, canary rollout, etc.).
- **`docs/UPGRADE_COMPAT.md`** — entry for `DUPLEX_PROTOCOL_VERSION`
  2 → 3 and plugin-SDK version bump required.

## Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| `ValidSchema` serde derive surfaces non-serializable internals | Low | Prep-PR 1 is dedicated to this; if a non-trivial issue, spec is blocked and we triage before continuing. |
| Protocol version bump breaks slices 1a/1b/1c fixtures or snapshots | Low | Fixtures already import `DUPLEX_PROTOCOL_VERSION`; version 3 propagates via `cargo update`. Tests that hard-code `2` get flipped. |
| Cycle via a not-yet-grepped use of `nebula_plugin` in one of the three sibling crates | Very low | Tech-lead grep came back empty; CI catches any new introduction via `deny.toml` + review. |
| `ResolvedPlugin::from` rejects a built-in plugin for a namespace violation at engine startup, breaking boot | Very low | Namespace invariant is a compile-aligned property of the plugin author's own key scheme; violations should only come from fresh hand-written plugins, caught in unit tests before wiring. Error message names the offending action/credential/resource key. |
| Out-of-process plugin authors complain that credentials/resources don't work | Expected | Deliberate per ADR-0025. `tracing::info!` log is addressed to the author; release notes mention slice 1d as the delivery vehicle. |
| `#[derive(Plugin)]` surfaces a compilation issue with the new trait bound | Very low | Per tech-lead, the macro does not emit component methods; author writes them manually. The macro's hand-over-the-manifest part is orthogonal. |
| Moving `PluginManifest` to `nebula-metadata` breaks downstream import paths | Low | `nebula-plugin` re-exports `PluginManifest` from its new home; existing `use nebula_plugin::PluginManifest` keeps working. Direct imports from `nebula_plugin::manifest::*` (less common) flip to `nebula_metadata::*`. ADR-0018 frontmatter `Follow-ups` records the move. |
| Dropping `PluginType` / `PluginVersions` breaks a downstream we haven't grepped | Very low | Confirmed only call sites are in `versions.rs` / `plugin_type.rs` tests themselves and re-exports in `lib.rs`. If CI surfaces an unexpected consumer, the spec is blocked and we triage before proceeding. |

## References

- Canon §3.5, §4.5, §7.1, §11.6, §12.6, §13.1, §17 — `docs/PRODUCT_CANON.md`
- `docs/INTEGRATION_MODEL.md` §7 — plugin packaging mechanics
- `docs/MATURITY.md` — `nebula-plugin` row
- `docs/STYLE.md` §6 — secret handling (relevant to the `RemoteAction`
  wrapper not carrying credential material)
- ADR-0006 — duplex broker transport (slices 1a–1c landed)
- ADR-0018 — `PluginMetadata` → `PluginManifest` (parent decision)
- ADR-0025 — broker RPC surface (blocks out-of-process credentials/resources)
- ADR-0027 — **to be drafted alongside implementation** (this spec's own
  deliverable)
- Feedback memory `feedback_no_shims` — direct replacement, no adapters
- Feedback memory `feedback_direct_state_mutation` — typed errors over
  silent swallow
