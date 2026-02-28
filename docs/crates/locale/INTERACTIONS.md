# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `api` / `webhook`: locale negotiation from request/user context.
- `runtime` / `engine` / `execution`: localized operational messages for workflow surfaces.
- `action`: localized action-level error and status messages.
- `parameter`: localized parameter labels/descriptions/options in UI/SDK schemas.
- `plugin` / `sdk`: plugin-provided locale bundles and key namespace contracts.
- `validator`: localization of validation diagnostics.
- `core`: shared IDs and context primitives.
- `config`: locale defaults, fallback chains, and catalog source configuration.
- `log` / `telemetry`: locale-related diagnostics and missing-key metrics.

## Planned crates

- `locale` (this crate):
  - why it will exist: single owner for i18n/l10n contracts.
  - expected owner/boundary: negotiation, catalogs, translation/rendering policies.

## Downstream Consumers

- `api/webhook`:
  - expectations from this crate: deterministic locale resolution.
- `action/runtime/validator`:
  - expectations from this crate: stable key-based localized rendering.
- `plugin/sdk/parameter`:
  - expectations from this crate: deterministic plugin locale loading and namespaced key resolution.

## Upstream Dependencies

- `config`:
  - why needed: default locale and fallback policy.
  - hard contract relied on: validated locale settings.
  - fallback behavior if unavailable: default safe locale (`en-US`) and static bundles.
- `core`:
  - why needed: consistent context propagation primitives.
  - hard contract relied on: stable request/execution context metadata.
  - fallback behavior if unavailable: none.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| locale <-> api/webhook | in/out | locale negotiation contract | sync/async | safe fallback locale | ingress path |
| locale <-> runtime/action | out | key-based translation rendering | sync | fallback key/locale behavior | user-facing surface |
| locale <-> plugin/sdk | in/out | plugin locale bundle loading (`locales/*.ftl`) | sync/async load | fallback to platform/base locale | extensibility path |
| locale <-> parameter | out | localized parameter metadata rendering | sync | fallback to base key labels | UI/schema path |
| locale <-> validator | out | localized validation messages | sync | fallback to base language key | diagnostics |
| locale <-> config | in | default/fallback/catalog config | sync | default static fallback | bootstrap |
| locale <-> log/telemetry | out | missing-key and fallback metrics | async export | non-blocking observability | operations |

## Runtime Sequence

1. Ingress resolves preferred locale and creates `LocaleContext`.
2. Locale context propagates through execution/action/validation flows.
3. Plugin bundles (`locales/*.ftl`) are merged by namespace when plugin is loaded.
4. Services render message keys via locale manager.
5. Missing keys/fallback events are emitted to telemetry.

## Cross-Crate Ownership

- who owns domain model: `locale` owns localization contracts and rendering policy.
- who owns orchestration: runtime/api layers.
- who owns persistence: optional catalog storage backends.
- who owns retries/backpressure: caller policies for transient catalog failures.
- who owns security checks: auth layers and config governance outside locale core.

## Failure Propagation

- how failures bubble up:
  - translation errors surface as explicit locale errors with fallback details.
- where retries are applied:
  - transient catalog/backend retrieval failures.
- where retries are forbidden:
  - invalid locale/tag/key/interpolation contract errors.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable key resolution and fallback semantics within major versions.
- breaking-change protocol:
  - proposal -> decision -> migration guide -> major release.
- deprecation window:
  - one minor release minimum for key and API transitions.

## Contract Tests Needed

- locale negotiation precedence tests.
- fallback-chain behavior tests.
- message interpolation and pluralization correctness tests.
- missing-key observability tests.
- plugin namespace collision and fallback tests.
