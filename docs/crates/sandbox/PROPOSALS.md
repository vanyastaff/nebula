# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Unified Capability Schema

Type: Non-breaking

Motivation:

Capability semantics are currently mostly conceptual and need one canonical model.

Proposal:

Define shared typed schema for network/resource/credential/filesystem/time/memory capabilities.

Expected benefits:

Consistent enforcement across all sandbox backends.

Costs:

Schema governance and action metadata migration work.

Risks:

Overly rigid model may block legitimate integrations.

Compatibility impact:

Non-breaking if additive with defaults.

Status: Review

## P002: WASM Sandbox Backend

Type: Non-breaking

Motivation:

Need stronger isolation for untrusted/community actions.

Proposal:

Implement `SandboxRunner` backend with WASM runtime limits (memory/fuel/capabilities).

Expected benefits:

Improved security containment and tenant isolation posture.

Costs:

Complex runtime/tooling and compatibility considerations.

Risks:

Behavior differences versus in-process backend.

Compatibility impact:

Non-breaking if selected via policy.

Status: Draft

## P003: Process Sandbox Backend

Type: Breaking

Motivation:

Some workloads may require OS-level isolation controls unavailable in pure WASM.

Proposal:

Add process-isolation backend with cgroup/seccomp style policies.

Expected benefits:

Strong defense-in-depth for high-risk actions.

Costs:

High operational and platform complexity.

Risks:

Cross-platform drift and maintenance burden.

Compatibility impact:

Potential breaking operational assumptions.

Status: Defer

## P004: Capability Violation Policy Engine

Type: Non-breaking

Motivation:

Different environments need different reaction to violations.

Proposal:

Pluggable policy deciding fail-fast, audit-only, or quarantine/escalation behavior.

Expected benefits:

Flexible governance and incident response.

Costs:

More config and testing matrix.

Risks:

Misconfiguration could weaken security posture.

Compatibility impact:

Additive if default remains fail-fast.

Status: Review

## P005: Backend Parity Certification Suite

Type: Non-breaking

Motivation:

Multiple backends require consistent behavior guarantees.

Proposal:

Create parity test suite validating consistent semantics across drivers.

Expected benefits:

Reduced regressions and predictable migration paths.

Costs:

Ongoing maintenance of contract fixtures.

Risks:

False sense of parity if suite misses edge cases.

Compatibility impact:

Non-breaking.

Status: Draft
