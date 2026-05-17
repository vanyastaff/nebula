# ADR-0059: Cross-foundation dependency graph + cycle detection

**Status:** Proposed (2026-05-14)
**Tags:** action, resource, credential, registration, validation

## Context

Charter F8: *"Dependencies as a typed graph, validated at registration.
Action → {Resource, Credential}, Resource → {Resource, Credential},
Credential → Credential (derived chains)."*

Slot binding fields create cross-foundation dependencies. Examples:

- `Action` declares `#[require("db")] pool: Handle<PostgresPool>` →
  Action depends on Resource.
- `PostgresPool` (resource) declares `#[require("db_password")] password:
  Handle<PgCredential>` → Resource depends on Credential.
- `OAuth2AccessToken` declares `#[require("refresh_token")] refresh:
  Handle<OAuth2RefreshToken>` → Credential depends on Credential
  (derived chain).

Cycles forbidden. Detection at registration time.

## Decision

### Dependency graph

Engine maintains a **directed graph** of dependencies built from
`#[require]` declarations across registered actions, resources,
credentials.

Nodes: registered instances (`PgCredential::main_db_password`,
`PostgresPool::analytics_pool`, `Action::stripe_charge`).
Edges: dependency relationships per `#[require]` declarations.

### Cycle detection

At registration time (after all `register_*` calls, before engine
starts serving):

1. Build dependency graph from all registered slot bindings.
2. Run Tarjan's SCC algorithm.
3. If any SCC has size > 1 → **cycle detected, registration fails**.

### Diagnostic format

```text
error[NEBULA_DEP_001]: dependency cycle detected
  
  cycle:
    PostgresPool::analytics_pool
      ↓ requires (slot "metrics")
    MetricsCollector::default
      ↓ requires (slot "audit_db")
    PostgresPool::analytics_pool        ← cycle returns here
  
  help: break the cycle by:
    - making MetricsCollector use a separate connection (not analytics_pool)
    - removing the audit_db dependency from MetricsCollector
    - introducing a third resource (e.g. AuditDbPool) that breaks the cycle
  
  cycle visualization: nebula-cli debug deps --workflow my.yaml --format graphviz
```

### Topological initialization order

After cycle check passes, initialize in topological order:

1. **Credentials** with no `#[require]` deps first.
2. Credentials with deps on credentials (derived chain) next.
3. **Resources** with no `#[require]` deps.
4. Resources with deps on resources/credentials.
5. **Actions** registered last (always depend on resources/credentials).

### `on_failure` policy per dependency

Per withoutboats Day 5 evening proposal:

```rust
#[require("metrics", on_failure = "degrade")]
metrics: Handle<MetricsCollector>,
```

Three policies:

| Policy | Behavior on init failure |
|---|---|
| `fail_fast` (default) | Abort dependent's init; propagate error |
| `degrade` | Init dependent without this dep (must be `Option<Handle<T>>`) |
| `defer` | Retry dep init in background; hold dependent requests |

### Type-keyed and string-keyed forms

Per Cart Day 5 evening:

```rust
// Type-keyed (no string):
#[require]
metrics: Handle<MetricsCollector>,    // engine looks up single instance of MetricsCollector type

// String-keyed (current):
#[require("metrics_v2")]
metrics: Handle<MetricsCollector>,   // engine looks up specific instance "metrics_v2"
```

Type-keyed only valid when **single instance of that type** registered;
otherwise compile error suggesting string key. String-keyed mandatory
when multiple instances exist.

## Consequences

### Positive

- Cycles caught at startup, not runtime — operators see cycle in
  deployment logs immediately, not 3 hours into running.
- Diagnostic format integrates with `nebula-cli` for graphviz export
  — operators visualize their resource topology.
- `on_failure` policies give production operators control over
  degraded-mode behavior.
- Type-keyed shortcut for single-instance simple cases.

### Negative

- Cycle detection runs at every `engine.build()` call — O(N+E) cost.
  For 10K-resource deployments still <100ms; for production sizes
  acceptable.
- Topological init means parallel init only within layer — slower than
  fully-parallel naive init. Acceptable trade-off for correctness.

### Neutral

- Cycles in user-defined workflow logic (e.g., LLM agent
  recursively calling itself) — different concern, runtime depth
  limit handles.

## References

- Conference Day 5 evening (CONFERENCE-NOTES.md) — F8 ratification.
- Tarjan SCC algorithm.
- ADR-0044 (resource/credential singular supersession) — established
  R → C dependencies.

## Out of scope

- Hot-reload dependency graph (add new resource without engine
  restart) — backlog.
- Dependency graph visualization in editor UI — separate
  `nebula-editor` concern.
