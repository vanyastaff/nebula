# Roadmap

## R1: Stabilize contracts (short term)

- finalize `ResourceProvider` semantics for typed and dynamic acquire
- document invariants for `Scope::contains` and compatibility checks
- tighten error taxonomy usage (`Unavailable` vs `PoolExhausted` vs `Timeout`)

Exit criteria:
- API docs match behavior 1:1
- no ambiguous state transitions in manager/pool tests

## R2: Production hardening

- complete graceful shutdown test matrix (in-flight, draining, timeout paths)
- increase property/concurrency tests around acquire/release races
- enforce redaction guidance for credential-bearing configs

Exit criteria:
- deterministic shutdown under load
- no leaked instances/semaphore permits in stress tests

## R3: Observability maturity

- standardize event schema and versioning strategy
- publish recommended metrics naming and labels
- provide example dashboards and alert thresholds

Exit criteria:
- health, pool, and failure signals are enough for incident triage

## R4: Runtime integrations

- formal integration contract with action/runtime context injection
- typed resource aliases for common platform resources
- improve docs for tenant/workflow/execution scoping patterns

Exit criteria:
- end-to-end examples from action call to managed resource usage

## R5: Ecosystem expansion

- keep drivers as separate crates (`resource-postgres`, `resource-redis`, etc.)
- avoid duplicating pooling when upstream client already has pool
- add compatibility matrix by driver capability (health, tls, auth, tracing)

Exit criteria:
- clear, minimal, maintainable driver ecosystem around `nebula-resource`
