# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Sticky Worker Affinity for Repeated Attempts

Type: Non-breaking

Motivation:

Reduce warm-up and cache miss costs for retries/repeated similar tasks.

Proposal:

Allow optional lease hint to route retry attempts to same worker when healthy.

Expected benefits:

Lower retry latency and better cache locality.

Costs:

More scheduler complexity and potential uneven load.

Risks:

Affinity hotspots during partial worker failures.

Compatibility impact:

Additive.

Status: Defer

## P002: Multi-Queue Priority Classes

Type: Non-breaking

Motivation:

Different workload classes need predictable latency isolation.

Proposal:

Introduce priority queues (`critical`, `default`, `bulk`) with weighted polling.

Expected benefits:

Better SLO control for critical executions.

Costs:

Operational tuning and queue topology complexity.

Risks:

Starvation of low-priority workloads if weights are misconfigured.

Compatibility impact:

Additive with default single-queue fallback.

Status: Review

## P003: Durable Local Recovery Buffer

Type: Breaking

Motivation:

Need stronger resilience during long upstream outages.

Proposal:

Persist in-flight completion envelopes locally and replay on reconnect.

Expected benefits:

Reduced risk of completion signal loss during dependency outages.

Costs:

Disk I/O overhead and recovery semantics complexity.

Risks:

Split-brain style duplicate finalization if replay contract is weak.

Compatibility impact:

May change failure and redelivery behavior.

Status: Draft

## P004: WASI-First Sandbox Backend

Type: Breaking

Motivation:

Improve portability and safety for untrusted actions.

Proposal:

Make WASI runtime primary backend, process/container sandbox as fallback.

Expected benefits:

Stronger syscall surface control and reproducibility.

Costs:

Migration effort for existing action runtimes.

Risks:

Compatibility gaps for native dependencies.

Compatibility impact:

Potentially breaking for plugins relying on full OS environment.

Status: Defer
