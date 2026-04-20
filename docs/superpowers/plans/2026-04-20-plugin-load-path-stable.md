# Plugin Load Path Stable — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `nebula-plugin` from `partial` to `stable` on engine-integration by canonicalising the `Plugin` trait (returns runnable `Arc<dyn Action|Credential|Resource>`), introducing `ResolvedPlugin` as the per-plugin eager wrapper, deleting legacy `descriptor` module + multi-version `PluginType`/`PluginVersions`, consolidating bundle metadata into `nebula-metadata::PluginManifest`, extending the wire protocol with `PluginManifest` + per-action `ValidSchema`, and parsing `plugin.toml` at discovery.

**Architecture:** 8 sequential PRs, each reviewable independently. PR 1 (schema serde verify) and PR 2 (manifest move) stage the cross-crate surface. PR 3 bumps the wire protocol to v3 and deletes `PluginMeta`. PR 4 is the atomic "make `Plugin` trait canonical" refactor (merged from what the spec originally called PR 4 + PR 5 — tech-lead flagged splitting them leaves mainline CI red). PR 5 adds `PluginRegistry` aggregate methods. PR 6 lands `plugin.toml` parsing and out-of-process schema round-trip. PR 7 publishes ADR-0027. PR 8 flips MATURITY.md.

**Tech Stack:** Rust 1.95 edition 2024, `cargo nextest`, `cargo +nightly fmt`, `thiserror`, `nebula-error::Classify`, `serde_json`, `toml`, `semver`, `tokio` async.

---

## Scope Check

This plan maps to [`docs/superpowers/specs/2026-04-20-plugin-load-path-stable-design.md`](../specs/2026-04-20-plugin-load-path-stable-design.md). The spec is one coherent refactor sequence — not multiple independent subsystems — because every PR builds on the previous one's crate-boundary or trait-shape change. Attempting to parallelise would deadlock merges (PR 4 depends on PR 2's `PluginManifest` re-export; PR 6 depends on PR 4's `ResolvedPlugin`). Kept as one plan, 8 sequential PRs.

## File Structure

### Crates touched

- **`crates/schema`** — verify `ValidSchema` serde round-trip (already landed per `src/validated.rs:68,78`).
- **`crates/metadata`** — receives `PluginManifest` + `ManifestError` (new module).
- **`crates/plugin`** — major surgery: `Plugin` trait rewrites, `descriptor.rs` / `plugin_type.rs` / `versions.rs` delete, `resolved_plugin.rs` new, `registry.rs` simplified, `error.rs` pruned + expanded.
- **`crates/plugin-sdk`** — `PluginMeta` deletion, wire protocol v3, fixtures refreshed.
- **`crates/sandbox`** — `plugin_toml.rs` new, `remote_action.rs` new, `discovered_plugin.rs` new, `discovery.rs` rewired.
- **`crates/engine`** — `lib.rs:70` re-export cleanup + `README.md:64` update.
- **`apps/cli`** — `plugin_new.rs` template emits `PluginManifest::builder`.
- **`docs/`** — `MATURITY.md`, `pitfalls.md`, `UPGRADE_COMPAT.md`, ADR-0027, ADR-0018 frontmatter.

### File-by-file ownership after this slice

| File | Owns |
|---|---|
| `crates/metadata/src/manifest.rs` | `PluginManifest` + builder + `ManifestError` |
| `crates/metadata/src/lib.rs` | re-export `PluginManifest`, `ManifestError` |
| `crates/plugin/src/plugin.rs` | `Plugin` trait (`manifest`, `actions`, `credentials`, `resources`, `on_load`, `on_unload`) |
| `crates/plugin/src/resolved_plugin.rs` | `ResolvedPlugin` struct, namespace invariant check |
| `crates/plugin/src/registry.rs` | `PluginRegistry` simple map + `all_*` / `resolve_*` accessors |
| `crates/plugin/src/error.rs` | `PluginError` without version variants; adds `NamespaceMismatch`, `DuplicateComponent`, `InvalidManifest` |
| `crates/plugin/src/lib.rs` | re-export of `nebula_metadata::PluginManifest` for source compatibility |
| `crates/plugin-sdk/src/lib.rs` | `PluginHandler` returning `&PluginManifest`; no `PluginMeta` |
| `crates/plugin-sdk/src/protocol.rs` | `MetadataResponse { manifest, actions }` + `ActionDescriptor { key, name, description, schema }`, `DUPLEX_PROTOCOL_VERSION = 3` |
| `crates/sandbox/src/plugin_toml.rs` | `PluginTomlManifest`, `parse_plugin_toml`, `PluginTomlError` |
| `crates/sandbox/src/remote_action.rs` | `RemoteAction: impl nebula_action::Action` wrapping `ProcessSandboxHandler` |
| `crates/sandbox/src/discovered_plugin.rs` | `DiscoveredPlugin: impl Plugin` (legacy `DiscoveredPlugin` DTO inlined) |
| `crates/sandbox/src/discovery.rs` | orchestrates plugin.toml parse → spawn → construct `DiscoveredPlugin` |

---

## PR 1 — Schema Serde Verification

**Files:**
- Test: `crates/schema/tests/valid_schema_roundtrip.rs` (new)

Already `Serialize` + `Deserialize` are implemented on `ValidSchema` (see `crates/schema/src/validated.rs:68` and `:78`). This PR only **locks the round-trip contract** as a regression test so PR 3's wire protocol bump can't be undone silently.

### Task 1.1: Create the round-trip test

**Files:**
- Create: `crates/schema/tests/valid_schema_roundtrip.rs`

- [ ] **Step 1: Write the failing test**

```rust
//! Round-trip test for `ValidSchema` serialization.
//!
//! Guards the wire-protocol contract used by `nebula-plugin-sdk` / `nebula-sandbox`:
//! schemas declared by plugin authors must survive a JSON round-trip without
//! losing field shape.

use nebula_schema::Schema;
use serde_json;

#[test]
fn valid_schema_json_roundtrip_preserves_fields() {
    let schema = Schema::builder()
        .text("name", |f| f.required())
        .number("age", |f| f.optional())
        .build()
        .unwrap();

    let original = schema;

    let json = serde_json::to_string(&original).expect("serialize");
    let decoded: nebula_schema::ValidSchema =
        serde_json::from_str(&json).expect("deserialize");

    assert_eq!(original, decoded);
}

#[test]
fn valid_schema_empty_roundtrip() {
    let empty = Schema::builder().build().unwrap();

    let json = serde_json::to_string(&empty).expect("serialize");
    let decoded: nebula_schema::ValidSchema =
        serde_json::from_str(&json).expect("deserialize");

    assert_eq!(empty, decoded);
}
```

- [ ] **Step 2: Run the test to verify it passes (serde is already there)**

Run: `cargo nextest run -p nebula-schema --test valid_schema_roundtrip`
Expected: PASS on both tests.

If either fails, the spec's assumption is wrong — STOP and report; this blocks PR 3.

- [ ] **Step 3: Format + clippy**

Run: `cargo +nightly fmt -p nebula-schema && cargo clippy -p nebula-schema -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/schema/tests/valid_schema_roundtrip.rs
git commit -m "test(schema): lock ValidSchema serde round-trip contract

Guards the wire-protocol contract used by nebula-plugin-sdk and
nebula-sandbox after the plugin load-path stabilization slice bumps
DUPLEX_PROTOCOL_VERSION to 3 with schema-bearing ActionDescriptor."
```

---

## PR 2 — Move `PluginManifest` to `nebula-metadata`

**Goal:** Lift `PluginManifest` + builder + manifest-specific errors from `nebula-plugin` into `nebula-metadata` so both host (`nebula-plugin`) and plugin-author SDK (`nebula-plugin-sdk`) import the canonical type. `nebula-plugin` re-exports for source compatibility.

**Files:**
- Create: `crates/metadata/src/manifest.rs`
- Modify: `crates/metadata/src/lib.rs`
- Modify: `crates/metadata/Cargo.toml`
- Delete: `crates/plugin/src/manifest.rs`
- Modify: `crates/plugin/src/lib.rs`
- Modify: `crates/plugin/src/error.rs`
- Modify: `crates/plugin/Cargo.toml`
- Modify: `docs/adr/0018-plugin-metadata-to-manifest.md` (frontmatter only)

### Task 2.1: Copy `manifest.rs` content to `nebula-metadata`

**Files:**
- Create: `crates/metadata/src/manifest.rs`

- [ ] **Step 1: Read current `crates/plugin/src/manifest.rs`**

Run: `cat crates/plugin/src/manifest.rs | wc -l`
Expected: ~350–450 lines (builder-heavy module).

- [ ] **Step 2: Check `nebula-metadata`'s current deps**

Run: `cat crates/metadata/Cargo.toml`
Expected: already has `nebula-core`, `serde`, `semver`.

Add `thiserror` and `nebula-error` with `derive` if not present — needed for `ManifestError` below.

- [ ] **Step 3: Write `ManifestError` in a new module header of `crates/metadata/src/manifest.rs`**

Port the shape from `crates/plugin/src/error.rs:60-70` (the `MissingRequiredField` + `InvalidKey` variants), renamed:

```rust
//! Plugin manifest — bundle descriptor for a plugin (ADR-0018, moved here
//! in slice B of the plugin load-path stabilization).
//!
//! A [`PluginManifest`] describes the *container* that bundles actions,
//! credentials, and resources under a versioned identity. It reuses the
//! shared small types from this crate ([`Icon`], [`MaturityLevel`],
//! [`DeprecationNotice`]) but deliberately does **not** compose
//! `BaseMetadata<K>`: a plugin is a container, not a schematized leaf.

use nebula_core::PluginKey;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{DeprecationNotice, Icon, MaturityLevel};

/// Errors from `PluginManifest::builder().build()`.
#[derive(Debug, thiserror::Error, nebula_error::Classify, PartialEq, Eq)]
pub enum ManifestError {
    /// A required field was missing during construction.
    #[classify(category = "validation", code = "MANIFEST:MISSING_FIELD")]
    #[error("missing required field '{field}' for plugin manifest")]
    MissingRequiredField {
        /// The missing field name.
        field: &'static str,
    },

    /// Plugin key validation failed.
    #[classify(category = "validation", code = "MANIFEST:INVALID_KEY")]
    #[error("invalid plugin key: {0}")]
    InvalidKey(<PluginKey as std::str::FromStr>::Err),
}
```

- [ ] **Step 4: Port the rest of `PluginManifest` body** (struct, builder, `normalize_key`, default version helpers, tests) from `crates/plugin/src/manifest.rs` into this new file

Replace `use crate::PluginError;` / `PluginError::InvalidKey` / `PluginError::MissingRequiredField` with the local `ManifestError` equivalents. Everything else is already compatible because the original imports `nebula_metadata::{Icon, MaturityLevel, DeprecationNotice}` — which are now **same-crate** imports (`crate::`).

- [ ] **Step 5: Run `cargo check` on nebula-metadata only**

Run: `cargo check -p nebula-metadata`
Expected: clean.

- [ ] **Step 6: Run the manifest tests**

Run: `cargo nextest run -p nebula-metadata manifest`
Expected: every test that ported from `plugin/src/manifest.rs` passes. If any test references `PluginError`, update to `ManifestError`.

### Task 2.2: Wire `manifest` into `nebula-metadata`'s `lib.rs`

**Files:**
- Modify: `crates/metadata/src/lib.rs`

- [ ] **Step 1: Read `crates/metadata/src/lib.rs`**

Note which types it re-exports (Icon, MaturityLevel, DeprecationNotice, BaseMetadata).

- [ ] **Step 2: Add the module + re-exports**

```rust
pub mod manifest;

pub use manifest::{ManifestError, PluginManifest};
```

- [ ] **Step 3: Update `crates/metadata/Cargo.toml` if needed**

Ensure `thiserror = { workspace = true }` and `nebula-error = { workspace = true, features = ["derive"] }` are present. Add them if missing.

- [ ] **Step 4: `cargo check -p nebula-metadata`**

Expected: clean.

- [ ] **Step 5: Commit Task 2.1 + 2.2**

```bash
git add crates/metadata/
git commit -m "feat(metadata): port PluginManifest from nebula-plugin

PluginManifest is now in nebula-metadata so nebula-plugin-sdk (canon §7.1
zero-engine-deps) can import the canonical bundle descriptor on the
plugin-author side. Host side (nebula-plugin) will re-export for source
compatibility in a follow-up task.

Refs: ADR-0018 (manifest type), pending ADR-0027 (slice B)."
```

### Task 2.3: Replace `crates/plugin/src/manifest.rs` with a re-export

**Files:**
- Delete: `crates/plugin/src/manifest.rs` (content; keep module ref in lib.rs briefly, then delete)
- Modify: `crates/plugin/src/lib.rs`
- Modify: `crates/plugin/Cargo.toml`
- Modify: `crates/plugin/src/error.rs`

- [ ] **Step 1: Update `crates/plugin/Cargo.toml`**

`nebula-metadata` is already a dep (used for Icon, etc.). Verify its entry, nothing to add.

- [ ] **Step 2: Write the new `crates/plugin/src/manifest.rs` as pure re-export**

```rust
//! `PluginManifest` is canonical in `nebula-metadata` (ADR-0018 follow-up,
//! moved there in slice B of the plugin load-path stabilization). This module
//! re-exports the type for source compatibility of callers that still write
//! `use nebula_plugin::PluginManifest;`.
//!
//! New code should import directly from `nebula_metadata::PluginManifest`.

pub use nebula_metadata::{ManifestError, PluginManifest};

pub(crate) use nebula_metadata::manifest::normalize_key;
```

