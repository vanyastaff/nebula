# nebula-expression Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflows need to compute values at runtime: "use the output of the previous node", "format this date", "check if this string matches that regex". Writing custom code for every transformation would be brittle and unsafe. The platform needs a single expression and template engine with predictable semantics and security boundaries.

**nebula-expression is the expression and template engine for dynamic workflow data transformation.**

It answers: *Given a string expression or template and a runtime context (node outputs, execution metadata, inputs), how does the platform produce a value or rendered text without executing arbitrary code?*

```
runtime builds EvaluationContext (variables, node outputs, execution/workflow IDs)
    ↓
node or action requests evaluation: expression "{{ $json.body.title }}" or template
    ↓
ExpressionEngine parses (optional parse cache), evaluates AST with builtins
    ↓
returns serde_json::Value or ExpressionError (deterministic, no side effects)
```

This is the expression contract: same expression + same context ⇒ same result; untrusted input is guarded by recursion depth, regex ReDoS checks, and a closed builtin set.

---

## User Stories

### Story 1 — Workflow Author Writes an Expression in the UI (P1)

A workflow author configures a node with an expression like `{{ $json.body.count + 1 }}`. The UI sends the expression string; at execution time the runtime provides context (e.g. previous node output as `$json`). The result is a number. No custom code is deployed.

**Acceptance**:
- ExpressionEngine evaluates expression with EvaluationContext
- Builtins (math, string, array, object, datetime) are available and documented
- Parse and eval errors are structured (ExpressionError) with positions where possible
- Recursion depth and regex patterns are limited to prevent abuse

### Story 2 — Action Developer Evaluates a Template (P1)

An action needs to render a message template with placeholders. It uses the template API: "Hello {{ $json.name }}, run at {{ now() }}". Whitespace and optional expressions are supported. The action receives a single rendered string.

**Acceptance**:
- Template with `{{ }}` and optional `{{- }}` / `{{ -}}` for whitespace control
- EvaluationContext supplies variables; builtins available in template expressions
- Template errors include position so UI can highlight the problem

### Story 3 — Runtime Provides Scoped Context (P2)

The runtime builds EvaluationContext with execution-scoped and workflow-scoped variables. Expression engine does not know about execution lifecycle; it only reads from context. Caching (if used) must be keyed by scope so one execution does not see another's cached result.

**Acceptance**:
- EvaluationContext is built by runtime/engine; expression crate does not depend on them
- Optional cache (e.g. via nebula-memory) is keyed by (scope, expression) or equivalent
- No cross-execution or cross-workflow leakage via cache

### Story 4 — Operator Gets Deterministic and Observable Failures (P2)

When an expression fails (syntax error, type error, missing variable), the failure is deterministic and classifiable. Operators and logs can distinguish "user typo" from "platform bug" and never see arbitrary panics from expressions.

**Acceptance**:
- ExpressionError variants cover parse, eval, type, missing variable, safety (recursion, regex)
- No panic in hot path; errors are Result-based
- Security-related limits (depth, regex) are documented and configurable where appropriate

---

## Core Principles

### I. Same Expression + Same Context ⇒ Same Result

**Evaluation is deterministic and side-effect free for the same inputs.**

**Rationale**: Workflows that depend on expressions must be reproducible and testable. Non-determinism would break idempotency and debugging.

**Rules**:
- Builtins that would be non-deterministic (e.g. random) are either excluded or explicitly documented
- No hidden state in evaluator that changes result for same (expression, context)
- Cache layer must not alter semantics — only performance

### II. Closed World: Builtins and Context Only

**Expressions can only access what is in EvaluationContext and the registered builtin set. No arbitrary code execution.**

**Rationale**: Workflow expressions often come from users or stored workflows. Open execution would be a security risk. Closed world keeps the attack surface bounded.

**Rules**:
- No eval-of-string-as-code; no dynamic script loading in expression path
- New builtins are added explicitly; no escape hatch to run user code
- Context shape is defined by runtime; expression crate does not assume beyond the API

### III. Safety Guards Always On

**Recursion depth, regex ReDoS checks, and other safety limits are always enforced in library code.**

**Rationale**: Untrusted or buggy expressions must not hang or exhaust memory. Guards are not optional for production.

**Rules**:
- Recursion depth limit enforced in evaluator
- Regex patterns checked for ReDoS-prone constructs or run with resource limits
- Document limits and behavior when limits are hit

### IV. Errors Are Structured and Actionable

**Parse and eval failures produce ExpressionError with enough information to show the user where the problem is.**

**Rationale**: "Expression failed" is not enough. UI and logs need position, variant (syntax vs type vs missing variable), and optional suggestion.

**Rules**:
- ExpressionError is an enum with clear variants
- Where possible, span or position is attached
- No loss of error info when converting to string (e.g. Debug/Display)

### V. Cache Is an Optimization, Not Part of Semantics

**Optional parse/eval caches (e.g. via nebula-memory) must not change the result of evaluation.**

**Rationale**: Caching is for performance. If cache could change semantics, debugging and correctness would be impossible.

**Rules**:
- Cache key must include all inputs that affect result (expression, context shape, or explicit scope)
- Cache miss must produce same result as no cache
- Invalidation and TTL are documented

### VI. No Orchestration or Storage in This Crate

**Expression engine evaluates; it does not schedule workflows, persist data, or manage credentials.**

**Rationale**: Single responsibility. Orchestration and storage belong to engine/runtime/storage. Expression is a pure function over context.

**Rules**:
- No dependency on engine, runtime, workflow, or storage for core evaluation path
- Optional integration (e.g. memory for cache) behind feature or trait

