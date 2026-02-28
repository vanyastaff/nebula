# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Fluent-First Catalog Standard

Type: Non-breaking

Motivation:

Need expressive message formatting (pluralization, parameter interpolation).

Proposal:

Adopt Fluent syntax as primary catalog format with strict key conventions.

Expected benefits:

Richer localization quality and maintainable message templates.

Costs:

Translator and tooling onboarding.

Risks:

Format complexity for small teams.

Compatibility impact:

Additive if adapters exist for legacy keys.

Status: Review

## P002: Locale Coverage Gates in CI

Type: Non-breaking

Motivation:

Avoid shipping with missing critical translations.

Proposal:

CI gate verifies key completeness for required locales and fallback safety.

Expected benefits:

Higher release quality and fewer runtime fallback surprises.

Costs:

Build pipeline complexity and maintenance.

Risks:

Overly strict gates can slow delivery.

Compatibility impact:

Additive.

Status: Draft

## P003: Tenant-specific Locale Overrides

Type: Non-breaking

Motivation:

Enterprise tenants often need custom phrasing.

Proposal:

Support override bundles layered above base catalogs.

Expected benefits:

Enterprise flexibility without forking core catalogs.

Costs:

Override governance and conflict resolution.

Risks:

Inconsistent UX across tenants.

Compatibility impact:

Additive if fallback remains deterministic.

Status: Draft

## P004: Dynamic Catalog Hot Reload

Type: Breaking

Motivation:

Need faster localization updates without service restarts.

Proposal:

Allow runtime bundle updates with versioned cache invalidation.

Expected benefits:

Operational agility for localization changes.

Costs:

Concurrency and consistency complexity.

Risks:

Transient inconsistent renders during reload windows.

Compatibility impact:

Potentially breaking operational behavior.

Status: Defer

## P005: Locale Policy as Config Profile

Type: Non-breaking

Motivation:

Different deployments require different fallback and strictness policies.

Proposal:

Expose policy profiles (`strict`, `balanced`, `fallback-heavy`) in config.

Expected benefits:

Clear operator control over localization behavior.

Costs:

More config/test matrix complexity.

Risks:

Misconfiguration causing poor UX.

Compatibility impact:

Additive if default profile stable.

Status: Review

## P006: Plugin `locales/` Bundle Contract

Type: Non-breaking

Motivation:

Plugin developers need first-class localization without touching core catalogs.

Proposal:

Define standard plugin structure:
- `plugin-root/locales/en-US.ftl`
- `plugin-root/locales/ru-RU.ftl`
- keys namespaced by plugin ID (for example, `plugin.telegram.node.send.title`).

Locale manager auto-discovers `locales/` at plugin registration and merges valid bundles into runtime catalogs with namespace collision checks.
Validation gate:
- locale tag must be valid BCP-47
- file must parse as Fluent
- keys must match plugin namespace
- required key-set for declared plugin UI/action metadata must be present

Expected benefits:

Consistent plugin UX localization and better ecosystem DX.

Costs:

Bundle validation tooling and namespace governance.

Risks:

Key collisions or incomplete locale sets in third-party plugins.

Compatibility impact:

Additive if fallback remains deterministic and plugin namespace rules are enforced.

Status: Review