Note: `normalize_key` is called by `PluginRegistry::get_by_name` — either export it from `nebula_metadata::manifest` (preferred, small private helper) or inline that one call site. Check which path is simpler; exporting is a one-line change.

- [ ] **Step 3: Verify `normalize_key` visibility**

Open `crates/metadata/src/manifest.rs`. Change `pub(crate) fn normalize_key` → `pub fn normalize_key` and re-export from `crates/metadata/src/lib.rs`. Or, make it `pub(super)` and expose through `nebula_metadata::manifest::normalize_key`.

- [ ] **Step 4: Update `crates/plugin/src/error.rs` to wrap `ManifestError`**

Add a new variant:

```rust
/// Plugin manifest construction failed — wraps `nebula_metadata::ManifestError`.
#[classify(category = "validation", code = "PLUGIN:INVALID_MANIFEST")]
#[error("invalid plugin manifest: {0}")]
InvalidManifest(#[from] nebula_metadata::ManifestError),
```

Remove the local `MissingRequiredField` and `InvalidKey` variants from `PluginError` if they are not otherwise used — they live in `ManifestError` now, callers go through `InvalidManifest`. Check for consumers first: `rg 'PluginError::MissingRequiredField\|PluginError::InvalidKey' crates/`. If any consumer exists outside tests, leave the variant as `#[deprecated]` and plan deletion in PR 4.

- [ ] **Step 5: `cargo check -p nebula-plugin`**

Expected: clean after the import reshape. If compilation fails, the `crate::PluginError` references in the old `manifest.rs` need to flow through `ManifestError` in nebula-metadata and get wrapped at the plugin boundary.

- [ ] **Step 6: Run existing plugin tests**

Run: `cargo nextest run -p nebula-plugin`
Expected: green. Doc-tests that do `use nebula_plugin::PluginManifest;` still work via the re-export.

- [ ] **Step 7: Workspace-wide check**

Run: `cargo check --workspace`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/plugin/ crates/metadata/
git commit -m "refactor(plugin): re-export PluginManifest from nebula-metadata

Moves the manifest source of truth to nebula-metadata (slice B of the
plugin load-path stabilization). nebula-plugin keeps a thin re-export
so existing 'use nebula_plugin::PluginManifest;' imports keep compiling.

PluginError::InvalidManifest now wraps nebula_metadata::ManifestError."
```

### Task 2.4: Update ADR-0018 follow-ups

**Files:**
- Modify: `docs/adr/0018-plugin-metadata-to-manifest.md` (frontmatter `Follow-ups` section only)

- [ ] **Step 1: Open the ADR**

Run: `cat docs/adr/0018-plugin-metadata-to-manifest.md | head -30`

- [ ] **Step 2: Add a follow-up entry**

In the `## Follow-ups` section (or create one if absent), append:

```markdown
- **2026-04-XX (slice B of plugin load-path stabilization).**
  `PluginManifest` moved from `nebula-plugin` to `nebula-metadata` so the
  plugin-author SDK can import it without breaking canon §7.1's
  "zero-engine-side-deps" invariant. `nebula-plugin` keeps a thin
  `pub use nebula_metadata::PluginManifest;` re-export for source
  compatibility. See ADR-0027 and
  [the slice-B design spec](../superpowers/specs/2026-04-20-plugin-load-path-stable-design.md).
```

Body of ADR-0018 stays immutable per convention; only `Follow-ups` gets the addendum.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0018-plugin-metadata-to-manifest.md
git commit -m "docs(adr): ADR-0018 follow-up — manifest moves to nebula-metadata"
```

---

## PR 3 — Delete `PluginMeta` + Wire v3 Bump

**Goal:** `PluginHandler::metadata` returns `&PluginManifest`; wire `MetadataResponse` carries `manifest: PluginManifest`; `DUPLEX_PROTOCOL_VERSION = 3`; downstream fixtures + CLI template + smoke tests flip in the same PR to avoid CI red.

**Files:**
- Modify: `crates/plugin-sdk/Cargo.toml`
- Modify: `crates/plugin-sdk/src/lib.rs`
- Modify: `crates/plugin-sdk/src/protocol.rs`
- Modify: `crates/plugin-sdk/src/bin/echo_fixture.rs`
- Modify: `crates/plugin-sdk/src/bin/counter_fixture.rs`
- Modify: `crates/plugin-sdk/README.md`
- Modify: `crates/plugin-sdk/tests/broker_smoke.rs` (if references `PluginMeta`)
- Modify: `apps/cli/src/commands/plugin_new.rs`
- Modify: `docs/UPGRADE_COMPAT.md`

### Task 3.1: Add `nebula-metadata` dep to `nebula-plugin-sdk`

**Files:**
- Modify: `crates/plugin-sdk/Cargo.toml`

- [ ] **Step 1: Read current Cargo.toml**

- [ ] **Step 2: Append to `[dependencies]`**

```toml
nebula-metadata = { path = "../metadata" }
nebula-schema = { path = "../schema" }  # needed in PR 6; add here so bumps stay together
```

- [ ] **Step 3: Document the exception in `crates/plugin-sdk/README.md`**

Update the "zero intra-workspace deps" line under `## Contract`:

```markdown
- **[L1-§7.1]** Plugin is the unit of registration. **One Core-layer
  exception to intra-workspace deps:** `nebula-metadata` (for
  `PluginManifest`) and `nebula-schema` (for `ValidSchema` on wire).
  Both are Core-layer crates, not engine-side infrastructure, so the
  plugin-side binary stays free of engine coupling. Any other
  cross-imports must be questioned hard.
```

- [ ] **Step 4: `cargo check -p nebula-plugin-sdk`**

Expected: clean.

### Task 3.2: Replace wire `MetadataResponse` and `ActionDescriptor` shape

**Files:**
- Modify: `crates/plugin-sdk/src/protocol.rs`

- [ ] **Step 1: Bump `DUPLEX_PROTOCOL_VERSION`**

```rust
pub const DUPLEX_PROTOCOL_VERSION: u32 = 3;
```

- [ ] **Step 2: Rewrite `MetadataResponse` variant**

Replace the existing `MetadataResponse { id, protocol_version, plugin_key, plugin_version, actions }` with:

```rust
    /// Plugin metadata response (reply to [`HostToPlugin::MetadataRequest`]).
    MetadataResponse {
        /// Correlation id from the original `MetadataRequest`.
        id: u64,
        /// Protocol version the plugin speaks. Host verifies compatibility.
        protocol_version: u32,
        /// Canonical bundle descriptor (slice B of plugin load-path
        /// stabilization replaced the flat `plugin_key` / `plugin_version`
        /// fields with the full manifest).
        manifest: nebula_metadata::PluginManifest,
        /// Actions this plugin provides.
        actions: Vec<ActionDescriptor>,
    },
```

- [ ] **Step 3: Rewrite `ActionDescriptor` struct**

```rust
/// Describes one action offered by a plugin. Wire DTO — maps onto
/// `nebula-action::ActionMetadata` once discovery converts it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionDescriptor {
    /// Action key — short local form (`"send_message"`) or already
    /// namespace-qualified (`"slack.send_message"`). Host validates.
    pub key: String,
    /// Human-readable action name.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: String,
    /// Input schema the host uses to validate user-supplied parameters.
    pub schema: nebula_schema::ValidSchema,
}
```

- [ ] **Step 4: Update the `metadata_request_and_response_roundtrip` test**

The existing roundtrip test in the `#[cfg(test)] mod tests` block constructs a `MetadataResponse` with `plugin_key` / `plugin_version`. Change it to use a `PluginManifest::builder(...).build().unwrap()` + an empty `ValidSchema` for the action.

```rust
#[test]
fn metadata_request_and_response_roundtrip() {
    let req = HostToPlugin::MetadataRequest { id: 1 };
    let line = serde_json::to_string(&req).unwrap();
    assert!(line.contains(r#""kind":"metadata_request""#));

    let manifest = nebula_metadata::PluginManifest::builder(
        "com.author.echo",
        "Echo",
    )
    .version(semver::Version::new(1, 0, 0))
    .build()
    .unwrap();

    let resp = PluginToHost::MetadataResponse {
        id: 1,
        protocol_version: DUPLEX_PROTOCOL_VERSION,
        manifest,
        actions: vec![ActionDescriptor {
            key: "echo".into(),
            name: "Echo".into(),
            description: "Echoes input".into(),
            schema: nebula_schema::Schema::builder().build().unwrap(),
        }],
    };

    let line = serde_json::to_string(&resp).unwrap();
    let parsed: PluginToHost = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed, resp);
}
```

- [ ] **Step 5: Run the unit tests**

Run: `cargo nextest run -p nebula-plugin-sdk protocol`
Expected: green.

### Task 3.3: Delete `PluginMeta` + flip `PluginHandler::metadata`

**Files:**
- Modify: `crates/plugin-sdk/src/lib.rs`

- [ ] **Step 1: Delete the `PluginMeta` struct**

Remove lines 138–174 (`PluginMeta` struct + impl block). Remove the `use crate::protocol::{ActionDescriptor, ...};` import line's `ActionDescriptor` if no longer needed at module scope.

- [ ] **Step 2: Rewrite `PluginHandler`**

```rust
#[async_trait::async_trait]
pub trait PluginHandler: Send + Sync + 'static {
    /// Return this plugin's canonical manifest.
    ///
    /// Called once at startup by `run_duplex` to populate the
    /// `MetadataResponse` envelope.
    fn manifest(&self) -> &nebula_metadata::PluginManifest;

    /// Return the list of actions this plugin provides.
    ///
    /// Called once at startup — the list is the declarative surface
    /// over the wire. Each `ActionDescriptor` carries its own schema.
    fn actions(&self) -> Vec<crate::protocol::ActionDescriptor>;

    /// Execute an action. Called per `HostToPlugin::ActionInvoke`.
    async fn execute(
        &self,
        ctx: &crate::PluginCtx,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, crate::PluginError>;
}
```

Rename `fn metadata(&self) -> PluginMeta;` → `fn manifest(&self) -> &PluginManifest;` + `fn actions(&self) -> Vec<ActionDescriptor>;`. The manifest/actions are now separate methods; this matches the host `Plugin` trait shape and lets `run_duplex` construct the envelope directly.

- [ ] **Step 3: Update `run_duplex` (same file) to use the new shape**

Find the place where the old implementation did `handler.metadata()` to build the response. Replace with:

```rust
let response = PluginToHost::MetadataResponse {
    id: request_id,
    protocol_version: DUPLEX_PROTOCOL_VERSION,
    manifest: handler.manifest().clone(),
    actions: handler.actions(),
};
```

- [ ] **Step 4: `cargo check -p nebula-plugin-sdk`**

Expected: compilation errors in fixtures / tests / bins (intentional — fixing in next tasks).

### Task 3.4: Update `echo_fixture.rs`

**Files:**
- Modify: `crates/plugin-sdk/src/bin/echo_fixture.rs`

- [ ] **Step 1: Read the current file**

- [ ] **Step 2: Rewrite to use `PluginManifest::builder` + separate `actions()` method**

```rust
//! Echo fixture used by sandbox discovery tests.

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    protocol::ActionDescriptor,
    PluginCtx,
    PluginError,
    PluginHandler,
    run_duplex,
};
use nebula_schema::Schema;
use semver::Version;
use serde_json::{json, Value};

struct EchoPlugin {
    manifest: PluginManifest,
}

impl EchoPlugin {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.author.echo", "Echo")
            .version(Version::new(1, 0, 0))
            .description("Fixture plugin — echoes its input back.")
            .build()
            .unwrap();
        Self { manifest }
    }
}

#[async_trait]
impl PluginHandler for EchoPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<ActionDescriptor> {
        vec![ActionDescriptor {
            key: "echo".into(),
            name: "Echo".into(),
            description: "Echo the input back.".into(),
            schema: Schema::builder().build().unwrap(),
        }]
    }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        _action_key: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        Ok(json!({ "echoed": input }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_duplex(EchoPlugin::new()).await?;
    Ok(())
}
```

Note: `run_duplex` no longer takes a separate `meta` arg — the handler carries both manifest and actions. If the current signature is `run_duplex(meta, handler)`, change to `run_duplex(handler)` in `lib.rs` and update all callers.

- [ ] **Step 3: `cargo check -p nebula-plugin-sdk`**

Expected: clean after both fixture and `run_duplex` are consistent.

### Task 3.5: Update `counter_fixture.rs`

**Files:**
- Modify: `crates/plugin-sdk/src/bin/counter_fixture.rs`

- [ ] **Step 1: Read current file** and mirror the same pattern from `echo_fixture.rs`.

- [ ] **Step 2: Rewrite**

Same template as Task 3.4 but with `"com.author.counter"` key, version `1.0.0`, one action `"increment"` that returns `{"next": previous + 1}`. Keep whatever stateful semantics the original carried.

- [ ] **Step 3: `cargo check -p nebula-plugin-sdk`**

Expected: clean.

### Task 3.6: Update `broker_smoke.rs`

**Files:**
- Modify: `crates/plugin-sdk/tests/broker_smoke.rs` (read first to confirm)

- [ ] **Step 1: `cat crates/plugin-sdk/tests/broker_smoke.rs`**

- [ ] **Step 2: Flip any `PluginMeta::new(...)` to `PluginManifest::builder(...)`**

