# Decisions

## D001: Sandbox is defined as a port contract first

Status: Adopt

Context:

Runtime must remain decoupled from concrete isolation implementation.

Decision:

Use `nebula-ports::sandbox::SandboxRunner` as primary contract.

Alternatives considered:

- direct runtime dependency on one backend implementation

Trade-offs:

- pro: pluggable backends and cleaner architecture
- con: requires stricter contract discipline and compatibility testing

Consequences:

Backends can evolve independently if they keep port semantics.

Migration impact:

New backends added without runtime API churn.

Validation plan:

Contract tests across drivers.

## D002: In-process backend remains default for trusted execution

Status: Adopt

Context:

Current implementation exists and is operationally simple.

Decision:

Keep `sandbox-inprocess` as baseline backend while stronger isolation backends are built.

Alternatives considered:

- block release until wasm/process isolation is complete

Trade-offs:

- pro: immediate usability and low complexity
- con: weaker containment for untrusted code

Consequences:

Policy must prevent risky actions from using in-process backend.

Migration impact:

Future backend selector policy rollout required.

Validation plan:

Policy tests ensuring action classes map to allowed backend types.

## D003: Capability model is mandatory target contract

Status: Adopt

Context:

Action-level least-privilege is required for production security.

Decision:

Sandbox contract evolves toward explicit capability checks around sensitive operations.

Alternatives considered:

- role-only static trust model without fine-grained capabilities

Trade-offs:

- pro: fine-grained control and auditability
- con: additional metadata and enforcement complexity

Consequences:

Action metadata and context APIs must support capability declarations.

Migration impact:

Actions may need explicit capability declarations.

Validation plan:

Capability allow/deny integration tests.

## D004: No hidden retry policy inside sandbox backends

Status: Adopt

Context:

Retry semantics should be centralized for consistency and observability.

Decision:

Sandbox backends return explicit errors; runtime/resilience owns retry policy.

Alternatives considered:

- backend-internal retry loops

Trade-offs:

- pro: predictable control flow
- con: runtime must configure retries explicitly

Consequences:

Error classification contract with resilience becomes critical.

Migration impact:

None for current in-process backend behavior.

Validation plan:

Failure propagation and retryability mapping tests.

## D005: Full isolation backend is deferred but planned

Status: Defer

Context:

WASM/process isolation requires more engineering and ops hardening.

Decision:

Defer full backend to roadmap phases, while documenting required contracts now.

Alternatives considered:

- immediate full backend implementation without contract baseline

Trade-offs:

- pro: lower short-term risk
- con: temporary security posture gap for untrusted action scenarios

Consequences:

Need explicit policy guardrails before enabling third-party actions broadly.

Migration impact:

Introduce backend selection migration path once full backend ships.

Validation plan:

Roadmap exit criteria tied to production-grade isolation tests.
