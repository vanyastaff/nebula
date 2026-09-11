---
name: nebula-plugin
role: Plugin Distribution Unit (registry + manifest re-export; canon §7.1 — unit of registration, not size)
status: partial
last-reviewed: 2026-07-27
canon-invariants: [L1-7.1, L2-7.1, L2-13.1]
related: [nebula-core, nebula-error, nebula-metadata, nebula-action, nebula-resource, nebula-credential]
---

# nebula-plugin

## Purpose

Actions, Resources, and Credentials need a versioned distribution unit — one that the engine can catalog without re-inventing per-integration registration. `nebula-plugin` provides that unit: the `Plugin` trait (returning runnable trait objects per canon §3.5), a `ResolvedPlugin` per-plugin wrapper with eager component-contract snapshots, an in-memory `PluginRegistry`, and the pure Graph-v1 compiler over a frozen registry. The `PluginManifest` bundle descriptor lives in `nebula-metadata` and is re-exported here for source compatibility. Plugin authors implement `Plugin` in Rust, return their actions/credentials/resources from the trait methods, and register them **in-process** (ADR-0091 — out-of-process execution retired).

## Role

**Plugin Distribution Unit.** A plugin is the unit of **registration**, not the unit of size — a full integration crate ("Slack plugin" with many actions, credentials, and resources) and a micro-plugin (one resource + one credential) use the exact same contract: `Rust crate + plugin.toml marker + impl Plugin`. See canon §7.1 and `docs/INTEGRATION_MODEL.md`.

## Public API

- `Plugin` — base trait every plugin implements. Methods: `manifest() -> &PluginManifest`, `actions() -> Vec<Arc<dyn ActionFactory>>`, `credentials() -> Vec<Arc<dyn AnyCredential>>`, `resources() -> Vec<Arc<dyn ResourceFactory>>`, `on_load()`, `on_unload()` (default no-ops). Returns the runnable factories and erased credential implementations directly, matching canon §3.5.
- `PluginManifest` — re-exported from `nebula-metadata` (canonical home after ADR-0018 follow-up in slice B of the plugin load-path stabilization). Bundle descriptor with builder API: key, human name, semver version, group, `Icon`, maturity, deprecation, author/license/homepage/repository metadata. Does **not** compose `BaseMetadata<K>` — a plugin is a container, not a schematized leaf. New code should prefer importing from `nebula_metadata`.
- `ResolvedPlugin` — per-plugin wrapper with eager component caches. Constructed via `ResolvedPlugin::from(impl Plugin)`, which calls `actions()` / `credentials()` / `resources()` exactly once, validates the namespace invariant (every component key starts with `{plugin.key()}.`), and catches within-plugin duplicate keys; O(1) `action()` / `credential()` / `resource()` lookups thereafter. See ADR-0027.
- `PluginRegistry` — in-memory `PluginKey → Arc<ResolvedPlugin>` registry. Accessors: `all_actions()` / `all_credentials()` / `all_resources()` flat iterators across every registered plugin; `resolve_action()` / `resolve_credential()` / `resolve_resource()` lookups by full key.
- `PluginRegistry::freeze` — default-public activation boundary that consumes mutable assembly,
  re-validates dependencies, and returns an immutable `FrozenPluginRegistry`.
- `PluginSet` / `PluginContractDescriptor` — normalized registered-surface descriptor. Identity includes sorted plugin keys, component keys, dependency keys and normalized semver requirements; prerelease is logical identity while build metadata is excluded.
- `WorkerFlavorRevision` — combines the logical plugin-set identity with trusted artifact-set provenance and the logical runtime contract version.
- `RecordedWorkerFlavorRevisionV1` / `WorkerFlavorIntegrityError` — closed persisted flavor
  envelope and its checked-loading failure boundary. Deserialization creates only the untrusted
  envelope; reconstruction rejects unknown format/hash versions and a forged identity before a
  `WorkerFlavorRevision` is returned.
- `WorkerFlavorContext::from_registry` — derives a canonically ordered execution-facing view from a successfully frozen registry.
- `FrozenPluginRegistry::compile_graph_v1` — pure, deterministic compilation of one exact
  `WorkflowVersionId` and `WorkflowDefinition` into an opaque
  `ExecutablePlanRevision`. The compiler selects the registry's own exact plugin set/flavor,
  validates the closed Graph-v1 contract, and leaves resource/credential selectors abstract.
  New plans use compiler version 4 and canonical hash version 2 inside the unchanged v1
  record framing. They include intrinsic error-edge semantics and pin each action's explicit
  effect declaration. Compiler 1/hash 1 and compiler 3/hash 2 records retain their original
  bytes, hashes, and schema-v1 interpretation; other compiler/hash tuples are rejected.
  Compiler 4 admits the versioned scalar schema envelope. Exact registry comparison accepts
  compiler 3 and 4 but rejects absent effect declarations. An undeclared factory cannot
  produce a new durable plan.
