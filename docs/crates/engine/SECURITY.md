# Security

## Threat Model

- **Assets:** Execution state (may include node outputs, metadata); workflow definitions and input (may be tenant-scoped). Credentials and secrets are in credential crate; engine passes refs only.
- **Trust boundaries:** Engine orchestrates; runtime/sandbox execute untrusted or semi-trusted actions. Engine must not leak execution state across tenants if multi-tenant.
- **Attacker capabilities:** Malicious workflow definition (e.g. excessive nodes, large payloads); DoS via many concurrent executions; cross-tenant access if scope is not enforced.

## Security Controls

- **Authn/authz:** Not in engine; API or caller enforces who can invoke execute_workflow. Engine may receive scoped workflow_id and options; tenant/scope enforcement at API layer.
- **Isolation:** Action execution is in runtime/sandbox; engine does not execute user code directly.
- **Input validation:** Workflow definition validated by workflow crate before engine uses it; parameter resolution and expression evaluation use expression crate (sanitization there). Engine should enforce execution budget (max nodes, max total bytes) to limit DoS.
- **Secret handling:** Engine does not handle secrets; credential refs only; resolution in runtime/action/credential layer.

## Abuse Cases

- **Run-away execution (DoS):** Prevention: execution budget (max nodes, timeout, max_total_execution_bytes). Detection: metrics on execution count and duration. Response: reject new executions or cancel when budget exceeded.
- **Cross-tenant execution:** Prevention: API passes only workflow_ids and options the caller is allowed to run; engine trusts caller. Detection: audit logs. Response: revoke API access.
- **Large payload / memory:** Prevention: data limits, spill-to-blob policy; backpressure (P-002). Detection: memory pressure, metrics. Response: reject or spill.

## Security Requirements

- **Must-have:** No execution of user code in engine; execution budget and timeouts; scope/tenant enforced by caller.
- **Should-have:** Document that API must enforce tenant isolation and rate limits.

## Security Test Plan

- Unit/integration: execution budget enforced; timeout and cancel paths work.
- No credential or secret in engine types; contract tests for context shape (no secret fields).