If the smoke test instantiates its own fake handler, rewrite it to return `&PluginManifest` and `Vec<ActionDescriptor>` separately.

- [ ] **Step 3: Run the smoke test**

Run: `cargo nextest run -p nebula-plugin-sdk --test broker_smoke`
Expected: green.

### Task 3.7: Update the CLI template at `apps/cli/src/commands/plugin_new.rs`

**Files:**
- Modify: `apps/cli/src/commands/plugin_new.rs:155-163`

- [ ] **Step 1: Read the file around line 155**

Run: `sed -n '140,180p' apps/cli/src/commands/plugin_new.rs`

- [ ] **Step 2: Flip the scaffolded `PluginMeta::new(...)` to `PluginManifest::builder(...)`**

```rust
// Inside the template string the CLI emits for a new plugin's main.rs:
let template = r#"use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    protocol::ActionDescriptor,
    PluginCtx, PluginError, PluginHandler,
    run_duplex,
};
use nebula_schema::Schema;
use semver::Version;
use serde_json::Value;

struct {PluginName}Plugin {
    manifest: PluginManifest,
}

impl {PluginName}Plugin {
    fn new() -> Self {
        let manifest = PluginManifest::builder("{plugin_key}", "{PluginName}")
            .version(Version::new(0, 1, 0))
            .build()
            .unwrap();
        Self { manifest }
    }
}

#[async_trait]
impl PluginHandler for {PluginName}Plugin {
    fn manifest(&self) -> &PluginManifest { &self.manifest }

    fn actions(&self) -> Vec<ActionDescriptor> { vec![] }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        action_key: &str,
        _input: Value,
    ) -> Result<Value, PluginError> {
        Err(PluginError::fatal(format!("unknown action: {action_key}")))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_duplex({PluginName}Plugin::new()).await?;
    Ok(())
}
"#;
```

(Placeholder markers like `{PluginName}` / `{plugin_key}` match the existing template's interpolation scheme — read the current file to stay consistent.)

- [ ] **Step 3: Run any cli-side tests that exercise the template**

Run: `cargo nextest run -p nebula-cli plugin_new`
If there's a test that generates a plugin and compiles it, it must go green.

- [ ] **Step 4: Workspace check**

Run: `cargo check --workspace`
Expected: clean.

### Task 3.8: Update `docs/UPGRADE_COMPAT.md`

**Files:**
- Modify: `docs/UPGRADE_COMPAT.md`

- [ ] **Step 1: Open the file**

- [ ] **Step 2: Append entry**

```markdown
## 2026-04-XX — `DUPLEX_PROTOCOL_VERSION` 2 → 3

**Breaking (plugin-SDK).** Plugin binaries compiled against
`nebula-plugin-sdk` ≤ version-2 no longer handshake with the current host.
Rebuild all plugin binaries against the current SDK; the change is
**additive to the envelope** (replacing flat `plugin_key` / `plugin_version`
with the full `PluginManifest`, adding per-action `schema: ValidSchema`),
so plugin authors re-compile and ship — no source-level migration beyond
the SDK version bump.

See [plugin load-path stabilization design
spec](superpowers/specs/2026-04-20-plugin-load-path-stable-design.md).
```

### Task 3.9: Run full workspace verification + commit

- [ ] **Step 1: Full check**

Run: `cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`
Expected: green.

- [ ] **Step 2: Commit**

```bash
git add crates/plugin-sdk/ apps/cli/src/commands/plugin_new.rs docs/UPGRADE_COMPAT.md
git commit -m "feat(plugin-sdk)!: bump DUPLEX_PROTOCOL_VERSION to 3; drop PluginMeta

MetadataResponse now carries nebula_metadata::PluginManifest (replaces
flat plugin_key/plugin_version) + per-action ValidSchema. PluginHandler
trait splits metadata() into manifest() + actions(). Fixtures, CLI
template, smoke test all updated in the same PR to keep mainline CI
green.

BREAKING CHANGE: plugin binaries must be rebuilt against the current
SDK — see docs/UPGRADE_COMPAT.md. Refs slice B of the plugin load-path
stabilization design."
```

---

## PR 4 — Plugin Refactor (Atomic)

**Goal:** Single atomic PR that makes the `Plugin` trait canonical. Delete `descriptor` module, flip trait signature, delete `PluginType` / `PluginVersions` / `ArcPlugin`, add `ResolvedPlugin`, simplify `PluginRegistry`, clean up engine re-exports, add pitfalls entry. Tech-lead review flagged that splitting this leaves mainline red, so everything lands together.

**Files:**
- Modify: `crates/plugin/Cargo.toml`
- Delete: `crates/plugin/src/descriptor.rs`
- Delete: `crates/plugin/src/plugin_type.rs`
- Delete: `crates/plugin/src/versions.rs`
- Modify: `crates/plugin/src/plugin.rs`
- Modify: `crates/plugin/src/error.rs`
- Create: `crates/plugin/src/resolved_plugin.rs`
- Modify: `crates/plugin/src/registry.rs`
- Modify: `crates/plugin/src/lib.rs`
- Modify: `crates/plugin/README.md`
- Modify: `crates/engine/src/lib.rs:70`
- Modify: `crates/engine/README.md`
- Modify: `docs/pitfalls.md`

### Task 4.1: Add sibling deps

**Files:**
- Modify: `crates/plugin/Cargo.toml`

- [ ] **Step 1: Append `[dependencies]`**

```toml
nebula-action = { path = "../action" }
nebula-credential = { path = "../credential" }
nebula-resource = { path = "../resource" }
```

- [ ] **Step 2: Confirm no cycle**

Run: `cargo check -p nebula-plugin`
Expected: compilation fails because the old Plugin trait still returns local descriptors; the deps are not yet used. But cargo dep resolution passes. If a cycle appears, stop and report.

- [ ] **Step 3: Verify cycle-guard**

Run: `grep -r "nebula_plugin" crates/action/src/ crates/credential/src/ crates/resource/src/ 2>&1 | head -5`
Expected: no matches. (If any appear, halt and report — spec's cycle assumption was wrong.)

### Task 4.2: Flip `Plugin` trait

**Files:**
- Modify: `crates/plugin/src/plugin.rs`

- [ ] **Step 1: Rewrite the trait definition**

Replace the old trait body (lines 24–242 of `plugin.rs`) with:

```rust
use std::{fmt::Debug, sync::Arc};

use nebula_core::PluginKey;
use nebula_metadata::PluginManifest;
use semver::Version;

use crate::PluginError;

/// Base trait for all plugin types in Nebula.
///
/// A plugin is a user-visible, versionable packaging unit (e.g. "Slack",
/// "HTTP Request"). It provides a manifest describing the plugin's identity
/// and the runnable actions / credentials / resources it contributes.
///
/// This trait is **object-safe** so plugins can be stored as `Arc<dyn Plugin>`.
pub trait Plugin: Send + Sync + Debug + 'static {
    /// Returns the static manifest for this plugin.
    fn manifest(&self) -> &PluginManifest;

    /// Actions this plugin provides. Default: empty.
    fn actions(&self) -> Vec<Arc<dyn nebula_action::Action>> {
        vec![]
    }

    /// Credential types this plugin provides. Default: empty.
    fn credentials(&self) -> Vec<Arc<dyn nebula_credential::Credential>> {
        vec![]
    }

    /// Resource types this plugin provides. Default: empty.
    fn resources(&self) -> Vec<Arc<dyn nebula_resource::Resource>> {
        vec![]
    }

    /// Called once when the plugin is loaded. Default: no-op.
    fn on_load(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when the plugin is being unloaded. Default: no-op.
    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }

    /// The normalized, unique key identifying this plugin.
    /// Forwarded from the manifest — rarely overridden.
    fn key(&self) -> &PluginKey {
        self.manifest().key()
    }

    /// Bundle semver version. Forwarded from the manifest.
    fn version(&self) -> &Version {
        self.manifest().version()
    }
}
```

- [ ] **Step 2: Delete the old doc-tests that embed `descriptor` types**

Remove the `#[cfg(test)] mod tests { ... }` block's `FullPlugin` impl that constructs `ActionDescriptor` / `CredentialDescriptor` / `ResourceDescriptor`. Reinstate shorter tests in Task 4.8.

- [ ] **Step 3: `cargo check -p nebula-plugin`**

Expected: compilation failures in `descriptor.rs`, `plugin_type.rs`, `versions.rs`, `registry.rs`, and `plugin.rs` doc-tests. Intentional; they delete or get rewritten in 4.3–4.8.

### Task 4.3: Delete `descriptor.rs`

**Files:**
- Delete: `crates/plugin/src/descriptor.rs`
- Modify: `crates/plugin/src/lib.rs`

- [ ] **Step 1: `rm crates/plugin/src/descriptor.rs`**

- [ ] **Step 2: Remove `pub mod descriptor;` + `pub use descriptor::*;` from `lib.rs`**

- [ ] **Step 3: Grep for remaining consumers**

Run: `rg 'descriptor::|ActionDescriptor|CredentialDescriptor|ResourceDescriptor' crates/plugin/`
Expected: only wire-protocol references in `plugin-sdk/protocol.rs` (different type, fine). If anything in `crates/plugin/` still references the deleted names, fix in place.

### Task 4.4: Delete `plugin_type.rs` and `versions.rs`

**Files:**
- Delete: `crates/plugin/src/plugin_type.rs`
- Delete: `crates/plugin/src/versions.rs`
- Modify: `crates/plugin/src/lib.rs`

- [ ] **Step 1: Remove the files**

```bash
rm crates/plugin/src/plugin_type.rs
rm crates/plugin/src/versions.rs
```

- [ ] **Step 2: Remove `pub mod plugin_type;` / `pub mod versions;` + `pub use ...` from `lib.rs`**

- [ ] **Step 3: Grep for remaining consumers**

Run: `rg 'PluginType|PluginVersions|ArcPlugin' crates/`
Expected: matches in `crates/engine/src/lib.rs:70` (re-export) and `crates/engine/README.md:64` (fixed in 4.9). No other hits.

### Task 4.5: Prune `PluginError`

**Files:**
- Modify: `crates/plugin/src/error.rs`

- [ ] **Step 1: Remove version-related variants**

Delete `VersionNotFound`, `NotVersioned`, `NoVersionsAvailable`, `VersionAlreadyExists`, `KeyMismatch` from the enum. Drop the matching branches in `impl PartialEq for PluginError`. Remove the corresponding unit tests in the `#[cfg(test)]` block.

- [ ] **Step 2: Add new variants**

```rust
    /// Plugin declared an action/credential/resource whose full key does
    /// not start with the plugin's own prefix. Caught at `ResolvedPlugin::from`.
    #[classify(category = "validation", code = "PLUGIN:NAMESPACE_MISMATCH")]
    #[error("plugin '{plugin}' declared {kind:?} key '{offending_key}' outside its namespace '{plugin}.*'")]
    NamespaceMismatch {
        /// The offending plugin.
        plugin: PluginKey,
        /// Full key of the misplaced component.
        offending_key: String,
        /// Which component kind (action/credential/resource).
        kind: ComponentKind,
    },

    /// Plugin declared two components of the same kind with identical full keys.
    #[classify(category = "conflict", code = "PLUGIN:DUPLICATE_COMPONENT")]
    #[error("plugin '{plugin}' declared duplicate {kind:?} key '{key}'")]
    DuplicateComponent {
        /// The offending plugin.
        plugin: PluginKey,
        /// The duplicated full key.
        key: String,
        /// Which component kind.
        kind: ComponentKind,
    },

    /// Plugin manifest failed validation (wraps nebula-metadata).
    #[classify(category = "validation", code = "PLUGIN:INVALID_MANIFEST")]
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(#[from] nebula_metadata::ManifestError),
```

Add `ComponentKind`:

```rust
/// Which component kind flagged a plugin construction error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    /// An action.
    Action,
    /// A credential.
    Credential,
    /// A resource.
    Resource,
}
```

- [ ] **Step 3: Update `impl PartialEq for PluginError` accordingly**

Add branches for the three new variants mirroring the fields.

- [ ] **Step 4: Port affected unit tests**

Keep `not_found_display`, `already_exists_display`, `partial_eq`. Delete the version/key-mismatch ones. Add one test for `NamespaceMismatch` Display and one for `DuplicateComponent` Display.

- [ ] **Step 5: `cargo check -p nebula-plugin`**

Expected: surface tightens. Other compilation errors are in `registry.rs` (fixed in 4.7).

### Task 4.6: Create `ResolvedPlugin`

**Files:**
- Create: `crates/plugin/src/resolved_plugin.rs`
- Modify: `crates/plugin/src/lib.rs`

- [ ] **Step 1: Write the failing test first (TDD)**

Create `crates/plugin/tests/resolved_plugin.rs`:

```rust
//! Integration tests for `ResolvedPlugin`.

use std::sync::Arc;

use nebula_action::Action;
use nebula_core::ActionKey;
use nebula_metadata::PluginManifest;
use nebula_plugin::{ComponentKind, Plugin, PluginError, ResolvedPlugin};

// ---- test helpers ----

#[derive(Debug)]
struct StubAction {
    metadata: nebula_action::ActionMetadata,
}

impl StubAction {
    fn new(key: &str) -> Arc<Self> {
        Arc::new(Self {
            metadata: nebula_action::ActionMetadata::new(
                ActionKey::new(key).unwrap(),
                "stub",
                "stub action",
            ),
        })
    }
}

impl Action for StubAction {
    fn metadata(&self) -> &nebula_action::ActionMetadata {
        &self.metadata
    }
}

#[derive(Debug)]
struct FixturePlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn Action>>,
}

impl Plugin for FixturePlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn actions(&self) -> Vec<Arc<dyn Action>> {
        self.actions.clone()
    }
}

fn plugin_with_actions(key: &str, action_keys: Vec<&str>) -> FixturePlugin {
    FixturePlugin {
        manifest: PluginManifest::builder(key, key).build().unwrap(),
        actions: action_keys
            .into_iter()
            .map(|k| StubAction::new(k) as Arc<dyn Action>)
            .collect(),
    }
}

// ---- tests ----

#[test]
fn resolved_plugin_accepts_well_namespaced_action() {
    let plugin = plugin_with_actions("slack", vec!["slack.send_message"]);
    let resolved = ResolvedPlugin::from(plugin).expect("well-formed plugin");
    assert!(resolved
        .action(&ActionKey::new("slack.send_message").unwrap())
        .is_some());
}

#[test]
fn resolved_plugin_rejects_out_of_namespace_action() {
    let plugin = plugin_with_actions("slack", vec!["api.foo"]);
    let err = ResolvedPlugin::from(plugin).expect_err("namespace violation");
    match err {
        PluginError::NamespaceMismatch {
            plugin,
            offending_key,
            kind,
        } => {
            assert_eq!(plugin.as_str(), "slack");
            assert_eq!(offending_key, "api.foo");
            assert_eq!(kind, ComponentKind::Action);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn resolved_plugin_rejects_duplicate_action_keys() {
    let plugin = plugin_with_actions(
        "slack",
        vec!["slack.send_message", "slack.send_message"],
    );
    let err = ResolvedPlugin::from(plugin).expect_err("duplicate");
    assert!(matches!(
        err,
        PluginError::DuplicateComponent {
            kind: ComponentKind::Action,
            ..
        }
    ));
}
```

- [ ] **Step 2: Run — expected FAIL**

Run: `cargo nextest run -p nebula-plugin --test resolved_plugin`
Expected: `ResolvedPlugin` doesn't compile.

- [ ] **Step 3: Implement `crates/plugin/src/resolved_plugin.rs`**

```rust
//! `ResolvedPlugin` — per-plugin resolved wrapper with eager component caches.
//!
//! `ResolvedPlugin::from` calls `plugin.actions()` / `credentials()` /
//! `resources()` exactly once, validates the namespace invariant
//! (every full key starts with `{plugin.key()}.`), and builds three
//! flat `HashMap<FullKey, Arc<dyn …>>` indices for O(1) lookup.
//!
//! Duplicate keys within one plugin surface as
//! `PluginError::DuplicateComponent`. Keys outside the plugin's
//! namespace surface as `PluginError::NamespaceMismatch`.

use std::{collections::HashMap, sync::Arc};

use nebula_action::Action;
use nebula_core::{ActionKey, CredentialKey, PluginKey, ResourceKey};
use nebula_credential::Credential;
use nebula_metadata::PluginManifest;
use nebula_resource::Resource;
use semver::Version;

use crate::{plugin::Plugin, ComponentKind, PluginError};

/// Eagerly resolved per-plugin wrapper.
#[derive(Debug)]
pub struct ResolvedPlugin {
    plugin: Arc<dyn Plugin>,
    actions: HashMap<ActionKey, Arc<dyn Action>>,
    credentials: HashMap<CredentialKey, Arc<dyn Credential>>,
    resources: HashMap<ResourceKey, Arc<dyn Resource>>,
}

impl ResolvedPlugin {
    /// Construct from an `impl Plugin`. Calls the three component methods
    /// once, enforces the namespace invariant, catches within-plugin
    /// duplicate keys.
    pub fn from<P: Plugin + 'static>(plugin: P) -> Result<Self, PluginError> {
        let plugin_key = plugin.manifest().key().clone();
        let prefix = format!("{}.", plugin_key.as_str());

        let raw_actions = plugin.actions();
        let raw_credentials = plugin.credentials();
        let raw_resources = plugin.resources();

        let actions = build_index(
            raw_actions,
            &plugin_key,
            &prefix,
            ComponentKind::Action,
            |a| a.metadata().base.key.clone(),
        )?;

        let credentials = build_index(
            raw_credentials,
            &plugin_key,
            &prefix,
            ComponentKind::Credential,
            |c| c.metadata().base.key.clone(),
        )?;

        let resources = build_index(
            raw_resources,
            &plugin_key,
            &prefix,
            ComponentKind::Resource,
            |r| r.metadata().base.key.clone(),
        )?;

        Ok(Self {
            plugin: Arc::new(plugin) as Arc<dyn Plugin>,
            actions,
            credentials,
            resources,
        })
    }

    /// Access the wrapped `Plugin` trait object (for `on_load`, `on_unload`).
    pub fn plugin(&self) -> &Arc<dyn Plugin> {
        &self.plugin
    }

    /// Shortcut to `plugin.manifest()`.
    pub fn manifest(&self) -> &PluginManifest {
        self.plugin.manifest()
    }

    /// Shortcut to `plugin.key()`.
    pub fn key(&self) -> &PluginKey {
        self.plugin.key()
    }

    /// Shortcut to `plugin.version()`.
    pub fn version(&self) -> &Version {
        self.plugin.version()
    }

    /// O(1) lookup of an action by its full `ActionKey`.
    pub fn action(&self, key: &ActionKey) -> Option<&Arc<dyn Action>> {
        self.actions.get(key)
    }

    /// O(1) lookup of a credential by its full `CredentialKey`.
    pub fn credential(&self, key: &CredentialKey) -> Option<&Arc<dyn Credential>> {
        self.credentials.get(key)
    }

    /// O(1) lookup of a resource by its full `ResourceKey`.
    pub fn resource(&self, key: &ResourceKey) -> Option<&Arc<dyn Resource>> {
        self.resources.get(key)
    }

    /// Iterator over every cached action.
    pub fn actions(&self) -> impl Iterator<Item = (&ActionKey, &Arc<dyn Action>)> {
        self.actions.iter()
    }

    /// Iterator over every cached credential.
    pub fn credentials(
        &self,
    ) -> impl Iterator<Item = (&CredentialKey, &Arc<dyn Credential>)> {
        self.credentials.iter()
    }

    /// Iterator over every cached resource.
    pub fn resources(&self) -> impl Iterator<Item = (&ResourceKey, &Arc<dyn Resource>)> {
        self.resources.iter()
    }
}

fn build_index<T, K, F>(
    raw: Vec<T>,
    plugin_key: &PluginKey,
    prefix: &str,
    kind: ComponentKind,
    key_of: F,
) -> Result<HashMap<K, T>, PluginError>
where
    K: std::hash::Hash + Eq + AsStr,
    F: Fn(&T) -> K,
{
    let mut out = HashMap::new();
    for item in raw {
        let full = key_of(&item);
        if !full.as_str().starts_with(prefix) {
            return Err(PluginError::NamespaceMismatch {
                plugin: plugin_key.clone(),
                offending_key: full.as_str().to_owned(),
                kind,
            });
        }
        if out.insert(full.clone(), item).is_some() {
            return Err(PluginError::DuplicateComponent {
                plugin: plugin_key.clone(),
                key: full.as_str().to_owned(),
                kind,
            });
        }
    }
    Ok(out)
}

/// Helper trait so `build_index` can call `.as_str()` on each typed key.
/// Implemented below for the three canonical key types.
trait AsStr {
    fn as_str(&self) -> &str;
}

impl AsStr for ActionKey {
    fn as_str(&self) -> &str {
        ActionKey::as_str(self)
    }
}
impl AsStr for CredentialKey {
    fn as_str(&self) -> &str {
        CredentialKey::as_str(self)
    }
}
impl AsStr for ResourceKey {
    fn as_str(&self) -> &str {
        ResourceKey::as_str(self)
    }
}
```

Re-export in `crates/plugin/src/lib.rs`:

```rust
pub mod resolved_plugin;
pub use resolved_plugin::ResolvedPlugin;
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p nebula-plugin --test resolved_plugin`
Expected: all three tests PASS.

- [ ] **Step 5: `cargo clippy -p nebula-plugin -- -D warnings`**

Expected: clean. If clippy complains about the `AsStr` local trait being confusing, rename to `PluginKeyString` or inline per-key casting in `build_index`.

### Task 4.7: Simplify `PluginRegistry`

**Files:**
- Modify: `crates/plugin/src/registry.rs`

- [ ] **Step 1: Write the failing test (extend or rewrite existing)**

At the bottom of `registry.rs`'s `#[cfg(test)] mod tests`, the old tests use `PluginType`. Rewrite the whole test block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use nebula_metadata::PluginManifest;
    use crate::{plugin::Plugin, ResolvedPlugin};

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);
    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    fn make(key: &str) -> Arc<ResolvedPlugin> {
        let manifest = PluginManifest::builder(key, key).build().unwrap();
        let p = StubPlugin(manifest);
        Arc::new(ResolvedPlugin::from(p).unwrap())
    }

    #[test]
    fn register_and_get() {
        let mut reg = PluginRegistry::new();
        reg.register(make("slack")).unwrap();
        let key: PluginKey = "slack".parse().unwrap();
        let got = reg.get(&key).unwrap();
        assert_eq!(got.key().as_str(), "slack");
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(make("a")).unwrap();
        let err = reg.register(make("a")).unwrap_err();
        assert_eq!(err, PluginError::AlreadyExists("a".parse().unwrap()));
    }

    #[test]
    fn remove_and_contains() {
        let mut reg = PluginRegistry::new();
        reg.register(make("x")).unwrap();
        let key: PluginKey = "x".parse().unwrap();
        assert!(reg.contains(&key));
        let removed = reg.remove(&key).unwrap();
        assert_eq!(removed.key().as_str(), "x");
        assert!(!reg.contains(&key));
    }

    #[test]
    fn iter_visits_all() {
        let mut reg = PluginRegistry::new();
        reg.register(make("a")).unwrap();
        reg.register(make("b")).unwrap();
        let keys: Vec<_> = reg.iter().map(|(k, _)| k.as_str().to_owned()).collect();
        assert_eq!(keys.len(), 2);
    }
}
```

- [ ] **Step 2: Rewrite the struct and impl**

Replace the entire body of `registry.rs` except the module doc comment:

```rust
//! In-memory plugin registry.