---

## Production Vision

### The expression engine in an n8n-class fleet

In a production Nebula deployment, every node that needs dynamic values uses the same expression engine. Expressions are stored in workflow JSON; at runtime the engine builds EvaluationContext (node outputs, execution ID, workflow ID, inputs) and calls ExpressionEngine. Results are JSON values or rendered strings. No per-tenant or per-workflow expression runtime — one engine, many invocations with different context.

```
WorkflowRun
    │
    ├── Node A output → context.$json
    ├── Node B: expression "{{ $json.count * 2 }}"
    │       → ExpressionEngine.evaluate(expr, context) → Value
    │
    └── Node C: template "Report: {{ $json.title }} at {{ now() }}"
            → ExpressionEngine.render(template, context) → String
```

Optional: expression parse cache and evaluation cache (e.g. scope-aware in nebula-memory) reduce CPU for repeated expressions. Security: recursion depth and regex guards are always on; builtins are closed set.

### From the archives: language and context boundaries

The expression archive (e.g. `_archive/README.md` and referenced architecture docs) places the expression layer in the "Core Layer" with workflow, execution, memory, eventbus. The design reasoning in ARCHITECTURE.md states: "feature-rich DSL improves developer velocity but increases semantic stability burden" and "explicit runtime context variables and function registry model" are adopted (n8n-like). Production vision aligns: stable ExpressionEngine and EvaluationContext API, closed builtin set, and deterministic errors. No arbitrary script execution.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Explicit compatibility policy for builtins and syntax | High | Document stable vs experimental; versioning for new builtins |
| Scope-aware cache keys (execution/workflow) | High | Prevent cross-execution cache hits when using nebula-memory |
| Formal grammar/schema snapshot for compatibility | Medium | Prevent accidental grammar breaks; migration for intentional breaks |
| Configurable safety limits (depth, regex) | Medium | Allow operators to tune without code change |
| Builtin versioning or namespacing | Low | Long-term compatibility for third-party extensions |

---

## Key Decisions

### D-001: Expression Language Over JSONPath-Only

**Decision**: Provide a full expression language (operators, conditionals, pipelines, lambdas, builtins), not only JSONPath-style queries.

**Rationale**: Workflow authors need transforms and conditions (math, string, date) without custom code. JSONPath alone is too limited for n8n-class UX.

**Rejected**: JSONPath-only — insufficient for business logic in workflows.

### D-002: serde_json::Value as the Value Type

**Decision**: Evaluation produces serde_json::Value (and templates produce strings). No custom value type in expression crate.

**Rationale**: Interop with rest of platform (workflow JSON, node I/O). One less type to convert at boundaries.

**Rejected**: Custom Nebula value type in expression — would push conversion into every caller.

### D-003: EvaluationContext Built by Caller

**Decision**: Expression engine accepts an EvaluationContext; it does not build context from execution or workflow types.

**Rationale**: Engine/runtime own execution and workflow state. Expression crate stays dependency-leaf for those domains and only defines the context interface.

**Rejected**: Expression crate depending on engine to build context — would create cycle.

### D-004: Caching Optional and Behind Abstraction

**Decision**: Parse and eval caches are optional; integration with nebula-memory (or similar) is behind feature or trait so core evaluation does not require it.

**Rationale**: Keeps expression crate usable in minimal environments; cache is an optimization, not part of contract.

**Rejected**: Mandatory cache dependency — would force all consumers to pull cache backend.

---

## Open Proposals

### P-001: Builtin Compatibility and Versioning Policy

**Problem**: Adding or changing builtins can break existing workflows.

**Proposal**: Document stable builtins; new builtins in minor; behavior changes in major with migration notes. Optional namespacing for experimental builtins.

**Impact**: Non-breaking if adopted as policy; may require deprecation path for existing builtins.

### P-002: Scope-Aware Cache Key Contract

**Problem**: Cache shared across executions could leak or return wrong result.

**Proposal**: Define cache key to include execution_id (or scope_id); document in INTERACTIONS with nebula-memory.

**Impact**: Requires memory/engine to pass scope into cache key; additive for expression API.

### P-003: Grammar Snapshot and Compatibility Tests

**Problem**: Parser changes can accidentally change semantics.

**Proposal**: Snapshot tests for parse results and eval results on a fixed expression set; treat grammar as stable in patch/minor.

**Impact**: Improves stability; no breaking change.

---

## Non-Negotiables

1. **Deterministic evaluation** — same expression + same context ⇒ same result; no hidden state.
2. **Closed world** — only EvaluationContext and registered builtins; no arbitrary code execution.
3. **Safety guards always on** — recursion depth and regex ReDoS protections in library path.
4. **Structured errors** — ExpressionError with variants and, where possible, position.
5. **Cache does not change semantics** — cache is optimization only; invalidation and keying documented.
6. **No orchestration or storage in this crate** — expression evaluates; engine/runtime provide context and persistence.
7. **Breaking grammar or builtin behavior = major + MIGRATION.md** — workflow expressions are long-lived.

---

## Governance

- **PATCH**: Bug fixes, docs, internal refactors. No change to ExpressionEngine, EvaluationContext, Template, or ExpressionError semantics.
- **MINOR**: Additive only (new builtins, new optional features). No removal or change of existing expression semantics or builtin behavior.
- **MAJOR**: Breaking changes to grammar, builtin behavior, or public API. Requires MIGRATION.md.

Every PR must verify: no new code execution path for untrusted input; safety guards remain; cache (if used) does not alter semantics.
