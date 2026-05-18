---
id: 0082
title: api-webhooks-idempotency
status: accepted
date: 2026-05-18
supersedes:
  - 0047-openapi-31-generator
  - 0048-idempotency-store-backend
  - 0049-webhook-handler-convergence
superseded_by: []
tags: [api, openapi, webhook, idempotency, m3, contract]
related:
  - docs/INTEGRATION_MODEL.md
  - docs/adr/HISTORICAL.md  # ADR-0022 webhook signature policy (historical)
---

# 0082. API edge contracts — OpenAPI, idempotency, webhooks (contract ADR)

## Context

M3 API-layer ADRs **0047–0049** cover the public HTTP contract surface: machine-readable
OpenAPI 3.1, safe retry via idempotency keys, and unified webhook ingress. Integrators
and agents previously opened three files for “how does the HTTP edge behave?” This
contract ADR captures **decisions**; route wiring and middleware mechanics remain in
[`docs/INTEGRATION_MODEL.md`](../INTEGRATION_MODEL.md) and `crates/api` README.

## Decision

### OpenAPI 3.1 generation (absorbs 0047)

Adopt **`utoipa`** to generate and serve OpenAPI 3.1 from handler annotations.
`/api/v1/openapi.json` and `/api/v1/docs` must reflect the **live route table** —
spec drift is a canon §4.5 violation. CI guards router ↔ spec parity.

### Idempotency store (absorbs 0048)

`IdempotencyLayer` uses a **hybrid** store: in-process L1 plus PostgreSQL-backed
persistence for multi-instance replay within the TTL window. Identity and body
fingerprints participate in the cache key.

### Webhook handler convergence (absorbs 0049)

Inbound webhooks share **one dispatch pipe** with two URL shapes (activation vs
generic ingress) so signature verification, activation lookup, and action triggering
do not fork. Aligns with ADR-0022 signature policy and storage activation repos.

## Consequences

- HTTP-edge contract questions → **0082**, then IM §API, then crate handlers.
- Stubs **0047–0049** redirect here; legacy links keep resolving.
- Implementation status (mounted middleware, stub handlers) is tracked in ROADMAP /
  crate README — not duplicated in this ADR body.

## Supersession

| Source ADR | Role |
|------------|------|
| [0047-openapi-31-generator](./0047-openapi-31-generator.md) | Stub → 0082 |
| [0048-idempotency-store-backend](./0048-idempotency-store-backend.md) | Stub → 0082 |
| [0049-webhook-handler-convergence](./0049-webhook-handler-convergence.md) | Stub → 0082 |
