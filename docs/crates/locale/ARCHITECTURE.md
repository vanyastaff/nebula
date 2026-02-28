# Architecture

## Problem Statement

- business problem:
  - user-facing workflow platform requires multi-language UX and localized diagnostics.
- technical problem:
  - provide one authoritative i18n/l10n contract across API, runtime, action, and validation errors.

## Current Architecture

- module map:
  - no `crates/locale` implementation yet.
  - localized strings are mostly ad-hoc and distributed.
- data/control flow:
  - locale selection and message formatting are not centralized.
- known bottlenecks:
  - inconsistent translation keys and fallback behavior across services.

## Target Architecture

- target module map:
  - `negotiation`: locale detection (Accept-Language, user/org settings, defaults)
  - `catalog`: loading/versioning of translation bundles
  - `format`: interpolation/pluralization/date-number formatting helpers
  - `context`: locale context propagation through runtime/action paths
  - `error`: localized error rendering adapters
- public contract boundaries:
  - `LocaleManager`, `LocaleContext`, translator API, and message-key conventions.
- internal invariants:
  - every localized render resolves via deterministic fallback chain.
  - missing keys are observable and non-silent.
  - interpolation variables are validated before render.
  - plugin catalog loading is deterministic:
    - if plugin contains `locales/`, catalogs are auto-discovered
    - only valid locale files are activated
    - invalid bundles fail plugin localization activation (with diagnostics), not whole platform startup.

## Design Reasoning

- key trade-off 1:
  - centralized localization crate improves consistency but adds runtime dependency to many layers.
- key trade-off 2:
  - rich ICU/Fluent-style formatting improves expressiveness but increases complexity.
- rejected alternatives:
  - per-crate independent localization implementations.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - explicit locale context propagation and fallback-first design.
  - message-key based rendering with parameterized templates.
- Reject:
  - free-form inline translation strings in business logic code paths.
- Defer:
  - real-time translation updates from remote control plane in first release.

## Breaking Changes (if any)

- change:
  - standardize message-key namespaces and interpolation rules.
- impact:
  - existing ad-hoc messages in consumers may require key migration.
- mitigation:
  - compatibility alias map and migration report tooling.

## Open Questions

- Q1: Fluent vs ICU message format as primary standard?
- Q2: where should locale preference precedence live (API gateway vs locale crate)?
