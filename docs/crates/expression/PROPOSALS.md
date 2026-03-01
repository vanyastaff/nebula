# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Strict Mode (No Implicit Coercion)

Type: Breaking

Motivation:

Implicit coercions can hide expression mistakes and produce surprising results.

Proposal:

Add strict evaluation mode that rejects ambiguous conversions and requires explicit casts.

Expected benefits:

Safer production behavior and fewer hidden bugs.

Costs:

Migration effort for existing expressions.

Risks:

High breakage if enabled globally without staged rollout.

Compatibility impact:

Breaking if strict mode becomes default.

Status: Review

## P002: Function Namespace Versioning

Type: Non-breaking

Motivation:

Built-in function evolution needs compatibility control.

Proposal:

Introduce versioned function namespaces or compatibility profiles.

Expected benefits:

Safer upgrades and clearer migration paths.

Costs:

More complexity in docs and engine configuration.

Risks:

Confusion if too many profiles coexist.

Compatibility impact:

Additive if opt-in.

Status: Draft

## P003: Expression Cost Budget

Type: Non-breaking

Motivation:

Need deterministic protection against expensive expression abuse.

Proposal:

Add configurable cost budget (node visits/function calls/regex complexity) with hard fail when exceeded.

Expected benefits:

Stronger runtime protection and predictable resource usage.

Costs:

Instrumentation overhead.

Risks:

False positives for valid complex expressions.

Compatibility impact:

Non-breaking if disabled by default.

Status: Review

## P004: Ahead-of-Time Compilation Cache

Type: Non-breaking

Motivation:

Hot repeated expressions may benefit from compiled representation.

Proposal:

Add optional AOT compiled expression layer on top of parsed AST cache.

Expected benefits:

Lower latency and CPU usage in high-throughput systems.

Costs:

Implementation complexity and more cache lifecycle rules.

Risks:

Semantic mismatch between interpreted and compiled paths.

Compatibility impact:

Non-breaking if parity-guaranteed and opt-in.

Status: Draft

## P005: Policy-managed Function Allowlist

Type: Non-breaking

Motivation:

Different tenants/environments may need restricted function exposure.

Proposal:

Add policy-driven builtin allowlist/denylist per runtime context.

Expected benefits:

Security and governance improvements.

Costs:

Policy management and test matrix growth.

Risks:

Misconfiguration causing unexpected function failures.

Compatibility impact:

Additive.

Status: In Progress

Notes:

- engine now supports `ExpressionEngine::restrict_to_functions(...)` as an MVP allowlist.
- current scope is engine-level policy only; per-context and external policy composition remain open.
