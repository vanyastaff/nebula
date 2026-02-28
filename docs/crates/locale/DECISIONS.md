# Decisions

## D001: Localization requires a dedicated crate owner

Status: Adopt

Context:

Without central ownership, locale behavior drifts between crates.

Decision:

`nebula-locale` becomes the authoritative localization contract layer.

Alternatives considered:

- keep per-crate ad-hoc i18n implementations

Trade-offs:

- pro: consistent behavior and easier governance
- con: additional shared dependency

Consequences:

Cross-crate localization contracts become testable and versioned.

Migration impact:

Existing localized strings/errors need key mapping migration.

Validation plan:

Contract tests for negotiation/render/fallback across consumers.

## D002: Key-based rendering with deterministic fallback chain

Status: Adopt

Context:

Production systems need predictable behavior when keys/locales are missing.

Decision:

Use message keys + explicit fallback order (`requested -> user/org default -> global default`).

Alternatives considered:

- free-form string rendering per call site

Trade-offs:

- pro: consistency and observability
- con: key management overhead

Consequences:

Missing keys must be surfaced and monitored.

Migration impact:

Legacy inline messages require key extraction.

Validation plan:

Fallback determinism and missing-key tests.

## D003: Locale negotiation precedence is an explicit contract

Status: Adopt

Context:

Different entry points may provide conflicting locale hints.

Decision:

Define fixed precedence rules and expose them via one API.

Alternatives considered:

- caller-defined precedence for each endpoint

Trade-offs:

- pro: predictable user experience
- con: less flexibility for edge cases

Consequences:

API gateways must follow locale contract or provide explicit override flags.

Migration impact:

Some endpoints may need behavior alignment.

Validation plan:

Negotiation precedence matrix tests.

## D004: Localization errors are non-fatal to core execution path

Status: Adopt

Context:

Localization should not break core automation correctness.

Decision:

On locale/render failures, return safe fallback message while preserving original machine-readable error.

Alternatives considered:

- hard-fail execution on localization faults

Trade-offs:

- pro: operational resilience
- con: temporary UX degradation

Consequences:

Need telemetry for localization degradation events.

Migration impact:

Consumer error rendering paths adopt dual payload (localized + canonical).

Validation plan:

Fallback-on-error tests and telemetry assertions.

## D005: Dynamic catalog hot-reload deferred

Status: Defer

Context:

Hot-reload adds consistency and cache invalidation complexity.

Decision:

Initial implementation uses static/startup catalog loading; hot-reload postponed.

Alternatives considered:

- implement full runtime hot-reload immediately

Trade-offs:

- pro: faster reliable MVP
- con: slower content update workflow

Consequences:

Roadmap includes staged hot-reload with safety gates.

Migration impact:

Additive feature later.

Validation plan:

MVP stabilization before dynamic update support.
