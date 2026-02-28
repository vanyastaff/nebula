# Decisions

## D001: Pull-Based Task Acquisition

Status: Accepted

Decision:
- Worker pulls tasks from queue and owns lease heartbeat.

Why:
- Better overload control and easier horizontal scaling than push delivery.

Trade-offs:
- Requires robust lease TTL + heartbeat handling.

## D002: At-Least-Once Delivery Semantics

Status: Accepted

Decision:
- System guarantees at-least-once execution; exact-once is out of scope for worker core.

Why:
- Practical for distributed failures and queue redelivery behavior.

Trade-offs:
- Requires idempotency at action/runtime boundaries.

## D003: Fail-Closed Sandbox Policy

Status: Accepted

Decision:
- If sandbox policy cannot be applied, task is rejected/fails safely.

Why:
- Safety and tenant isolation are mandatory for untrusted workloads.

Trade-offs:
- Lower availability during sandbox subsystem incidents.

## D004: Explicit Draining State Before Shutdown

Status: Accepted

Decision:
- Worker has explicit `draining` state: no new claims, in-flight completes/cancels by policy.

Why:
- Prevents duplicate side effects during rolling deploys.

Trade-offs:
- Longer shutdown time if workloads are heavy.

## D005: Policy-Owned Retries via `resilience`

Status: Accepted

Decision:
- Worker does not hardcode retry rules; it executes policies provided by `resilience`.

Why:
- Centralized reliability governance and predictable behavior across crates.

Trade-offs:
- Integration complexity and policy version coordination.
