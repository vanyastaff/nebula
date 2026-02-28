# Decisions

## D001: JSON value model as universal expression IO

Status: Adopt

Context:

Workflow data is heterogeneous and dynamic across nodes/services.

Decision:

Keep `serde_json::Value` as canonical expression input/output type.

Alternatives considered:

- strict typed value system as primary API

Trade-offs:

- pro: interoperability and flexibility
- con: runtime type checks and coercion complexity

Consequences:

Evaluator/type errors remain central to correctness.

Migration impact:

None immediate.

Validation plan:

Type/coercion regression tests and function behavior tests.

## D002: Engine-level optional caching

Status: Adopt

Context:

Parse/template overhead appears in hot workflow paths.

Decision:

Provide explicit cache-enabled constructors, keep no-cache default.

Alternatives considered:

- always-on cache

Trade-offs:

- pro: caller-controlled memory/perf balance
- con: configuration complexity and cache observability gaps

Consequences:

Runtime must choose cache strategy explicitly for workload.

Migration impact:

None; additive API.

Validation plan:

Parity tests with and without cache.

## D003: Safety guards in evaluator are mandatory

Status: Adopt

Context:

Expression inputs can be user-controlled and potentially hostile.

Decision:

Maintain recursion depth limit, regex pattern safeguards, and bounded template expression count.

Alternatives considered:

- rely only on runtime timeouts

Trade-offs:

- pro: defense-in-depth inside engine
- con: some valid edge-case expressions may be rejected

Consequences:

Safety guard behavior is part of compatibility contract.

Migration impact:

Potential breaking only if limits semantics change significantly.

Validation plan:

Security-focused tests for ReDoS/recursion/template limits.

## D004: Keep parser/eval internals outside stable contract

Status: Adopt

Context:

Internal compiler/evaluator architecture may evolve quickly.

Decision:

Publicly document hidden modules as non-stable while keeping high-level APIs stable.

Alternatives considered:

- stabilize AST/parser internals now

Trade-offs:

- pro: implementation agility
- con: advanced consumers need caution

Consequences:

Major API clarity maintained for normal consumers.

Migration impact:

Advanced consumers may require adaptation on internals.

Validation plan:

API documentation + semver discipline on stable surface.

## D005: Retry policy belongs to caller/resilience layer

Status: Adopt

Context:

Expression failures are mostly deterministic; retries can hide configuration bugs.

Decision:

Engine reports explicit errors and avoids hidden retry behavior.

Alternatives considered:

- built-in retries on eval failures

Trade-offs:

- pro: transparent failure semantics
- con: caller needs policy logic for transient failures

Consequences:

Integration docs must classify retryability clearly.

Migration impact:

None.

Validation plan:

Error-classification tests and resilience integration checks.
