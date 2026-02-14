---
title: Action Catalog
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# Action Catalog

This catalog summarizes available action types and when to use them.

| Type | Purpose | Typical Use | Key Methods |
|---|---|---|---|
| **ProcessAction** | Stateless, pure function over input -> output | Mapping, HTTP call, transform | `execute(input, ctx)` |
| **StatefulAction** | Keeps persistent state between runs | Counters, windows, rolling aggregates | `execute_with_state(input, &mut state, ctx)`, `migrate_state` |
| **TriggerAction** | Emits events to start/advance workflows | Kafka consumer, S3 notifications | `start(config, tctx) -> EventStream`, `stop()` |
| **PollingAction** | Periodically fetches items with cursor & lease | Gmail/IMAP polling, API pagination | `poll(config, cursor, lease, ctx)` |
| **WebhookAction** | Handles inbound HTTP callbacks with verify/auth | OAuth redirect, Stripe webhook | `subscribe(config, wctx)`, `handle(request, wctx)`, `unsubscribe()` |
| **StreamingAction** | Produces/consumes item streams with backpressure | File processing, LLM token stream | `open_stream`, `next_chunk`, `close` |
| **InteractiveAction** | Awaits external input/approval | Human-in-the-loop, CAPTCHA solve | `prompt`, `await_input`, `resume` |
| **TransactionalAction** | Do step with compensation (Saga) | Payments, inventory reservation | `try`, `confirm`, `compensate` |
| **QueueAction** | Enqueue/dequeue tasks reliably | Background job queues | `enqueue`, `dequeue`, `ack/nack` |
| **ScheduleAction** | Time-based schedule/cron | Nightly compaction, reports | `plan`, `due(now)`, `next_after` |
| **SupplyAction** | Create/manage resources for reuse | DB connection pools, clients | `create`, `health_check`, `destroy` |

> Not all types must be separate traits at runtime; some can be feature-extensions over `Action`. In Nebula, we prefer explicit traits for clarity and compile-time guarantees.
