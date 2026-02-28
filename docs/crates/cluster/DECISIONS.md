# Decisions

## D001: Cluster control-plane needs a dedicated crate owner

Status: Adopt

Context:

Distributed scheduling/failover concerns currently have no single ownership boundary.

Decision:

Introduce `nebula-cluster` as authoritative owner for cluster contracts.

Alternatives considered:

- keep cluster logic distributed across runtime/storage layers

Trade-offs:

- pro: clearer contracts and governance
- con: adds central dependency and implementation effort

Consequences:

Cross-crate integration can be standardized and tested.

Migration impact:

Runtime/execution paths will migrate to cluster APIs.

Validation plan:

Contract tests and staged integration rollout.

## D002: Safety-first scheduling with strong consistency for control state

Status: Adopt

Context:

Incorrect placement/ownership leads to duplicates, losses, or split-brain behavior.

Decision:

Control-plane mutations require strong-consistency guarantees (consensus-backed state).

Alternatives considered:

- eventually-consistent best-effort control state

Trade-offs:

- pro: correctness under failure
- con: higher complexity and coordination overhead

Consequences:

Consensus and recovery logic become critical engineering areas.

Migration impact:

Operational tooling and observability must support consensus lifecycle.

Validation plan:

Failure-injection tests and state-machine consistency checks.

## D003: Scheduling strategies are pluggable but contract-bounded

Status: Adopt

Context:

Different workloads require different placement behavior.

Decision:

Support multiple strategies (least-loaded, round-robin, consistent-hash, affinity) behind one stable scheduling contract.

Alternatives considered:

- single hardcoded strategy

Trade-offs:

- pro: flexibility and workload fit
- con: testing and operability complexity

Consequences:

Need strategy observability and predictable fallback behavior.

Migration impact:

Strategy transitions require controlled rollout.

Validation plan:

Strategy-specific determinism and fairness tests.

## D004: Failover/rebalance operations must be idempotent

Status: Adopt

Context:

Distributed failure handling may repeat operations due to retries/timeouts.

Decision:

Design failover/rebalance APIs and state transitions for idempotency.

Alternatives considered:

- one-shot operations with implicit non-repeatable side effects

Trade-offs:

- pro: safer recovery under transient faults
- con: additional state bookkeeping

Consequences:

Operation IDs and transition guards become required.

Migration impact:

Execution ownership records need idempotency metadata.

Validation plan:

Replay/retry scenario test suite.

## D005: Advanced autoscaling deferred after stable baseline

Status: Defer

Context:

Autoscaling and topology adaptation are high-risk before core stability.

Decision:

Implement baseline membership/placement/failover first; defer advanced autoscaling.

Alternatives considered:

- launch with full autoscaling complexity from day one

Trade-offs:

- pro: reduced initial risk
- con: slower path to full elasticity features

Consequences:

Roadmap phases autoscaling after control-plane maturity.

Migration impact:

Additive capability in later phases.

Validation plan:

Phase-gated exit criteria and load testing.
