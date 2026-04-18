---
name: Nebula observability contract
description: SLI / SLO / error budget, structured event schema for execution_journal, core analysis loop for operators. Fills in Pass 3.
status: skeleton
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, MATURITY.md]
---

# Nebula observability contract

> **Status:** skeleton; content fills in Pass 3 of the docs redesign.

## 1. Service level indicators (SLIs)

*Filled in Pass 3. Candidate SLIs from spec §10 (OBSERVABILITY rationale):*

- `execution_terminal_rate` — percent of started executions reaching terminal state.
- `cancel_honor_latency` — p95 time from outbox Cancel row to terminal Cancelled.
- `checkpoint_write_success_rate` — percent of checkpoint writes that succeed.
- `dispatch_lag` — p95 delay between outbox row insert to consumer acknowledgement.

## 2. Service level objectives (SLOs)

*Filled in Pass 3.*

## 3. Error budgets

*Filled in Pass 3.*

## 4. Structured event schema (execution_journal)

*Filled in Pass 3. Fields (proposed, subject to code verification):*
- `execution_id`, `node_id`, `attempt`, `correlation_id`
- `trace_id`, `span_id`, `event_type`, `payload`, `timestamp`

## 5. Core analysis loop

*Filled in Pass 3. Four-step operator procedure from Observability Engineering:*
1. What failed?
2. When?
3. What changed?
4. What to try?
