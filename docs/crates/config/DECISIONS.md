# nebula-config — Architecture Decisions

## Versioning Policy

### Decision: major-version bump for precedence and path-breaking changes

**Rule:** Any change to the configuration key precedence order or to the
canonical path of an existing configuration field requires a **major** version
bump (e.g. `v1 → v2`).

**Rationale:** Precedence reordering silently changes effective values for
consumers who rely on the existing merge order (env > file > defaults). Path
renames force all downstream references to update. Both are breaking changes.

**Additive rule:** minor releases remain additive — new fields with defaults
and new precedence sources appended at the lowest priority are allowed in minor
releases, because they cannot silently change existing effective values.

## Validation Contract

### Decision: validator rejections are synchronous and fatal by default

Failed validator checks during config load reject the candidate configuration
and preserve the **last-known-good** snapshot. This is deliberate: allowing
invalid config to become active creates undefined runtime behavior.

## Reload Atomicity

### Decision: configuration reloading uses atomic swap

Config reload applies all validated changes in a single atomic swap so
consumers never observe a partially-updated config.