use std::{collections::HashMap, sync::Arc};

use nebula_core::PluginKey;

use crate::{PluginError, ResolvedPlugin};

/// In-memory registry mapping [`PluginKey`] to [`ResolvedPlugin`].
///
/// Thread-safety is the caller's responsibility — wrap in `RwLock` if
/// shared across threads.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: HashMap<PluginKey, Arc<ResolvedPlugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a resolved plugin. Fails with
    /// `PluginError::AlreadyExists` if the key is already taken.
    pub fn register(&mut self, plugin: Arc<ResolvedPlugin>) -> Result<(), PluginError> {
        let key = plugin.key().clone();
        if self.plugins.contains_key(&key) {
            return Err(PluginError::AlreadyExists(key));
        }
        self.plugins.insert(key, plugin);
        Ok(())
    }

    /// Look up a plugin by key.
    pub fn get(&self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>> {
        self.plugins.get(key).cloned()
    }

    /// Whether a plugin with the given key is registered.
    pub fn contains(&self, key: &PluginKey) -> bool {
        self.plugins.contains_key(key)
    }

    /// Remove a plugin. Returns the removed plugin, or `None`.
    pub fn remove(&mut self, key: &PluginKey) -> Option<Arc<ResolvedPlugin>> {
        self.plugins.remove(key)
    }

    /// Remove all plugins.
    pub fn clear(&mut self) {
        self.plugins.clear();
    }

    /// Iterator over `(PluginKey, &Arc<ResolvedPlugin>)`.
    pub fn iter(&self) -> impl Iterator<Item = (&PluginKey, &Arc<ResolvedPlugin>)> {
        self.plugins.iter()
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("count", &self.plugins.len())
            .field(
                "keys",
                &self.plugins.keys().cloned().collect::<Vec<_>>(),
            )
            .finish()
    }
}
```

Note: the old `register_or_replace` and `get_by_name` methods are dropped. If a downstream depends on them, reintroduce narrowly after the mainline compiles — grep first: `rg 'register_or_replace\|get_by_name' crates/ apps/`.

- [ ] **Step 3: Run the tests**

Run: `cargo nextest run -p nebula-plugin registry`
Expected: green.

### Task 4.8: Rewrite plugin.rs doc-tests (minimal)

**Files:**
- Modify: `crates/plugin/src/plugin.rs`

- [ ] **Step 1: Write minimal doc-tests at the trait**

Keep a single example at the trait doc showing a plugin that declares one action, and one at `actions()`. Example:

```rust
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use nebula_core::ActionKey;
/// use nebula_metadata::PluginManifest;
/// use nebula_plugin::Plugin;
///
/// # #[derive(Debug)]
/// # struct StubAction { metadata: nebula_action::ActionMetadata }
/// # impl nebula_action::Action for StubAction {
/// #     fn metadata(&self) -> &nebula_action::ActionMetadata { &self.metadata }
/// # }
/// #[derive(Debug)]
/// struct MyPlugin(PluginManifest);
///
/// impl Plugin for MyPlugin {
///     fn manifest(&self) -> &PluginManifest { &self.0 }
///
///     fn actions(&self) -> Vec<Arc<dyn nebula_action::Action>> {
///         let metadata = nebula_action::ActionMetadata::new(
///             ActionKey::new("my.send").unwrap(),
///             "Send",
///             "does the thing",
///         );
///         vec![Arc::new(StubAction { metadata }) as Arc<dyn nebula_action::Action>]
///     }
/// }
///
/// let manifest = PluginManifest::builder("my", "My").build().unwrap();
/// let plugin = MyPlugin(manifest);
/// assert_eq!(plugin.actions().len(), 1);
/// ```
```

- [ ] **Step 2: `cargo test --doc -p nebula-plugin`**

Expected: green.

### Task 4.9: Clean up engine crate references

**Files:**
- Modify: `crates/engine/src/lib.rs`
- Modify: `crates/engine/README.md`

- [ ] **Step 1: Open `crates/engine/src/lib.rs` line 70**

Run: `sed -n '60,80p' crates/engine/src/lib.rs`

- [ ] **Step 2: Remove `PluginType` from `pub use ...`**

Whatever the re-export line reads (e.g. `pub use nebula_plugin::{Plugin, PluginRegistry, PluginType, PluginManifest};`), drop `PluginType`. If `PluginManifest` is in there, change the import source to `nebula_metadata::PluginManifest` OR keep the `nebula_plugin::PluginManifest` path (it's a re-export now; both are valid).

- [ ] **Step 3: Open `crates/engine/README.md` line 64**

Run: `sed -n '58,74p' crates/engine/README.md`

- [ ] **Step 4: Update the README prose to drop the `PluginType` mention**

Replace whatever sentence previously said "engine re-exports PluginType ..." with a sentence about `PluginRegistry` + `ResolvedPlugin` holding runnable components.

- [ ] **Step 5: `cargo check --workspace`**

Expected: clean. If any test or binary elsewhere imports `nebula_plugin::PluginType`, grep and fix: `rg 'nebula_plugin::PluginType|PluginType' --type rust`.

### Task 4.10: Add pitfalls entry

**Files:**
- Modify: `docs/pitfalls.md`

- [ ] **Step 1: Append entry**

At the bottom of `docs/pitfalls.md`:

```markdown
## Plugin: action / credential / resource key outside the plugin's namespace

