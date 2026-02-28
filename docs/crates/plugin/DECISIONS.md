# Decisions

## D001: Plugin as Parent of Action, Credential, Resource

Status: Adopt

Context: The platform initially used "Node" as the packaging unit. We needed a clearer hierarchy: what is the user-visible unit, and what belongs to it?

Decision: **Plugin** is the canonical packaging unit. Action, Credential, and Resource belong to Plugin. Node (as a graph vertex) is a workflow concept; Plugin is the integration packaging concept.

Alternatives considered:
- Node as parent of Action only
- Flat structure (Action, Credential, Resource independent)

Trade-offs: Plugin adds one layer of indirection; clarifies ownership and discovery.

Consequences: All integration docs reference Plugin; Node docs archived. SDK/registry consume Plugin, not Node.

Migration impact: Documentation and mental model only; codebase already used Plugin.

Validation plan: INTERACTIONS.md and ARCHITECTURE.md reflect this; no code migration needed.

---

## D002: Object-Safe Plugin Trait

Status: Adopt

Context: Plugins must be storable in a registry as `Arc<dyn Plugin>` and loadable dynamically.

Decision: `Plugin` trait is object-safe: no generic methods, no associated types that prevent dynamic dispatch.

Alternatives considered:
- Generic Plugin with type parameters
- Enum of plugin kinds

Trade-offs: Object-safe enables registry and dynamic loading; limits trait design.

Consequences: `register(&mut PluginComponents)` is the only callback; no async in trait.

Migration impact: None.

Validation plan: `Arc<dyn Plugin>` used in tests and registry.

---

## D003: u32 Versioning

Status: Adopt

Context: Plugins need versioning for multi-version support and migration.

Decision: Use `u32` for version numbers (1-based). Semver can be in metadata for UI if needed.

Alternatives considered:
- semver::Version
- String version

Trade-offs: u32 is simple, sortable, no parsing; semver in metadata for display.

Consequences: `PluginVersions` keyed by u32; `Plugin::version()` returns u32.

Migration impact: None.

Validation plan: Version resolution tests pass.

---

## D004: Plugin registry not thread-safe by design

Status: Adopt

Context: Registry is used during bootstrap; runtime may wrap in RwLock.

Decision: `PluginRegistry` is not `Sync`; caller wraps in `RwLock` if shared across threads.

Alternatives considered:
- Built-in RwLock in registry
- AtomicRefCell

Trade-offs: Simpler API; no hidden locking; caller controls concurrency.

Consequences: Runtime/registry must wrap registry.

Migration impact: None.

Validation plan: Documented in API; usage in runtime/registry.

---

## D005: InternalHandler as Temporary Placeholder

Status: Adopt

Context: Action adapters (ProcessActionAdapter, etc.) are not yet restored; plugin needs to register handlers.

Decision: Use `InternalHandler` trait as placeholder; restore typed `process_action`/`stateful_action` when action adapters are available.

Alternatives considered:
- Block plugin until action adapters ready
- Keep InternalHandler permanently

Trade-offs: Unblocks plugin development; temporary API surface.

Consequences: `PluginComponents::handler()` accepts `Arc<dyn InternalHandler>`; typed methods commented out.

Migration impact: When restored, add typed methods; deprecate InternalHandler after migration.

Validation plan: Handler registration works; runtime can execute.
