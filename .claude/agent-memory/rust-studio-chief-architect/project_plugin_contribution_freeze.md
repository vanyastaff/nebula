---
name: plugin-contribution-freeze
description: Plugin→engine Resource contribution contract-freeze decision (B+ merged erased factory + lifecycle protocol); closes M12.4; ADR candidate
metadata:
  type: project
---

# Plugin contribution contract freeze (Resource/Action/Credential trinity)

DECISION (deliberation 2026-06-15): freeze the resource-contribution shape as a
**single merged erased factory trait** `ResourceFactory` (the maintainer-lens "merge
don't pair" refinement of B), driven by an **engine load/unload protocol** (D's
lifecycle win) — *without* a second descriptor surface and *without* a `#[resource(factory)]`
container attribute.

**Why:** The resource arm is asymmetric — `Plugin::resources()` returns descriptor-only
`Arc<dyn ResourceDescriptor>` (key+metadata) and can never construct a live
`ManagedResource<R>`. The live path (`KindActivator<R,FRes,FTopo>` →
`Manager::register_resolved::<R>`) is hand-wired at the composition root, disconnected
from the plugin. This is M12.4 bind-population.

**How to apply (grounded facts that constrain any future change here):**
- `ResourceActivator`/`KindActivator` (crates/engine/src/resource/registrar.rs:250/326)
  ALREADY is the object-safe 3-method erased shape (`register`+`resource_key`+`validate`).
  The freeze = move it DOWN into nebula-resource, rename to `ResourceFactory`, add
  `metadata()`, derive-emit it. It is a move+derive, not a new mechanism.
- `KindActivator` is **NOT Clone** (the two `#[derive(Clone)]` in registrar.rs are
  test-only at lines 823/1162). Any proposal calling `Arc::new(self.clone())` on it is wrong.
- `#[derive(Resource)]` emits slot plumbing ONLY (DeclaresDependencies, `<field>_slot()`,
  HasCredentialSlots). It does NOT emit Provider/key()/metadata()/topology. `key()`+`metadata()`
  are hand-written on `impl Provider`. Topology is author-spelled (no Default) — see the
  `from_plugins` test hand-supplying `Resident::new(config)`.
- The `#[resource(...)]` container attribute was **deliberately retired** (it emitted a
  `todo!()` body) — see crates/resource/macros/src/lib.rs:43. Do NOT reintroduce
  `#[resource(factory)]`; gate the factory emit on a `#[topology(...)]` attribute instead,
  or make it unconditional with topology from an associated const.
- Live registration must NOT be opt-in: every resource author needs a live factory, so
  emit it by default (opt-OUT if anything), per maintainer-lens critique of Proposal #3.
- `ResolvedPlugin` has NO Drop → L3 leak. on_load/on_unload are dead → L1. The load/unload
  protocol (InstallTxn atomic add + PluginHandle removal-funnel) fixes both structurally.
- Two frozen invariant laws REQUIRED with the shape: (1) schema-single-source — `metadata()`
  schema derives from the same `<R::Config as HasSchema>::schema()` that `validate()` uses,
  so catalog cannot advertise a config `register()` rejects; (2) removal-funnel — unload
  only via PluginHandle, raw registry mutation non-pub.

**Symmetry target:** `ActionFactory` (crates/action/src/factory.rs:53) =
`metadata()`+`instantiate()`, object-safe, derive-emitted GenericStatelessFactory<A>.
The resource arm mirrors this exactly. Trinity becomes uniform:
all three return `Vec<Arc<dyn …Factory>>`.

THE single product question that flips it: **is a pre-install visual catalog
(enumerate nodes/resources without running install side-effects) a hard requirement?**
If YES → erased factory (this decision). If NO → imperative `build(&mut Registrar)` (variant
C) is simpler and type-preserving. Nebula is n8n/Windmill/Dagster-class, so catalog ≈ hard req.

Relates to [[plugin-loading-reopened]] (in-process locked #805/ADR-0091 but contract must
survive a hypothetical future dynamic boundary — erased Vec-of-vtable is that shape; C welds
it shut). ADR candidate — supersedes the descriptor-only resources() contract.
