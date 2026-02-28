# Proposals

## P001: Resource Registration in PluginComponents

Type: Non-breaking

Motivation: Plugin should declare resource requirements (e.g. DB pool, HTTP client) alongside credentials. Resource crate defines `ResourceDescription`; plugin collects them.

Proposal: Add `resource(desc: ResourceDescription)` to `PluginComponents`; add `resources()` getter. Mirror credential pattern.

Expected benefits: Consistent plugin authoring; runtime can resolve resources from plugin metadata.

Costs: Resource crate must define `ResourceDescription`; plugin depends on resource.

Risks: Resource crate design may change.

Compatibility impact: Additive; no breaking changes.

Status: Draft

---

## P002: Plugin Capability Hints for Sandbox

Type: Non-breaking

Motivation: Runtime/sandbox could use plugin metadata to enforce least-privilege (e.g. "this plugin needs network", "this plugin needs filesystem").

Proposal: Add optional `capabilities: Vec<Capability>` to `PluginMetadata`; runtime checks before execution.

Expected benefits: Safer sandbox policy; explicit capability declaration.

Costs: Plugin authors must declare capabilities; schema evolution.

Risks: Over/under declaration; false sense of security.

Compatibility impact: Additive; optional field.

Status: Defer

---

## P003: #[derive(Plugin)] Macro

Type: Non-breaking

Motivation: Reduce boilerplate for simple plugins that only wrap metadata and register components.

Proposal: Add `#[derive(Plugin)]` in nebula-macros; generates `metadata()`, `register()` from struct fields and a `register_components` method.

Expected benefits: Faster plugin authoring; fewer errors.

Costs: Macro complexity; limited flexibility for custom plugins.

Risks: Macro may not cover all cases.

Compatibility impact: Optional; manual impl still supported.

Status: Defer
