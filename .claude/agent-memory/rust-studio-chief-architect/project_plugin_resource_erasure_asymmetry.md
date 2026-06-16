---
name: plugin-resource-erasure-asymmetry
description: Plugin erasure-trinity asymmetry — ResourceDescriptor is metadata-only while ActionFactory constructs; plugin-declared resources cannot reach a live ManagedResource without an out-of-band KindActivator
metadata:
  type: project
---

Plugin contract erasure trinity is asymmetric (verified 2026-06-15 against as-built source on `charming-villani-b241bf`):

- `Plugin::actions() -> Vec<Arc<dyn ActionFactory>>` — `ActionFactory` has `metadata()` AND `instantiate(node, ctx) -> Future<ErasedAction>` (`crates/action/src/factory.rs:53-63`). A real factory: constructs runnable instances.
- `Plugin::resources() -> Vec<Arc<dyn ResourceDescriptor>>` — `ResourceDescriptor` has ONLY `key()` + `metadata()` (`crates/resource/src/resource.rs:44-50`). No factory, no `R`, no `Config`/`Topology`/`slot_bindings`.

**Live-registration trace (confirmed):** a plugin-declared resource reaches a live `ManagedResource<R>` ONLY through `Manager::register_resolved::<R>` (`crates/resource/src/manager/registration.rs:409`) → `register` funnel (`:54`) → `ManagedResource` (`:118`). That call needs `resource: R` + `R::Topology` + `R::Config` (by-value, monomorphized). The erased boundary (`KindActivator`, `crates/engine/src/resource/registrar.rs:326`) closes over per-`R` `resource_factory` + `topology_factory` — supplied by the composition root, NOT recovered from `dyn ResourceDescriptor`.

**The disconnect:** `PluginRegistry::all_resources()` (`crates/plugin/src/registry.rs:113`) has NO non-test caller. Its only use is `crates/engine/tests/resource_registrar_from_plugins.rs:220`, where it is read ONLY to extract the `kind` string — the concrete `KindActivator::<DemoResource,_,_>::new(DemoResource::new, || Resident::new(...))` is hand-written, fully bypassing the descriptor. The descriptor is decorative on the live path.

The engine documents the gap itself (`crates/engine/src/engine.rs:719-745`, test header `:1-35`): "engine cannot synthesize this allowlist by reflecting over `Plugin::resources()`." This IS the M12.4 bind-population frontier surfacing at the plugin contract.

**Stale-name defect:** engine docs + plugin README/DESIGN call the trait `AnyResource` (`engine.rs:151,724-732`; README:32,67) but the real trait is `ResourceDescriptor`. No `AnyResource` type exists. Pure doc rot — see [[feedback-skill-body-hygiene]].

Recommended fix (DECOMPOSITION lens): add a `ResourceFactory` registration arm symmetric to `ActionFactory` — an object-safe `fn register_into(&self, &mut ResourceActivatorRegistry)` (or `fn activator() -> Arc<dyn ResourceActivator>`) that `#[derive(Resource)]` emits, closing over the concrete `R`/`Topology`. `ResourceDescriptor` stays as the catalog/metadata view; the factory becomes the live-registration view. This keeps `slot_bindings`/`scope` caller-threaded (activation context, not plugin-declaration) while removing the disconnected hand-wired path. Respects in-process constraint (no sandbox).

See [[feedback-boundary-erosion]], [[feedback-type-enforce-not-discipline]].