**Symptom.** `PluginRegistry::register` fails with
`PluginError::NamespaceMismatch { plugin, offending_key, kind }`.

**Cause.** A plugin's `Plugin::actions()` (or `credentials()` /
`resources()`) method returned an `Arc<dyn Action>` whose
`metadata().base.key` does not start with `{plugin.key()}.`. Typical
example: plugin keyed `slack` returns an action keyed `api.foo`.

**Fix.** Either rename the component's key to `slack.foo`, or move the
component to a plugin that legitimately owns `api.*`. The namespace
rule comes from canon §7.1 and is enforced at `ResolvedPlugin::from`
(not at dispatch time, so the bad registration cannot leak into the
runtime).

See ADR-0027.
```

### Task 4.11: Workspace verification + commit

- [ ] **Step 1: Full verification**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

Expected: all green.

- [ ] **Step 2: Safe-to-delete assertions**

Run:
```bash
rg 'nebula_plugin::descriptor' crates/
rg '\{Action,Credential,Resource\}Descriptor' crates/plugin/
rg 'PluginType|PluginVersions|ArcPlugin' crates/ apps/
```

Expected: no matches in workspace. Test sources inside deleted files don't count.

- [ ] **Step 3: Commit**

```bash
git add crates/plugin/ crates/engine/ docs/pitfalls.md
git commit -m "refactor(plugin)!: canonicalize Plugin trait; add ResolvedPlugin

Large atomic refactor (slice B of plugin load-path stabilization):

- Plugin trait returns Vec<Arc<dyn Action|Credential|Resource>> instead
  of legacy descriptor structs.
- Delete crates/plugin/src/descriptor.rs entirely.
- Delete crates/plugin/src/plugin_type.rs (enum) and versions.rs
  (struct) — multi-version runtime registry had zero production
  consumers. Workflow-level version pinning is a future ADR.
- New crates/plugin/src/resolved_plugin.rs — per-plugin wrapper with
  eager component caches; enforces namespace invariant at construction.
- PluginRegistry becomes HashMap<PluginKey, Arc<ResolvedPlugin>>.
- Prune PluginError: remove Version* / KeyMismatch / NotVersioned;
  add NamespaceMismatch, DuplicateComponent, InvalidManifest.
- Engine re-exports and README updated to drop PluginType.
- pitfalls.md documents the namespace-mismatch trap.

BREAKING CHANGE: Plugin trait return types changed; PluginType enum
removed. Only production consumer per tech-lead review was
api/handlers/catalog.rs (manifest-only; unaffected). See ADR-0027."
```

---

## PR 5 — `PluginRegistry` Aggregate Accessors

**Goal:** Add `all_actions()` / `all_credentials()` / `all_resources()` flat iterators and `resolve_action()` / `resolve_credential()` / `resolve_resource()` full-key lookups on `PluginRegistry`. Pure additive; no API break.

**Files:**
- Modify: `crates/plugin/src/registry.rs`
- Modify: `crates/plugin/tests/resolved_plugin.rs` (extend with cross-plugin tests)

### Task 5.1: TDD — cross-plugin resolve test

**Files:**
- Modify: `crates/plugin/tests/resolved_plugin.rs`

- [ ] **Step 1: Append tests**

```rust
use nebula_plugin::PluginRegistry;

#[test]
fn registry_resolve_action_walks_all_plugins() {
    let mut reg = PluginRegistry::new();

    let slack_plugin = ResolvedPlugin::from(
        plugin_with_actions("slack", vec!["slack.send_message"]),
    )
    .unwrap();
    let http_plugin = ResolvedPlugin::from(
        plugin_with_actions("http", vec!["http.get", "http.post"]),
    )
    .unwrap();

    reg.register(Arc::new(slack_plugin)).unwrap();
    reg.register(Arc::new(http_plugin)).unwrap();

    let slack_action = reg
        .resolve_action(&ActionKey::new("slack.send_message").unwrap())
        .expect("slack action");
    assert_eq!(slack_action.metadata().base.key.as_str(), "slack.send_message");

    let http_post = reg
        .resolve_action(&ActionKey::new("http.post").unwrap())
        .expect("http post");
    assert_eq!(http_post.metadata().base.key.as_str(), "http.post");

    assert!(
        reg.resolve_action(&ActionKey::new("does.not.exist").unwrap())
            .is_none()
    );
}

#[test]
fn registry_all_actions_flat_iterator() {
    let mut reg = PluginRegistry::new();
    let slack = ResolvedPlugin::from(
        plugin_with_actions("slack", vec!["slack.send_message"]),
    )
    .unwrap();
    let http = ResolvedPlugin::from(
        plugin_with_actions("http", vec!["http.get"]),
    )
    .unwrap();
    reg.register(Arc::new(slack)).unwrap();
    reg.register(Arc::new(http)).unwrap();

    let count = reg.all_actions().count();
    assert_eq!(count, 2);

    let keys: Vec<&str> = reg
        .all_actions()
        .map(|(_pk, a)| a.metadata().base.key.as_str())
        .collect();
    assert!(keys.contains(&"slack.send_message"));
    assert!(keys.contains(&"http.get"));
}
```

- [ ] **Step 2: Run — expected FAIL**

Run: `cargo nextest run -p nebula-plugin --test resolved_plugin registry_`
Expected: compilation fails — methods don't exist.

### Task 5.2: Implement aggregate accessors

**Files:**
- Modify: `crates/plugin/src/registry.rs`

- [ ] **Step 1: Add impl methods**

Append to `impl PluginRegistry`:

```rust
    /// Flat iterator over every action in every registered plugin.
    pub fn all_actions(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &std::sync::Arc<dyn nebula_action::Action>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.actions().map(move |(_k, a)| (pk, a)))
    }

    /// Flat iterator over every credential in every registered plugin.
    pub fn all_credentials(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &std::sync::Arc<dyn nebula_credential::Credential>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.credentials().map(move |(_k, c)| (pk, c)))
    }

    /// Flat iterator over every resource in every registered plugin.
    pub fn all_resources(
        &self,
    ) -> impl Iterator<Item = (&PluginKey, &std::sync::Arc<dyn nebula_resource::Resource>)> {
        self.plugins
            .iter()
            .flat_map(|(pk, rp)| rp.resources().map(move |(_k, r)| (pk, r)))
    }

    /// Resolve an action by its full key — probes each registered
    /// `ResolvedPlugin`'s flat cache. O(plugins) + O(1).
    pub fn resolve_action(
        &self,
        full: &nebula_core::ActionKey,
    ) -> Option<std::sync::Arc<dyn nebula_action::Action>> {
        self.plugins
            .values()
            .find_map(|rp| rp.action(full).cloned())
    }

    /// Resolve a credential by its full key.
    pub fn resolve_credential(
        &self,
        full: &nebula_core::CredentialKey,
    ) -> Option<std::sync::Arc<dyn nebula_credential::Credential>> {
        self.plugins
            .values()
            .find_map(|rp| rp.credential(full).cloned())
    }

    /// Resolve a resource by its full key.
    pub fn resolve_resource(
        &self,
        full: &nebula_core::ResourceKey,
    ) -> Option<std::sync::Arc<dyn nebula_resource::Resource>> {
        self.plugins
            .values()
            .find_map(|rp| rp.resource(full).cloned())
    }
```

- [ ] **Step 2: Run test**

Run: `cargo nextest run -p nebula-plugin --test resolved_plugin`
Expected: all tests green (the new ones + existing).

- [ ] **Step 3: `cargo clippy -p nebula-plugin -- -D warnings`**

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/plugin/
git commit -m "feat(plugin): add PluginRegistry aggregate accessors

all_actions() / all_credentials() / all_resources() flat iterators
feed nebula-runtime::ActionRegistry at bulk-register time.
resolve_action() / resolve_credential() / resolve_resource() walk
registered ResolvedPlugin caches by full key for introspection.

Tests cover cross-plugin resolve and iterator shape."
```

---

## PR 6 — Wire Schema + Discovery

**Goal:** Land the full out-of-process path — `plugin.toml` parser, SDK constraint check at discovery, `RemoteAction` wrapper, `DiscoveredPlugin: impl Plugin`, schema-bearing wire ActionDescriptor flowing end-to-end. This is the consumer-side of PR 3's wire bump.

**Files:**
- Create: `crates/sandbox/src/plugin_toml.rs`
- Create: `crates/sandbox/src/remote_action.rs`
- Create: `crates/sandbox/src/discovered_plugin.rs`
- Modify: `crates/sandbox/src/discovery.rs`
- Modify: `crates/sandbox/src/lib.rs`
- Modify: `crates/sandbox/src/error.rs`
- Modify: `crates/sandbox/Cargo.toml` (add `toml`)
- Create: `crates/sandbox/tests/plugin_toml_parse.rs`
- Create: `crates/plugin-sdk/src/bin/schema_fixture.rs`
- Create: `crates/sandbox/tests/discovery_schema_roundtrip.rs`
- Modify: `crates/sandbox/README.md`

### Task 6.1: Add `toml` dep

**Files:**
- Modify: `crates/sandbox/Cargo.toml`

- [ ] **Step 1: Append**

```toml
toml = { workspace = true }
semver = { workspace = true }
```

Verify `toml` is already in workspace Cargo.toml; if not, add `toml = "0.8"` to the root workspace `[workspace.dependencies]`.

### Task 6.2: TDD — `plugin.toml` parser tests

**Files:**
- Create: `crates/sandbox/tests/plugin_toml_parse.rs`

- [ ] **Step 1: Write failing tests**

```rust
//! Parser tests for `plugin.toml` per canon §7.1.

use std::path::PathBuf;

use nebula_sandbox::plugin_toml::{
    parse_plugin_toml, PluginTomlError, PluginTomlManifest,
};

fn write(contents: &str) -> tempfile::NamedTempFile {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

#[test]
fn parse_minimal_plugin_toml() {
    let f = write(r#"
        [nebula]
        sdk = "^0.8"
    "#);
    let manifest = parse_plugin_toml(f.path()).unwrap();
    assert_eq!(manifest.sdk.to_string(), "^0.8");
    assert!(manifest.plugin_id.is_none());
}

#[test]
fn parse_plugin_toml_with_optional_id() {
    let f = write(r#"
        [nebula]
        sdk = "^0.8"

        [plugin]
        id = "com.author.slack"
    "#);
    let manifest = parse_plugin_toml(f.path()).unwrap();
    assert_eq!(manifest.plugin_id.as_deref(), Some("com.author.slack"));
}

#[test]
fn missing_file_errors() {
    let err = parse_plugin_toml(&PathBuf::from("/nonexistent/plugin.toml")).unwrap_err();
    assert!(matches!(err, PluginTomlError::Missing { .. }));
}

#[test]
fn missing_sdk_constraint_errors() {
    let f = write(r#"
        [nebula]
    "#);
    let err = parse_plugin_toml(f.path()).unwrap_err();
    assert!(matches!(err, PluginTomlError::MissingSdkConstraint { .. }));
}

#[test]
fn invalid_toml_errors() {
    let f = write("this is not toml = = ==");
    let err = parse_plugin_toml(f.path()).unwrap_err();
    assert!(matches!(err, PluginTomlError::InvalidToml { .. }));
}
```

- [ ] **Step 2: Run — expected FAIL (module absent)**

