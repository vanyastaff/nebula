# Decisions

## D001: Layered source precedence with deterministic merge

Status: Adopt

Context:
- multi-environment deployments need predictable override behavior.

Decision:
- keep explicit source priorities and deterministic merge order.

Alternatives considered:
- unordered source merge.

Trade-offs:
- predictable behavior, but requires strict documentation and tests.

Consequences:
- easier operations and incident debugging.

Migration impact:
- precedence changes become breaking.

Validation plan:
- precedence contract tests.

## D002: Dynamic JSON storage with typed access bridges

Status: Adopt

Context:
- consumers need both flexible and typed config access.

Decision:
- internal JSON tree + `get<T>` typed deserialization.

Alternatives considered:
- fully static typed config model only.

Trade-offs:
- flexibility with runtime path/type errors possible.

Consequences:
- broad compatibility across crates and plugin-like use cases.

Migration impact:
- low unless path semantics change.

Validation plan:
- type conversion tests across core primitive and structured types.

## D003: Validation as first-class activation gate

Status: Adopt

Context:
- invalid config must never silently activate.

Decision:
- run validators on merged config before activation/reload completion.

Alternatives considered:
- best-effort warnings only.

Trade-offs:
- stricter startup/reload failures but much safer runtime behavior.

Consequences:
- stronger reliability and operational correctness.

Migration impact:
- validator strictness changes may impact deployments.

Validation plan:
- invalid config integration tests with expected rejection.

## D004: Hot reload opt-in

Status: Adopt

Context:
- not all services need dynamic reconfiguration.

Decision:
- keep watcher/auto-reload explicit in builder configuration.

Alternatives considered:
- default always-on watch.

Trade-offs:
- explicit setup burden vs safer default behavior.

Consequences:
- avoids accidental mutable-runtime config behavior.

Migration impact:
- none.

Validation plan:
- start/stop watcher lifecycle tests.

## D005: Remote source expansion

Status: Defer

Context:
- source enum already models remote/database/kv, but default loader coverage is incomplete.

Decision:
- defer production remote source support until security/reliability model is finalized.

Alternatives considered:
- implement quickly with partial guarantees.

Trade-offs:
- slower feature rollout, higher confidence.

Consequences:
- current deployments focus on file/env/local composite reliability.

Migration impact:
- additive when introduced with clear contracts.

Validation plan:
- provider contract tests + chaos/failure simulations before GA.

## D006: Compatibility governance for contract evolution

Status: Adopt

Context:
- precedence/path/error semantics are consumed across multiple crates and releases.

Decision:
- minor releases remain additive; contract-semantic changes require major release.
- any proposed breaking change requires migration mapping and updated fixtures.

Alternatives considered:
- implicit compatibility enforcement via release notes only.

Trade-offs:
- stronger release discipline and higher upfront documentation effort.

Consequences:
- earlier detection of downstream breakage risk.

Migration impact:
- old -> new behavior mapping becomes mandatory for breaking changes.

Validation plan:
- governance and migration contract tests in `crates/config/tests/contract/*`.

## D007: Validator crate integration via trait bridge

Status: Adopt

Context:
- config lifecycle needs explicit integration with `nebula-validator` semantics.

Decision:
- provide direct trait-bridge integration for validator types under `ConfigValidator`.
- preserve config-owned activation and fallback semantics while reusing validator contracts.

Alternatives considered:
- direct dependency on validator internals in config core lifecycle.

Trade-offs:
- adapter introduces small mapping layer, but keeps boundaries clean.

Consequences:
- consistent cross-crate validation behavior and simpler compatibility testing.

Migration impact:
- bridge behavior changes are contract-significant and require mapping updates.

Validation plan:
- validator integration contract tests in `crates/config/tests/contract/validator_*`.
