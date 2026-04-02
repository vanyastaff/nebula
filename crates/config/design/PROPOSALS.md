# nebula-config — Change Proposals

This file tracks proposed changes to the nebula-config public API and
configuration contract. All proposals that affect compatibility must document
a migration path before merging.

## Active Proposals

_None at this time._

## Proposal Template

```markdown
### [PROP-XXX] Title

**Status:** Draft | Under Review | Accepted | Rejected

**Summary:** One-line description.

**Compatibility:** breaking | additive | no change

**Migration:** Describe old -> new mapping for any breaking change.

**Motivation:** Why this change is needed.
```

## Accepted Proposals

### [PROP-001] Add telemetry integration hook

**Status:** Accepted

**Summary:** Add optional `telemetry` field to `Config` for OpenTelemetry sink
configuration.

**Compatibility:** additive — new optional field with `None` default.

**Migration:** No migration required; existing consumers unaffected.