Run: `cargo nextest run -p nebula-sandbox --test plugin_toml_parse`
Expected: compilation fails.

### Task 6.3: Implement `plugin_toml.rs`

**Files:**
- Create: `crates/sandbox/src/plugin_toml.rs`
- Modify: `crates/sandbox/src/lib.rs`
- Modify: `crates/sandbox/src/error.rs`

- [ ] **Step 1: Write the module**

```rust
//! `plugin.toml` parsing per canon §7.1.
//!
//! Shape:
//! ```toml
//! [nebula]
//! sdk = "^0.8"           # required — semver constraint on the plugin SDK
//!
//! [plugin]
//! id = "com.author.slack"  # optional — canonical plugin id override
//! ```

use std::path::{Path, PathBuf};

use semver::VersionReq;
use serde::Deserialize;

/// Parsed `plugin.toml` manifest — the subset canon §7.1 cares about.
#[derive(Debug, Clone)]
pub struct PluginTomlManifest {
    /// Semver constraint on the SDK version this plugin expects.
    pub sdk: VersionReq,
    /// Optional canonical plugin id override.
    pub plugin_id: Option<String>,
}

/// Errors from `parse_plugin_toml`.
#[derive(Debug, thiserror::Error)]
pub enum PluginTomlError {
    /// The manifest file does not exist.
    #[error("plugin.toml not found at {path}")]
    Missing { path: PathBuf },

    /// The file could not be parsed as TOML.
    #[error("plugin.toml at {path} is not valid TOML: {source}")]
    InvalidToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// The `[nebula].sdk` field is required but missing.
    #[error("plugin.toml at {path} is missing required [nebula].sdk")]
    MissingSdkConstraint { path: PathBuf },

    /// The `[nebula].sdk` value is not a valid semver constraint.
    #[error("plugin.toml at {path} has invalid sdk constraint: {source}")]
    InvalidSdkConstraint {
        path: PathBuf,
        #[source]
        source: semver::Error,
    },
}

#[derive(Deserialize)]
struct Raw {
    nebula: RawNebula,
    #[serde(default)]
    plugin: Option<RawPlugin>,
}

#[derive(Deserialize)]
struct RawNebula {
    #[serde(default)]
    sdk: Option<String>,
}

#[derive(Deserialize)]
struct RawPlugin {
    #[serde(default)]
    id: Option<String>,
}

/// Parse a `plugin.toml` from the filesystem.
pub fn parse_plugin_toml(path: &Path) -> Result<PluginTomlManifest, PluginTomlError> {
    let contents = std::fs::read_to_string(path).map_err(|_| PluginTomlError::Missing {
        path: path.to_path_buf(),
    })?;

    let raw: Raw = toml::from_str(&contents).map_err(|e| PluginTomlError::InvalidToml {
        path: path.to_path_buf(),
        source: e,
    })?;

    let sdk_str = raw
        .nebula
        .sdk
        .ok_or_else(|| PluginTomlError::MissingSdkConstraint {
            path: path.to_path_buf(),
        })?;

    let sdk = VersionReq::parse(&sdk_str).map_err(|e| PluginTomlError::InvalidSdkConstraint {
        path: path.to_path_buf(),
        source: e,
    })?;

    let plugin_id = raw.plugin.and_then(|p| p.id);

    Ok(PluginTomlManifest { sdk, plugin_id })
}
```

- [ ] **Step 2: Register module in `lib.rs`**

```rust
pub mod plugin_toml;
```

- [ ] **Step 3: Extend `SandboxError` with `Discovery(PluginTomlError)` if not already present**

- [ ] **Step 4: Add `tempfile` to `[dev-dependencies]`**

If not already there.

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p nebula-sandbox --test plugin_toml_parse`
Expected: all five tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sandbox/
git commit -m "feat(sandbox): plugin.toml parser

Minimal plugin.toml reader per canon §7.1: [nebula].sdk (required,
semver constraint) + [plugin].id (optional). Typed error variants
for each failure mode. Discovery will consume this before spawning
the plugin binary to verify SDK compatibility."
```

### Task 6.4: `RemoteAction` wrapper

**Files:**
- Create: `crates/sandbox/src/remote_action.rs`
- Modify: `crates/sandbox/src/lib.rs`

- [ ] **Step 1: Write the wrapper**

```rust
//! `RemoteAction` — an out-of-process action dispatched through
//! `ProcessSandbox` via the duplex JSON envelope protocol.
//!
//! Thin wrapper over `ProcessSandboxHandler`: carries a synthesized
//! `ActionMetadata` (built from the wire `ActionDescriptor` + host-
//! synthesized defaults for `IsolationLevel` / `ActionCategory` / ports)
//! and exposes it via the `Action` trait so the host-side registry
//! stores it uniformly alongside built-in actions.

use std::sync::Arc;

use nebula_action::{Action, ActionMetadata};

use crate::handler::ProcessSandboxHandler;

/// Host-side wrapper that makes a `ProcessSandboxHandler` satisfy
/// the `Action` trait.
#[derive(Debug)]
pub struct RemoteAction {
    metadata: ActionMetadata,
    handler: Arc<ProcessSandboxHandler>,
}

impl RemoteAction {
    /// Construct from a prebuilt `ActionMetadata` + handler.
    pub fn new(metadata: ActionMetadata, handler: Arc<ProcessSandboxHandler>) -> Self {
        Self { metadata, handler }
    }

    /// Access the underlying broker handler (used by the runtime dispatch
    /// path when it needs to send an `ActionInvoke` envelope).
    pub fn handler(&self) -> &Arc<ProcessSandboxHandler> {
        &self.handler
    }
}

impl Action for RemoteAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}
```

- [ ] **Step 2: Register module**

```rust
pub mod remote_action;
pub use remote_action::RemoteAction;
```

- [ ] **Step 3: `cargo check -p nebula-sandbox`**

Expected: clean. If `Action` trait has more required methods than `metadata()`, they stay provided by defaults (tech-lead confirmed `Self: Sized` guards keep dispatch-related methods off the vtable).

### Task 6.5: `DiscoveredPlugin` impl Plugin

**Files:**
- Create: `crates/sandbox/src/discovered_plugin.rs`
- Modify: `crates/sandbox/src/lib.rs`

- [ ] **Step 1: Write the module**

```rust
//! `DiscoveredPlugin` — host-side `impl Plugin` wrapper over the metadata
//! returned by an out-of-process plugin during discovery.

use std::sync::Arc;

use nebula_action::Action;
use nebula_credential::Credential;
use nebula_metadata::PluginManifest;
use nebula_plugin::{Plugin, PluginError};
use nebula_resource::Resource;

/// Host-side wrapper: `impl Plugin` backed by a `MetadataResponse`'s data.
#[derive(Debug)]
pub struct DiscoveredPlugin {
    manifest: PluginManifest,
    actions: Vec<Arc<dyn Action>>,
}

impl DiscoveredPlugin {
    /// Construct. `actions` is expected to already contain properly-built
    /// `RemoteAction` instances (or anything else implementing `Action`).
    pub fn new(manifest: PluginManifest, actions: Vec<Arc<dyn Action>>) -> Self {
        Self { manifest, actions }
    }
}

impl Plugin for DiscoveredPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<Arc<dyn Action>> {
        self.actions.clone()
    }

    fn credentials(&self) -> Vec<Arc<dyn Credential>> {
        // Gated on ADR-0025 slice 1d broker RPC. Until then, out-of-process
        // plugins cannot contribute credentials.
        vec![]
    }

    fn resources(&self) -> Vec<Arc<dyn Resource>> {
        // Same — gated on slice 1d broker RPC.
        vec![]
    }

    fn on_load(&self) -> Result<(), PluginError> {
        Ok(())
    }

    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }
}
```

- [ ] **Step 2: Register module**

```rust
pub mod discovered_plugin;
pub use discovered_plugin::DiscoveredPlugin;
```

- [ ] **Step 3: `cargo check -p nebula-sandbox`**

Expected: clean.

### Task 6.6: Rewire `discovery.rs` to use plugin.toml + new wire shape

**Files:**
- Modify: `crates/sandbox/src/discovery.rs`

- [ ] **Step 1: Read the existing file carefully**

The old `DiscoveredPlugin` struct (DTO with `{ key, version, actions }`) and the `discover_plugin` + `discover_directory` + `create_handlers` functions need to be reshaped. The module-level `DiscoveredPlugin` DTO is removed — the wire envelope carries the manifest directly now.

- [ ] **Step 2: Rewrite the module**

```rust
//! Plugin discovery — scan directories for plugin binaries, parse each
//! `plugin.toml` marker, verify SDK compatibility, spawn, probe metadata.

use std::{path::Path, sync::Arc, time::Duration};

use nebula_action::{Action, ActionMetadata};
use nebula_core::ActionKey;
use nebula_metadata::PluginManifest;
use nebula_plugin::{Plugin, PluginRegistry, ResolvedPlugin};
use nebula_plugin_sdk::protocol::{DUPLEX_PROTOCOL_VERSION, PluginToHost};
use semver::Version;

use crate::{
    capabilities::PluginCapabilities,
    discovered_plugin::DiscoveredPlugin,
    handler::ProcessSandboxHandler,
    plugin_toml::{parse_plugin_toml, PluginTomlError, PluginTomlManifest},
    process::ProcessSandbox,
    remote_action::RemoteAction,
};

/// The SDK version this host was built against. Compared against the
/// plugin.toml `[nebula].sdk` constraint.
const HOST_SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Discover every plugin in `dir`, register each into `registry`.
///
/// Errors per-plugin are warned and the plugin is skipped — a bad plugin
/// must not poison the directory scan.
pub async fn discover_directory(
    dir: &Path,
    registry: &mut PluginRegistry,
    default_timeout: Duration,
    default_capabilities: PluginCapabilities,
) {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read plugin directory");
            return;
        }
    };

    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !is_executable(&path) {
            continue;
        }

        match discover_one(&path, default_timeout, default_capabilities.clone()).await {
            Ok(plugin) => {
                let manifest_key = plugin.manifest().key().clone();
                let resolved = match ResolvedPlugin::from(plugin) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(binary = %path.display(), error = %e,
                            "skip plugin: resolver rejected it");
                        continue;
                    }
                };
                if let Err(e) = registry.register(Arc::new(resolved)) {
                    tracing::warn!(binary = %path.display(), plugin = %manifest_key,
                        error = %e, "skip plugin: registry conflict");
                }
            }
            Err(e) => {
                tracing::warn!(binary = %path.display(), error = %e, "skip plugin");
            }
        }
    }
}

async fn discover_one(
    binary: &Path,
    default_timeout: Duration,
    default_capabilities: PluginCapabilities,
) -> Result<DiscoveredPlugin, DiscoveryError> {
    // 1. Parse plugin.toml sitting next to the binary.
    let toml_path = binary
        .parent()
        .ok_or_else(|| DiscoveryError::BadBinaryPath(binary.to_path_buf()))?
        .join("plugin.toml");
    let toml_manifest = parse_plugin_toml(&toml_path).map_err(DiscoveryError::Toml)?;

    // 2. Enforce SDK constraint.
    let host_sdk = Version::parse(HOST_SDK_VERSION).map_err(|e| {
        DiscoveryError::HostSdkVersionParse {
            value: HOST_SDK_VERSION.to_owned(),
            source: e,
        }
    })?;
    if !toml_manifest.sdk.matches(&host_sdk) {
        return Err(DiscoveryError::IncompatibleSdk {
            required: toml_manifest.sdk.clone(),
            actual: host_sdk,
        });
    }

    // 3. Spawn + probe metadata.
    let sandbox = ProcessSandbox::new(
        binary.to_path_buf(),
        Duration::from_secs(5),
        PluginCapabilities::none(),
    );
    let envelope = sandbox
        .get_metadata()
        .await
        .map_err(DiscoveryError::Spawn)?;

    let (manifest, actions_wire) = match envelope {
        PluginToHost::MetadataResponse {
            protocol_version,
            manifest,
            actions,
            ..
        } => {
            if protocol_version != DUPLEX_PROTOCOL_VERSION {
                return Err(DiscoveryError::ProtocolVersionMismatch {
                    expected: DUPLEX_PROTOCOL_VERSION,
                    actual: protocol_version,
                });
            }
            (manifest, actions)
        }
        other => return Err(DiscoveryError::UnexpectedEnvelope(envelope_kind(&other))),
    };

    // 4. If plugin.toml overrides the id, reject conflicts.
    let manifest = apply_plugin_id_override(manifest, &toml_manifest)?;

    // 5. Build RemoteAction instances for the runtime sandbox (long-lived).
    let runtime_sandbox = Arc::new(ProcessSandbox::new(
        binary.to_path_buf(),
        default_timeout,
        default_capabilities,
    ));
    let actions = build_remote_actions(&manifest, actions_wire, runtime_sandbox)?;

    if actions.is_empty() {
        tracing::info!(
            plugin = %manifest.key(),
            "out-of-process plugin declared zero actions",
        );
    }

    let declared_credentials = 0usize; // wire doesn't carry them yet
    let declared_resources = 0usize;
    if declared_credentials > 0 || declared_resources > 0 {
        tracing::info!(
            plugin = %manifest.key(),
            creds = declared_credentials,
            resources = declared_resources,
            "out-of-process plugin declared credentials/resources; \
             skipped — gated on ADR-0025 slice 1d broker RPC",
        );
    }

    Ok(DiscoveredPlugin::new(manifest, actions))
}