- `ExecutablePlanRevision` / `RecordedExecutablePlanRevisionV1` — immutable checked plan and its
  persistable v1 projection. A recorded value becomes trusted only through the fallible integrity
  check; `validate_against` separately proves exact compatibility with a frozen registry.
- `PlanCompilationError` / `ExecutablePlanIntegrityError` /
  `PlanRegistryCompatibilityError` — distinct compile, recorded-integrity, and registry
  compatibility boundaries. None proves retention, artifact authenticity, tenant authority, or
  admission.
- `ComponentKind` — discriminant for namespace-mismatch and duplicate-component errors.
- `PluginError` — typed error for plugin operations (including `NamespaceMismatch`, `DuplicateComponent`, `AlreadyExists`).
- `PluginKey` — re-exported from `nebula-core`; stable identity type.
- `#[derive(Plugin)]` — proc-macro derivation for `Plugin` boilerplate.

## Contract

- **[L1-§7.1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract. No secondary manifest duplicating `fn actions()` / `fn credentials()` / `fn resources()` — `impl Plugin` is the single runtime source of truth for what is registered.
- **[L2-§7.1]** Three sources of truth, no drift: `Cargo.toml` (Rust package identity + dependency graph), `plugin.toml` (trust + compatibility boundary, read without compiling), `impl Plugin + PluginManifest` (runtime registration and bundle metadata). This crate owns the `impl Plugin` surface; `plugin.toml` parsing belongs to tooling.
- **[L2-§13.1]** Plugin load → registry: a plugin loads; Actions / Resources / Credentials from `impl Plugin` appear in the catalog without a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()`. Seam: `PluginRegistry::register(Arc<ResolvedPlugin>)` — construction of `ResolvedPlugin` enforces the `{plugin.key()}.` namespace invariant and rejects within-plugin duplicate keys before the entry reaches the registry. Test: unit tests in `crates/plugin/`.
- **Cross-plugin dependency rule** — types from another plugin come in only via `Cargo.toml
  [dependencies]` on the provider plugin crate, while the provider plugin is also declared in the
  consumer's `PluginManifest` dependency set. Rust enforces the type-level edge at link time;
  registry freeze and Graph-v1 compilation enforce the runtime load/version edge. Neither source
  is a substitute for the other.

## Immutable activation boundary

The default-public immutable worker-flavor vocabulary defines a closed activation epoch.
`PluginRegistry::freeze` consumes the mutable assembly registry,
re-validates its dependency graph, and derives:

1. a canonical `PluginSetId` from logical plugin versions, registered component
   keys, and declared dependency contracts; and
2. a `WorkerFlavorRevisionId` that additionally binds the runtime contract
   version and artifact-set digest.

The encoding is domain-separated and length-framed. Registration order,
component order, dependency order, and comparator order do not affect the
result; exact duplicate dependency declarations and comparators are collapsed.
Parsed semantic versions are encoded structurally with fixed operator tags and
big-endian numeric fields, so the v1 fingerprint does not depend on a
dependency crate's display formatting. Semver prerelease data remains part of
logical identity; build metadata is artifact provenance and is excluded from
logical versions.

`FrozenPluginRegistry::compile_graph_v1` consumes only the borrowed workflow contract and frozen
snapshots. It performs no lookup outside the registry, no selector resolution, no tenant or
credential/resource-ID binding, no persistence, and no runtime mutation. The plan's binding
requirements remain untrusted author selectors for the authenticated runtime-control plane to
resolve later.

First-party activation retains checked plans and worker flavors; runtime start atomically
materializes their execution bundle, and exact loading checks the retained registry before
dispatch. The boundary remains `partial`: binding resolution and the complete remote-effect
execution protocol have separate runtime admission requirements. Compilation grants neither.

The new plan hash domain is `nebula.executable-plan.graph.v2`. It includes the complete
static effect descriptor and a closed versioned policy projection. Dynamic requests, targets,
and credentials are not compiler inputs. Snapshot construction and exact compatibility checks
require a remote declaration to match the actual factory's remote capability before any action
is instantiated; a no-external-effects declaration must have no remote capability.

The composition root must supply `ArtifactSetDigest` and
`RuntimeContractVersion` from trusted activation state. Hash derivation does
not authenticate caller-provided bytes. Likewise, `PluginSetId` is an independent
registered-surface pin and audit identity. The ID alone is not proof of schemas, runtime
behavior, a complete frozen registry, capability possession, artifact authenticity,
compatibility, or execution authorization.

## Non-goals

- Not process/WASM isolation — out-of-process plugin execution was retired (ADR-0091, canon §12.6). Plugins run in-process as trusted code; process / OS / WASM isolation is a non-goal, not a deferred 1.0 capability.
- Not responsible for `plugin.toml` parsing or signature verification — those belong to pre-compile tooling (`cargo-nebula`); see canon §7.1.
- Not a persistent runtime catalog — this is an in-memory registry; persistence lives behind the
  storage port and is owned by the runtime-control path.

## Contract migration

The erased contribution traits expose required, snapshot-once contract projections:

- `ActionFactory::dependencies`;
- `ResourceFactory::dependencies` and `ResourceFactory::resource_type_id`;
- `AnyCredential::capabilities`.

These methods have no default-empty compatibility behavior. Custom implementations must return
their exact declared contract. Generic first-party factories cache the author declaration, and
`ResolvedPlugin::from` reads each projection once; freeze, compilation, and compatibility checks
consume only that immutable snapshot.

Metadata and `HasSchema` construction are fallible. Registry construction propagates typed
metadata admission errors without substituting an empty schema or partial component snapshot.
Intrinsic schema construction failures also remain typed causes of compilation and recorded-plan
integrity errors.

Recorded literal JSON is converted with `AuthoredValue::from_data`, so template-looking strings
and `$expr`-shaped objects stay data. Only the record's explicit expression and template variants
create expression nodes. Schema validation consumes the owned authored tree; the compiler still
grants neither runtime admission nor a resolved-value proof.
Literal secret checks include every read alias, even aliases shadowed by canonical keys: the
persisted record retains raw JSON, so preparation discarding a shadowed alias cannot erase its
plaintext from that record.

Persisted plan IDs remain independent of the authored tree wire format. Both existing plan hash
domains retain their original framing and use `canonical_json_v1` on the raw recorded JSON
projection. Schema canonicality checks use that same fixed JSON-v1 encoder; neither path serializes
an authored tree or changes existing recorded identity goldens.

Schema envelopes are shape-specific: Record/Union/Any remain schema wire version 1;
Scalar uses envelope version 2 with schema-owned scalar descriptor version 1. Old epochs
cannot admit a scalar even if its outer envelope is mislabeled as version 1. `()` and unit
structs now describe null, not `{}`; recompilation therefore records a distinct contract and
plan identity. A legacy empty Record is never reinterpreted as null on load. Logical
`PluginSetId` and flavor hashing remain unchanged; deployment provenance and runtime-contract
pins still belong to the composition root.

Scalar node inputs require an empty named-parameter map, which preserves the incoming root
payload. Flow edges use whole-schema assignability. Whole-output references into named scalar
fields remain unsupported without a schema-to-field assignability proof. New trigger plans
validate and retain the actual JSON root; only legacy plan checking retains null-to-object
normalization. Current node compilation and recorded-plan checking admit root rules only
when schema's static pass proves them against the complete parameter object. Pending root
checks, reference-bearing parameter objects, and passthrough root rules are rejected;
compiler epochs 1 and 3 retain their blanket node root-rule rejection. Compilation never
completes deferred root rules or grants runtime proofs.

## Maturity

See `docs/MATURITY.md` row for `nebula-plugin`.

- API stability: `partial`. `Plugin`, `ResolvedPlugin`, and the mutable registry are implemented.
  Frozen worker-flavor primitives, checked plan/flavor recorded forms, the pure Graph-v1 compiler,
  and registry compatibility check are consumed by first-party activation and exact execution.
  Complete binding and remote-effect admission remain separate runtime work. `PluginManifest` is canonical in
  `nebula-metadata` and re-exported here.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` enforced.
- Signing / trust boundary (`[signing]` in `plugin.toml`): `planned` — not enforced at runtime yet. See canon §7.1 and `docs/INTEGRATION_MODEL.md` signing section.

## Related

- Canon: `docs/PRODUCT_CANON.md` §1 (plugin as integration surface), §3.5 (Plugin = distribution + registration unit, returns runnable trait objects), §7.1 (packaging: `Cargo.toml` + `plugin.toml` + `impl Plugin`; unit of registration not size), §13.1 (plugin load → registry contract).
- ADRs: ADR-0018 (rename rationale), ADR-0027 (`ResolvedPlugin`, namespace invariant, registry accessors) — historical, indexed in the maintainers' private design vault.
- Integration model: `docs/INTEGRATION_MODEL.md` §7 — full plugin packaging mechanics, three-sources-of-truth rule, cross-plugin dependency rule, signing rationale, discovery / load lifecycle, ABI policy, tooling notes.
- Siblings: `nebula-metadata` (canonical `PluginManifest`), `nebula-core` (`PluginKey` identity type), `nebula-action` (`Action` trait), `nebula-resource` (`AnyResource` trait), `nebula-credential` (`AnyCredential` trait).
