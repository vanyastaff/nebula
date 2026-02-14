---
title: Custom Actions
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# Custom Actions

This guide helps you design robust, reusable actions.

## Choosing the type
- Pure stateless work → ProcessAction
- Needs durable state → StatefulAction
- Emits events → TriggerAction / WebhookAction / PollingAction
- Streams data → StreamingAction
- Human/system pause → InteractiveAction
- Distributed transaction → TransactionalAction
- Needs background work → QueueAction + ScheduleAction
- Provides shared resource → SupplyAction

## Idempotency
- Derive a stable key from `(action_id, version, normalized_input)`.
- Store outcome keyed by this; short-circuit repeated calls.
- For webhooks/polling: use provider delivery IDs or offsets.

## Observability
- Log structured fields (no secrets).
- Emit counters/timers: `attempts`, `duration_ms`, `errors{kind}`.
- Attach `action_id`, `node_id`, `workflow_id`, `tenant` to all events.

## Security
- Validate inputs and schemas; mark secrets as sensitive in parameters.
- Timeouts and rate limiting are mandatory on network calls.
- Verify webhook signatures; rotate secrets via credentials system.

## Testing
- Use test contexts with fake time/clock and injected stubs.
- Golden tests for outputs; property-based tests for idempotency.
- Chaos tests for retry/backoff/lease expiration.