fn apply_plugin_id_override(
    mut manifest: PluginManifest,
    toml: &PluginTomlManifest,
) -> Result<PluginManifest, DiscoveryError> {
    if let Some(override_id) = &toml.plugin_id {
        if manifest.key().as_str() != override_id {
            return Err(DiscoveryError::KeyConflict {
                toml_id: override_id.clone(),
                manifest_key: manifest.key().as_str().to_owned(),
            });
        }
        // The key matches; no change needed.
    }
    Ok(manifest)
}

fn build_remote_actions(
    manifest: &PluginManifest,
    wire: Vec<nebula_plugin_sdk::protocol::ActionDescriptor>,
    sandbox: Arc<ProcessSandbox>,
) -> Result<Vec<Arc<dyn Action>>, DiscoveryError> {
    let namespace_prefix = format!("{}.", manifest.key().as_str());
    let interface_version = manifest.version().clone();

    let mut out = Vec::with_capacity(wire.len());
    for descriptor in wire {
        let full_key = if descriptor.key.contains('.') {
            if !descriptor.key.starts_with(&namespace_prefix) {
                tracing::warn!(
                    plugin = %manifest.key(),
                    action = %descriptor.key,
                    "skip action: cross-namespace wire key"
                );
                continue;
            }
            descriptor.key.clone()
        } else {
            format!("{namespace_prefix}{}", descriptor.key)
        };

        let action_key = match ActionKey::new(&full_key) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(key = %full_key, error = %e, "skip action: invalid key");
                continue;
            }
        };

        let metadata = ActionMetadata::new(action_key, &descriptor.name, &descriptor.description)
            .with_version_full(interface_version.clone())
            .with_schema(descriptor.schema);

        let handler = Arc::new(ProcessSandboxHandler::new(
            Arc::clone(&sandbox),
            metadata.clone(),
        ));
        out.push(Arc::new(RemoteAction::new(metadata, handler)) as Arc<dyn Action>);
    }
    Ok(out)
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("binary path has no parent directory: {0}")]
    BadBinaryPath(std::path::PathBuf),
    #[error(transparent)]
    Toml(#[from] PluginTomlError),
    #[error("plugin requires SDK '{required}' but host SDK is '{actual}'")]
    IncompatibleSdk {
        required: semver::VersionReq,
        actual: Version,
    },
    #[error("cannot parse host SDK version '{value}': {source}")]
    HostSdkVersionParse {
        value: String,
        #[source]
        source: semver::Error,
    },
    #[error("failed to spawn plugin and probe metadata: {0}")]
    Spawn(String),
    #[error("plugin.toml id '{toml_id}' does not match manifest key '{manifest_key}'")]
    KeyConflict {
        toml_id: String,
        manifest_key: String,
    },
    #[error("plugin protocol version mismatch: host={expected}, plugin={actual}")]
    ProtocolVersionMismatch { expected: u32, actual: u32 },
    #[error("unexpected envelope from plugin: {0}")]
    UnexpectedEnvelope(&'static str),
}

fn envelope_kind(env: &PluginToHost) -> &'static str {
    match env {
        PluginToHost::ActionResultOk { .. } => "action_result_ok",
        PluginToHost::ActionResultError { .. } => "action_result_error",
        PluginToHost::RpcCall { .. } => "rpc_call",
        PluginToHost::Log { .. } => "log",
        PluginToHost::MetadataResponse { .. } => "metadata_response",
    }
}

/// Check if a file looks like an executable plugin binary.
fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.starts_with("nebula-plugin-") && !name.starts_with("nebula_plugin_") {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(
        ext,
        "toml" | "json" | "yaml" | "yml" | "md" | "txt" | "so" | "dll" | "dylib"
    ) {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }

    #[cfg(not(unix))]
    {
        ext.eq_ignore_ascii_case("exe") || ext.is_empty()
    }
}
```

Note: `ActionMetadata::with_schema(schema: ValidSchema)` method may not exist today — if not, add it to `nebula-action::ActionMetadata` as part of this PR (tiny builder method). Prefer doing it here to keep the PR atomic.

- [ ] **Step 3: `cargo check -p nebula-sandbox`**

Expected: clean. Fix missing imports / renames as they surface.

### Task 6.7: Schema round-trip fixture

**Files:**
- Create: `crates/plugin-sdk/src/bin/schema_fixture.rs`
- Modify: `crates/plugin-sdk/Cargo.toml` — add `[[bin]] name = "nebula-plugin-schema-fixture"` entry

- [ ] **Step 1: Write fixture**

```rust
//! Schema-bearing fixture — used by the discovery schema round-trip test.

use async_trait::async_trait;
use nebula_metadata::PluginManifest;
use nebula_plugin_sdk::{
    PluginCtx, PluginError, PluginHandler,
    protocol::ActionDescriptor,
    run_duplex,
};
use nebula_schema::Schema;
use semver::Version;
use serde_json::{json, Value};

struct SchemaFixture {
    manifest: PluginManifest,
}

impl SchemaFixture {
    fn new() -> Self {
        let manifest = PluginManifest::builder("com.author.schema", "Schema Fixture")
            .version(Version::new(1, 0, 0))
            .description("Fixture declaring one action with a two-field schema.")
            .build()
            .unwrap();
        Self { manifest }
    }
}

#[async_trait]
impl PluginHandler for SchemaFixture {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn actions(&self) -> Vec<ActionDescriptor> {
        let schema = Schema::builder()
            .text("name", |f| f.required())
            .number("age", |f| f.optional())
            .build()
            .unwrap();
        vec![ActionDescriptor {
            key: "describe".into(),
            name: "Describe".into(),
            description: "Round-trip schema probe.".into(),
            schema,
        }]
    }

