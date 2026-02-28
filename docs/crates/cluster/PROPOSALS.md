# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Multi-strategy Scheduler Profiles

Type: Non-breaking

Motivation:

Workloads differ in locality, fairness, and latency needs.

Proposal:

Expose scheduler profiles combining strategy + weights (latency, load, affinity).

Expected benefits:

Simpler policy tuning and better workload fit.

Costs:

More config complexity and tuning overhead.

Risks:

Misconfigured profiles causing unfair distribution.

Compatibility impact:

Additive if profile defaults stay stable.

Status: Review

## P002: Control-plane Event Log Replay

Type: Non-breaking

Motivation:

Need reliable state reconstruction and incident analysis.

Proposal:

Add append-only event log replay for membership/placement transitions.

Expected benefits:

Improved auditability and faster recovery diagnostics.

Costs:

Storage and replay complexity.

Risks:

Replay bugs could diverge from live state.

Compatibility impact:

Additive.

Status: Draft

## P003: Adaptive Autoscaling Controller

Type: Breaking

Motivation:

Static thresholds may underperform across mixed workloads.

Proposal:

Introduce adaptive autoscaling with predictive metrics.

Expected benefits:

Better elasticity and cost/performance balance.

Costs:

Significant complexity and tuning needs.

Risks:

Oscillation and instability under noisy metrics.

Compatibility impact:

Potentially breaking operational semantics.

Status: Defer

## P004: Multi-region Cluster Federation

Type: Breaking

Motivation:

Global deployments need geo-distributed execution.

Proposal:

Federate clusters with region-aware placement and failover policies.

Expected benefits:

Higher resilience and locality optimization.

Costs:

Very high architectural and operational complexity.

Risks:

Consistency, latency, and split-brain risk amplification.

Compatibility impact:

Likely major and multi-phase migration.

Status: Defer

## P005: Cluster Policy as Code

Type: Non-breaking

Motivation:

Operators need repeatable, reviewable control-plane policy.

Proposal:

Declarative policy spec for placement/failover/maintenance windows.

Expected benefits:

Better governance and change traceability.

Costs:

Policy engine and validation tooling.

Risks:

Policy mistakes affecting cluster stability.

Compatibility impact:

Additive with safe defaults.

Status: Draft