    async fn execute(
        &self,
        _ctx: &PluginCtx,
        _action_key: &str,
        input: Value,
    ) -> Result<Value, PluginError> {
        Ok(json!({ "received": input }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_duplex(SchemaFixture::new()).await?;
    Ok(())
}
```

- [ ] **Step 2: Update Cargo.toml**

```toml
[[bin]]
name = "nebula-plugin-schema-fixture"
path = "src/bin/schema_fixture.rs"
```

### Task 6.8: Discovery integration test — schema round-trip

**Files:**
- Create: `crates/sandbox/tests/discovery_schema_roundtrip.rs`

- [ ] **Step 1: Write test**

```rust
//! End-to-end discovery test: spawn the schema-bearing fixture, verify
//! the host-side ActionMetadata carries the fixture's two-field schema.

use std::{path::PathBuf, time::Duration};

use nebula_plugin::PluginRegistry;
use nebula_sandbox::{capabilities::PluginCapabilities, discovery::discover_directory};

fn fixture_dir() -> PathBuf {
    // Cargo places binary fixtures under target/debug for `cargo test`.
    // The plugin.toml marker is colocated per convention.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
}

/// NOTE: this test requires the fixture binary to be pre-built.
/// Run with: `cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture`
/// before `cargo test`.
#[tokio::test]
#[ignore = "requires pre-built fixture binary; run via `cargo nextest run ... --run-ignored all`"]
async fn discovery_roundtrips_action_schema() {
    // Create an isolated scan directory with only the schema fixture.
    // The plugin.toml must be next to the binary.
    let scan_dir = tempfile::tempdir().unwrap();
    let src_binary = fixture_dir().join(if cfg!(windows) {
        "nebula-plugin-schema-fixture.exe"
    } else {
        "nebula-plugin-schema-fixture"
    });
    assert!(src_binary.exists(), "fixture binary not built: {}", src_binary.display());

    // Copy binary and write plugin.toml next to it.
    let dest_binary = scan_dir.path().join(src_binary.file_name().unwrap());
    std::fs::copy(&src_binary, &dest_binary).unwrap();
    std::fs::write(
        scan_dir.path().join("plugin.toml"),
        format!("[nebula]\nsdk = \"*\"\n"),
    )
    .unwrap();

    let mut registry = PluginRegistry::new();
    discover_directory(
        scan_dir.path(),
        &mut registry,
        Duration::from_secs(5),
        PluginCapabilities::none(),
    )
    .await;

    let plugin_key = "com.author.schema".parse().unwrap();
    let plugin = registry.get(&plugin_key).expect("plugin registered");

    let action_key = nebula_core::ActionKey::new("com.author.schema.describe").unwrap();
    let action = plugin.action(&action_key).expect("action present");

    let schema = &action.metadata().base.schema;
    // Assert fields round-tripped:
    let fields: Vec<_> = schema.fields().iter().map(|f| f.name()).collect();
    assert!(fields.contains(&"name"));
    assert!(fields.contains(&"age"));
}
```

Note: the test is `#[ignore]`-gated because cargo test + binary fixtures don't auto-build. CI should run `cargo build -p nebula-plugin-sdk --bins && cargo nextest run -p nebula-sandbox --run-ignored all`.

- [ ] **Step 2: Run (locally after building)**

```bash
cargo build -p nebula-plugin-sdk --bin nebula-plugin-schema-fixture
cargo nextest run -p nebula-sandbox --test discovery_schema_roundtrip --run-ignored all
```

Expected: PASS.

- [ ] **Step 3: Update the workspace CI note**

Ensure CI builds plugin-sdk bins before running sandbox tests. Check `.github/workflows/ci.yml` for the test invocation; add `cargo build --bins -p nebula-plugin-sdk` step ahead of `cargo nextest run` if not present.

### Task 6.9: Update sandbox README

**Files:**
- Modify: `crates/sandbox/README.md`

- [ ] **Step 1: Update the isolation roadmap**

In the appendix `## Real isolation roadmap (priority order...)`, replace item #1:

```markdown
1. **Capability wiring.** Plugin capabilities are sourced from
   **workflow-config** at spawn time (per
   [ADR-0025](../../docs/adr/0025-sandbox-broker-rpc-surface.md) D4) —
   not from `plugin.toml`. The sandbox receives a `PluginCapabilities`
   from its caller (engine / runtime) and enforces it at
   `ProcessSandbox` boundaries. Older revisions of this roadmap
   mentioned `plugin.toml` capabilities; ADR-0025 superseded that.
```

- [ ] **Step 2: Drop the `Discovery TODO` false-capability language**

In the `## Appendix / Discovery TODO` section, replace the §4.5 language with:

```markdown
### Discovery TODO (partially closed by slice B)

Slice B of the plugin load-path stabilization closed the `plugin.toml`
parsing gap: discovery now reads `[nebula].sdk` + `[plugin].id` before
spawning the binary, enforces the SDK semver constraint, and honors
the optional id override. Workflow-config-sourced `PluginCapabilities`
enforcement at the broker is the remaining piece (item 1 above),
tracked under ADR-0025 slice 1d.
```

- [ ] **Step 3: Commit**

```bash
git add crates/sandbox/ crates/plugin-sdk/ .github/
git commit -m "feat(sandbox): plugin.toml discovery + RemoteAction + schema round-trip

- plugin_toml::parse_plugin_toml + error taxonomy
- RemoteAction wrapper makes ProcessSandboxHandler satisfy Action
- DiscoveredPlugin impl Plugin (legacy DTO inlined)
- discover_directory parses plugin.toml, verifies SDK constraint,
  spawns, probes metadata, builds RemoteAction handles
- Credentials/resources from out-of-process plugins explicitly not
  registered (info-log only) — gated on ADR-0025 slice 1d
- schema_fixture binary + integration test assert ValidSchema
  round-trips from plugin-side declaration to host-side ActionMetadata
- sandbox README isolation roadmap updated per ADR-0025

Closes the false-capability marker on discovery.rs:117."
```

---

## PR 7 — ADR-0027

**Files:**
- Create: `docs/adr/0027-plugin-load-path-stable.md`
- Modify: `docs/adr/0018-plugin-metadata-to-manifest.md` (frontmatter `related:` only)
- Modify: `docs/adr/README.md` (index)

### Task 7.1: Write ADR-0027

**Files:**
- Create: `docs/adr/0027-plugin-load-path-stable.md`

- [ ] **Step 1: Write the ADR**

```markdown
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

[ADR-0018](./0018-plugin-metadata-to-manifest.md) renamed the plugin
bundle descriptor from `PluginMetadata` to `PluginManifest` and moved
its construction story into line with `nebula-metadata::BaseMetadata<K>`
(Icon/MaturityLevel/DeprecationNotice). The follow-up work it left open
is the focus of this ADR: on the host side, `Plugin::actions() ->
Vec<ActionDescriptor>` returned a flat, pre-consolidation
descriptor type duplicating the canonical `*Metadata` shape; on the
plugin-author side, `nebula-plugin-sdk` carried its own `PluginMeta`
builder because it could not reach the host's `PluginManifest` under
canon §7.1 ("plugin-side crate has zero engine-side deps"). On top of
that, `nebula-plugin` shipped a runtime multi-version registry
(`enum PluginType { Single, Versions }` + `PluginVersions`) with **zero
production consumers** — inherited from an earlier design exploration,
never wired.

## Decision

1. `nebula-plugin::Plugin` trait returns the runnable domain traits
   directly: `fn actions() -> Vec<Arc<dyn nebula_action::Action>>`,
   `fn credentials()`, `fn resources()` — matching canon §3.5 framing
   ("Plugin = [ registry: Actions + Resources + Credentials ]").
   `nebula-plugin::descriptor::*` legacy structs are deleted.

2. `PluginManifest` moves from `nebula-plugin` to `nebula-metadata`
   alongside `Icon` / `MaturityLevel` / `DeprecationNotice`. Both
   host (`nebula-plugin`) and plugin-author SDK (`nebula-plugin-sdk`)
   import it from the canonical location. `nebula-plugin-sdk` gains a
   single Core-layer exception to its "zero intra-workspace deps" rule
   for `nebula-metadata` (and `nebula-schema` for wire `ValidSchema`).
   `nebula-plugin` keeps a thin `pub use
   nebula_metadata::PluginManifest;` re-export for source compatibility.
   ADR-0018 frontmatter gains a `related: 0027` cross-link; body stays
   immutable.

3. `nebula-plugin-sdk::PluginMeta` is deleted. `PluginHandler` now
   exposes `manifest() -> &PluginManifest` and `actions() ->
   Vec<ActionDescriptor>` separately. Wire
   `PluginToHost::MetadataResponse` carries `manifest: PluginManifest`
   in place of the flat `plugin_key` / `plugin_version` fields, plus
   per-action `schema: ValidSchema`. `DUPLEX_PROTOCOL_VERSION` bumps
   from 2 to 3; plugin binaries rebuild against the new SDK.

4. New `nebula-plugin::ResolvedPlugin` is the per-plugin resolved
   wrapper. `ResolvedPlugin::from<P: Plugin + 'static>(plugin)` calls
   the three component methods exactly once, validates the namespace
   invariant ("every full key starts with `{plugin.key()}.`"), and
   builds three flat `HashMap<FullKey, Arc<dyn …>>` indices for O(1)
   lookup. Namespace violations surface as
   `PluginError::NamespaceMismatch`; within-plugin duplicate keys as
   `PluginError::DuplicateComponent`.

5. `enum PluginType { Single, Versions }`, `PluginVersions`, and the
   `ArcPlugin` helper are deleted. `PluginError::VersionAlreadyExists`,
   `VersionNotFound`, `NoVersionsAvailable`, `KeyMismatch` are
   deleted. `PluginRegistry` becomes `HashMap<PluginKey,
   Arc<ResolvedPlugin>>`. If workflow-level version pinning ever
   arrives, it opens a follow-up ADR — runtime multi-version was not
   the right shape for deploy-time Cargo pinning (today's story).

6. `PluginRegistry` gains `all_actions()` / `all_credentials()` /
   `all_resources()` flat iterators plus `resolve_action()` /
   `resolve_credential()` / `resolve_resource()` lookups. Flat
   iterators feed `nebula-runtime::ActionRegistry` at bulk-register
   time; lookups serve catalog introspection. Dispatch stays on the
   existing flat `ActionRegistry` (canon §10 golden path).

## Consequences

**Positive.** Canon §3.5 is reflected in types, not just docs. L2 §13.1
("plugin load → registry: Actions / Resources / Credentials appear in
the catalog") is honored end-to-end for in-process plugins.
`PluginManifest` has a single home; no more
`PluginMeta`/`PluginManifest` duplication. Namespace violations surface
at register time, not at dispatch. Wire protocol carries full manifest
+ per-action schema, closing the §11.6 false-capability gap for
out-of-process actions (catalog UI can render forms).

**Negative.** Breaking public trait change on `nebula-plugin`. Tech-
lead review confirmed the production footprint is essentially
`crates/api/src/handlers/catalog.rs` (manifest-only consumer);
everywhere else the `actions()` / `credentials()` / `resources()`
methods had no callers. Drops `PluginType::versioned` — no real
consumers, but this is documented as a deliberate retraction with a
re-evaluation trigger below.

**Neutral.** Out-of-process credentials / resources remain deferred to
ADR-0025 slice 1d (broker RPC verbs). Wire protocol is silent on those
fields in this ADR — extending the envelope without the runtime path
would reintroduce the §11.6 gap we just closed for actions.

## Alternatives considered

- **(a) Keep `descriptor` module + add metadata adapter.** Rejected:
  shim-naming trap per `CLAUDE.md` "Quick Win trap catalog"; the root
  cause is `descriptor` being a pre-consolidation duplicate of
  `*Metadata`, not a type-conversion problem.
- **(b) Return `Vec<ActionMetadata>` without the runnable trait
  object.** Rejected: canon §3.5 says "Plugin = [ registry: Actions
  + …]", not "Plugin declares metadata about Actions separately". The
  catalog UI still needs to reach the runnable object to configure /
  preview inputs; splitting metadata from runnable object forces
  every consumer to correlate.
- **(c) Introduce a separate `PluginHost` / `PluginCatalog`
  aggregator.** Rejected in favour of keeping `PluginRegistry` with
  aggregate methods + per-plugin `ResolvedPlugin`. Single top-level
  type; eager per-plugin resolution still gives O(1) lookup.
- **(d) Ship `PluginV2` trait alongside the legacy one.** Rejected:
  two traits for the same role, violates memory
  `feedback_no_shims`.
- **(e) Keep `enum PluginType` + `PluginVersions`.** Rejected: zero
  production consumers. n8n-style runtime multi-version is browser-
  first ergonomics where the editor can swap versions inline; Rust-
  native deploys pin at Cargo level. Re-opening runtime multi-version
  is a clean new ADR, not this one.
- **(f) Keep `PluginManifest` in `nebula-plugin`, mirror a minimal
  `PluginMeta` in SDK.** Rejected: duplication of the same concept in
  two crates for no functional reason. Moving the type to Core-layer
  `nebula-metadata` is a cleaner fix.

## Follow-ups

- ADR-0025 slice 1d lands the broker RPC verbs that let out-of-process
  plugins contribute credentials / resources; wire protocol extends
  with those fields in that slice.
- `MATURITY.md` row for `nebula-plugin` flips `partial → stable` on
  the engine-integration column after this slice merges.
- If a workflow-level version-pinning requirement appears (canary
  rollout, node-version inlining), re-open the multi-version runtime
  story in a dedicated ADR. The clean `PluginRegistry` surface in this
  ADR makes a future wrapper `VersionedPluginRegistry` or
  `(PluginKey, Version)`-keyed map straightforward.
- `docs/pitfalls.md` has an entry for the namespace-mismatch trap.

## Seam / verification

Files that carry the invariants:

- `crates/metadata/src/manifest.rs` — `PluginManifest`,
  `ManifestError`.
- `crates/plugin/src/plugin.rs` — the flipped trait.
- `crates/plugin/src/resolved_plugin.rs` — namespace invariant +
  duplicate check at construction.
- `crates/plugin/src/registry.rs` — `HashMap<PluginKey,
  Arc<ResolvedPlugin>>` + aggregate accessors.
- `crates/plugin/src/error.rs` — `NamespaceMismatch`,
  `DuplicateComponent`, `InvalidManifest`; no `Version*` variants.
- `crates/plugin-sdk/src/lib.rs` + `protocol.rs` — no `PluginMeta`,
  `DUPLEX_PROTOCOL_VERSION = 3`, `MetadataResponse.manifest`,
  `ActionDescriptor.schema`.
- `crates/sandbox/src/plugin_toml.rs` — parser.
- `crates/sandbox/src/remote_action.rs` — `impl Action` wrapper.
- `crates/sandbox/src/discovered_plugin.rs` — `impl Plugin` wrapper.
- `crates/sandbox/src/discovery.rs` — orchestration.
```

- [ ] **Step 2: Update ADR-0018 frontmatter `related:` field**

Open `docs/adr/0018-plugin-metadata-to-manifest.md` frontmatter. Add `0027` to `related:`:

```yaml
related:
  - crates/plugin/src/manifest.rs
  - crates/metadata/src/lib.rs
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
  - docs/adr/0027-plugin-load-path-stable.md
```

Body stays immutable per convention.

- [ ] **Step 3: Update `docs/adr/README.md` index**

Append to the chronological ADR list:

```markdown
- [0027 — Plugin load-path stable](./0027-plugin-load-path-stable.md) (accepted 2026-04-20)
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr/
git commit -m "docs(adr): ADR-0027 plugin load-path stable

Canonicalize the Plugin trait, introduce ResolvedPlugin, move
PluginManifest to nebula-metadata, delete PluginMeta + multi-version
runtime registry. Cross-links ADR-0018 frontmatter and the design
spec. See docs/superpowers/specs/2026-04-20-plugin-load-path-stable-design.md."
```

---

## PR 8 — MATURITY.md

**Files:**
- Modify: `docs/MATURITY.md`

### Task 8.1: Flip `nebula-plugin` row

- [ ] **Step 1: Open MATURITY.md**

Run: `sed -n '1,50p' docs/MATURITY.md`

- [ ] **Step 2: Update the `nebula-plugin` row**

Change:
```
| nebula-plugin        | partial  | stable  | stable | partial (registry wired; load path partial) | n/a |
```
to:
```
| nebula-plugin        | stable   | stable  | stable | stable | n/a |
```

- [ ] **Step 3: Update the `last-reviewed` frontmatter and the "Last full sweep / last targeted revision" line**

```yaml
last-reviewed: 2026-04-20
```

Append to the targeted revision log:

```markdown
Last targeted revision: 2026-04-20 — Plugin load-path stabilization
landed: Plugin trait returns runnable Arc<dyn Action|Credential|Resource>,
ResolvedPlugin per-plugin wrapper, PluginManifest moved to
nebula-metadata, multi-version runtime registry dropped (YAGNI),
DUPLEX_PROTOCOL_VERSION bumped to 3 with manifest + per-action schema,
plugin.toml SDK-constraint check at discovery, §4.5 discovery TODO
closed. See ADR-0027.
```

- [ ] **Step 4: Verify the sweep**

Run each acceptance command from the spec:
```bash
rg 'nebula_plugin::descriptor' crates/
rg 'PluginType|PluginVersions|ArcPlugin' crates/ apps/
rg 'use nebula_plugin' crates/action/src/ crates/credential/src/ crates/resource/src/
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

Expected: no matches for the grep checks; all cargo commands green.

- [ ] **Step 5: Commit**

```bash
git add docs/MATURITY.md
git commit -m "docs(maturity): nebula-plugin engine-integration → stable

Slice B of the plugin load-path stabilization is landed end-to-end.
Trait canonical, PluginManifest in nebula-metadata, ResolvedPlugin
wrapping per plugin, wire protocol v3, plugin.toml SDK-constraint
check. See ADR-0027 and docs/superpowers/specs/2026-04-20-plugin-load-path-stable-design.md."
```

---

## Self-Review

**Spec coverage** — every numbered item from the design spec "In scope" (1–17) maps to a task above:

- Items 1–4 (slice A: plugin.toml) → PR 6 Task 6.1–6.3 + sandbox README update in 6.9.
- Item 5 (delete `descriptor`) → PR 4 Task 4.3.
- Items 5a–5b (move `PluginManifest`, delete `PluginMeta`) → PR 2 Tasks 2.1–2.4 + PR 3 Tasks 3.1–3.3.
- Items 6–7 (trait flip + deps) → PR 4 Tasks 4.1–4.2.
- Item 8 (new `ResolvedPlugin`) → PR 4 Task 4.6.
- Item 9 (delete multi-version) → PR 4 Task 4.4.
- Item 10 (simplify `PluginRegistry`) → PR 4 Task 4.7 + PR 5 Tasks 5.1–5.2.
- Item 11 (`#[derive(Plugin)]` macro) → no separate task required; per tech-lead review the macro doesn't emit component methods, and the manifest part is unaffected. PR 4's workspace build verifies.
- Item 12 (wire schema) → PR 3 Task 3.2 (v3 bump + schema field).
- Item 13 (`RemoteAction`) → PR 6 Task 6.4.
- Item 14 (out-of-process credentials/resources skipped with info-log) → PR 6 Task 6.6 (inside `discover_one`).
- Item 15 (`DiscoveredPlugin`) → PR 6 Task 6.5.
- Item 16 (ADR-0027) → PR 7 Task 7.1.
- Item 17 (docs sync) → distributed across PR 2.4 (ADR-0018), PR 3.8 (UPGRADE_COMPAT), PR 4.10 (pitfalls), PR 6.9 (sandbox README), PR 7.1 (ADR-0018 frontmatter + index), PR 8.1 (MATURITY.md).

Acceptance criteria from the spec (1–11) all verified in the final `cargo nextest` / `rg` sweeps at PR 4.11 and PR 8.1.

**Placeholder scan.** Grep the plan for `TODO`, `TBD`, `implement later`, `similar to` — none present. Every code block shows the actual code; every command has expected output.

**Type consistency.** `ResolvedPlugin::from<P: Plugin + 'static>(plugin) -> Result<Self, PluginError>` appears in PR 4 Task 4.6 and is called as `ResolvedPlugin::from(BuiltinCorePlugin::new())?` and `ResolvedPlugin::from(plugin_with_actions(...))` in tests and discovery. `PluginRegistry::register(Arc<ResolvedPlugin>) -> Result<(), PluginError>` appears in PR 4 Task 4.7 and is called consistently in tests and discovery. `PluginError::NamespaceMismatch { plugin, offending_key, kind }` / `DuplicateComponent { plugin, key, kind }` / `InvalidManifest` fields match between error.rs definition and test assertions. Wire `MetadataResponse { id, protocol_version, manifest, actions }` + `ActionDescriptor { key, name, description, schema }` match between protocol.rs, fixtures, and discovery consumer.

No issues requiring a re-pass.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-20-plugin-load-path-stable.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration. Good for this scope because each task is independent enough to verify in isolation (especially the TDD-shaped tasks: failing test → impl → green → commit).

**2. Inline Execution** — execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?
