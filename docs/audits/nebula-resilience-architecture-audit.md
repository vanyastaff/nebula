# nebula-resilience Architecture & Correctness Audit

Scope: `crates/resilience` in the current worktree. This audit reviews architecture,
semantics, cancellation, state machines, API misuse risk, observability, and fit for
Nebula workflow/runtime infrastructure. The findings below capture the audited baseline;
the current branch has since started implementing the P0/P1 corrections, so some evidence
line numbers intentionally describe the pre-fix code that motivated the change.

Fix classification used throughout:

- Patch: small local correction, no API/design impact.
- Refactor: internal structure change preserving public API.
- API correction: public API must change to encode the right invariant.
- Architecture correction: current abstraction is wrong or incomplete.
- Documentation/test only: behavior is correct but not proven or explained.

Implementation status in this worktree:

| Finding | Status | Notes |
|---------|--------|-------|
| F-001 | Addressed | Pipeline retry now treats unknown operation errors as permanent unless a classifier or `retry_if` opts into replay. |
| F-002 | Addressed | Hedging defaults to no duplicates and requires `HedgeSafety::Idempotent` for `max_hedges > 0`; dropped calls abort owned spawned tasks. |
| F-003 | Addressed | `RetryConfig` stores attempts as `NonZeroU32` behind private fields with getters. |
| F-004 | Addressed for retry, pipeline, and key standalone policies | `RetryConfig::total_budget` now uses shared `Deadline` to bound attempts and sleeps; `PolicyContext` can bound a whole pipeline call and context-aware standalone timeout/load-shed/bulkhead/rate-limiter/circuit-breaker/fallback-operation calls. |
| F-005 | Addressed | `build_checked()` rejects unsafe order and `build_recommended_order()` sorts config-driven pipelines. |
| F-006 | Addressed | Half-open has `half_open_success_threshold`, tracks probe successes, and handles ignored half-open timeouts terminally. |
| F-007 | Partially addressed | Default fallback declines cancellation/overload and emits fallback events; `FallbackStrategy::fallback()` now enforces `should_fallback()` before recovery, including chain and priority dispatch; `FunctionFallback` preserves primary+fallback failures with `FallbackFailedWithContext`. Universal source-chain preservation for all custom fallback recoveries remains future API work. |
| F-008 | Partially addressed | `PolicyContext` now groups cancellation, deadline, and scope; `call_with_policy_context*` applies it to pipeline, timeout, load-shed, bulkhead, rate limiter, circuit breaker, and fallback-operation paths. Custom integrations still need coverage. |
| F-009 | Partially addressed | Fallback lifecycle events, scoped `PipelineCompleted`, context-provided scope, and standalone `FallbackOperation` sink events were added; policy id and per-policy duration/reason fields remain future work. |
| F-010 | Addressed | Crate docs were updated for the revised retry, cancellation, fallback, hedge, and composition contracts. |
| F-011 | Partially addressed | Shared `Deadline` now exists, retry uses it, and `PolicyContext` applies it to whole pipeline plus key standalone policy calls; `TimeoutExecutor::try_new()` rejects zero-duration config and zero free-function timeout does not poll; `MockClock` is deterministic rather than leaking real time; full injectable-clock coverage across all policies remains future work. |
| F-012 | Partially addressed | Retry attempts and hedge duplicate safety now encode invariants; exponential backoff sanitizes invalid multipliers; `ConstantLoad` now uses validated construction plus `LoadSnapshot`; broader config hardening remains. |
| F-013 | Partially addressed | Pipeline outcome now preserves fallback success/failure context in telemetry; `CallError` has `FallbackFailedWithContext` plus `fallback_context()`; full source-chain preservation across all custom fallback strategies remains future work. |
| F-014 | Addressed | Added object-safe `ErasedRateLimiter` facade and `PipelineBuilder::rate_limiter_erased()` for heterogeneous tenant/resource registries while preserving the generic `RateLimiter` API. |

Third pass notes:

- Patch: fixed `MockClock` so virtual time advances only through `advance()`, making state-machine
  tests deterministic instead of depending on real elapsed time.
- Patch: hardened exponential backoff against `NaN`, infinite, zero, negative, and shrinking
  multipliers so bad config cannot accidentally collapse retry delay to a hot loop.
- Documentation/test only: added a direct bulkhead cancellation regression test proving that a
  dropped queued acquire releases its queue slot.
- API correction: added `LoadSnapshot` and validated `ConstantLoad::new`; `LoadSignal::snapshot()`
  gives adaptive policies a checked view of external telemetry instead of raw unconstrained `f64`s.

Fourth pass notes:

- API correction: added `CallError::FallbackFailedWithContext { primary, fallback }` and
  `fallback_context()` so fallback failures can carry both the primary failure and the fallback
  failure when both are available.
- Refactor: `map_operation()` and `flat_map_inner()` now traverse nested fallback context instead
  of dropping inner operation errors hidden inside contextual policy errors.
- Refactor: `FunctionFallback` now uses the contextual error variant when its closure fails after
  erasing `Operation(E)` to `Operation(())`.
- Remaining architecture correction: `FallbackStrategy::fallback(error)` is now the safe wrapper,
  but strategy recovery still consumes the primary error by value. Outer wrappers cannot
  universally preserve primary error context for arbitrary custom fallback failures without either
  `E: Clone` or a future typed fallback request/outcome redesign.

Fifth pass notes:

- API correction: added `ErasedRateLimiter`, an object-safe facade for heterogeneous limiter
  registries.
- API correction: added `PipelineBuilder::rate_limiter_erased(Arc<dyn ErasedRateLimiter>)` so
  schema-selected tenant/resource policies can be composed without bespoke closure glue.
- Kept the existing `RateLimiter` trait unchanged for static dispatch and direct use.

Sixth pass notes:

- Architecture correction: added `PolicyContext`, a shared execution context carrying
  cancellation, whole-call deadline, and low-cardinality `PolicyScope`.
- API correction: added `ResiliencePipeline::call_with_policy_context()` and
  `call_with_policy_context_and_fallback()` so Nebula engine can pass one execution contract
  through primary and fallback paths.
- Documentation/test only: added tests proving a context deadline bounds a pipeline without a
  timeout step, bounds an in-flight fallback, and context scope overrides builder scope for
  `PipelineCompleted`.

Seventh pass notes:

- API correction: added `PolicyContext` entry points to standalone `Bulkhead`, `RateLimiter`,
  `ErasedRateLimiter`, `CircuitBreaker`, and `FallbackOperation`.
- Architecture correction: context deadlines/cancellation now release bulkhead queue slots and
  operation permits, bound rate-limited operations, avoid tripping breakers on cancellation, record
  context deadline as circuit timeout, and bound standalone fallback execution.
- Documentation/test only: added regression tests for each standalone context path.

Eighth pass notes:

- API correction: split fallback into a safe `FallbackStrategy::fallback()` wrapper and
  strategy-specific recovery. The safe wrapper always checks `should_fallback()` first.
- Architecture correction: `ChainFallback` and `PriorityFallback` now call nested strategies
  through the safe wrapper, so fallback-side cancellation, overload, or contextual fallback
  failure is not silently recovered by a later fallback unless a custom strategy explicitly
  opts into that error class.
- Documentation/test only: added regressions proving direct value fallback and priority fallback
  decline cancellation, and updated the public docs to describe recovery vs safe fallback entry
  points.
- Architecture correction: standalone `FallbackOperation` now accepts `with_sink()` /
  `with_shared_sink()` and emits `FallbackAttempted`, `FallbackSucceeded`, and `FallbackFailed`,
  so fallback recovery outside a pipeline no longer has to look like primary success.

Ninth pass notes:

- API correction: added `Gate::close_with_timeout()` and `GateCloseTimeout` so shutdown drains can
  fail with typed diagnostics instead of only logging forever; `Gate::active_count()` exposes a
  best-effort diagnostic count.
- Architecture correction: added `load_shed_with_policy_context*()` so cancellation/deadline wins
  before load-shed predicate evaluation and while the operation is in flight.
- API correction: added `timeout_with_policy_context*()` and
  `TimeoutExecutor::call_with_policy_context()` so standalone timeout composes with the workflow
  execution context. `TimeoutExecutor::try_new()` rejects zero-duration config; a zero-duration free
  timeout is now an immediate timeout that does not poll the protected future.

## Executive Summary

`nebula-resilience` has the right broad ingredients for a foundational resilience
crate: typed `CallError`, retry, timeout, circuit breaker, bulkhead, rate limiting,
fallback, hedging, cancellation helpers, and a composable `ResiliencePipeline`.
Conceptually, the crate is promising, but it is not yet safe to treat as the default
runtime resilience substrate for Nebula without stricter contracts and guardrails.

Biggest remaining 3 risks:

1. Idempotency is still caller-attested rather than modeled from action/resource metadata.
   Pipeline retry and hedging are safer now, but explicit classifiers or `HedgeSafety::Idempotent`
   can still be wrong for side-effecting operations.
2. `PolicyContext` now exists for pipeline and the major standalone policy calls. Custom
   limiter/fallback integrations can still ignore it, so full cancellation/deadline propagation
   depends on choosing the context-aware entry points.
3. Telemetry has improved but is not yet production-complete. `PipelineCompleted` carries scope and
   final outcome, but per-policy identity, duration, decision reason, and low-cardinality labels are
   not uniform across events.

Verdict: do not build Nebula's runtime defaults directly on top of the current public
API. It is acceptable as a lower-level toolkit for expert callers who always configure
classification, idempotency, cancellation, shared policy instances, and telemetry
outside the crate. To become foundational, it needs semantic tightening, a deadline and
cancellation model, safer builders, stronger circuit breaker recovery semantics, and
workflow-scoped observability.

## Critical Findings

| ID | Severity | Area | Problem | Failure Scenario | Fix Class | Recommended Fix |
|----|----------|------|---------|------------------|-----------|-----------------|
| F-001 | Critical | Retry / API safety | Pipeline retry treats every operation error as transient by default when no classifier is set. | A non-idempotent action returns a permanent validation error; the pipeline repeats it until `max_attempts`, duplicating side effects if the error happened after a partial commit. | Architecture correction | Make replay explicit: require classifier/idempotency policy for retrying `Operation(E)`, or default unknown operation errors to permanent in pipeline builders. |
| F-002 | Critical | Hedging | Hedging duplicates user operations concurrently without an idempotency/cancel-safety contract. | A Slack/webhook/payment-like action is hedged; primary succeeds slowly while hedge also sends the side effect. | Architecture correction | Gate hedging behind an idempotent operation marker or explicit `HedgeSafety` config; document and enforce non-use for unsafe actions. |
| F-003 | Critical | Retry config invariant | `RetryConfig::max_attempts` is public and can be mutated to `0`; release builds then return `Cancelled` without executing the operation. | Schema-loaded or user-mutated config sets `max_attempts = 0`; workflow engine records cancellation instead of config error or retry exhaustion. | API correction | Make config fields private or revalidate in `retry_with`; represent attempts with `NonZeroU32` in the public API. |
| F-004 | High | Deadline semantics | `RetryConfig::total_budget` is not a real deadline and does not cancel long-running attempts. | A database operation hangs for minutes with a 5s retry budget; retry never wakes to enforce the budget. | Architecture correction | Introduce a `Deadline` object and wrap each attempt plus sleep with remaining time and cancellation. |
| F-005 | High | Composition | `PipelineBuilder::build()` allows harmful policy order and only warns for two cases. | `retry(rate_limiter(operation))` retries rate-limit rejections across a fleet, amplifying overload. | Architecture correction | Add validated policy profiles or return `Result<ResiliencePipeline, ConfigError>` for unsafe orders; make `build_recommended_order` the safe default. |
| F-006 | High | Circuit breaker state machine | Half-open closes on the first successful probe, and ignored timeouts can leave the breaker half-open indefinitely. | A flaky API allows one successful half-open probe, closes the circuit, then all traffic floods back and fails again. | Architecture correction | Model half-open probes explicitly with required success/failure thresholds and terminal handling for ignored outcomes. |
| F-007 | High | Fallback / error model | Fallback attempts all errors by default and can erase the primary failure and operation error type. | Engine shutdown cancellation is converted into cached data, so the workflow appears successful during shutdown. | API correction | Default fallback should decline cancellation/overload unless opted in; preserve original error and emit fallback outcome events. |
| F-008 | High | Cancellation | Cancellation is not first-class in retry, pipeline, rate-limit sleeps, or hedging. | Engine shutdown cancels execution, but retry is sleeping or hedge tasks are running under independent spawned tasks. | Architecture correction | Thread `CancellationToken`/`Deadline` through all policies and test cancellation at each await point. |
| F-009 | High | Observability | Events are too coarse for production diagnosis and can lie when fallback succeeds. | Operators see successful workflow steps but cannot tell they came from fallback after rate limiting, timeout, or open circuit. | Architecture correction | Replace coarse events with structured attempt/outcome/rejection events carrying policy id, scope, reason, duration, and fallback metadata. |
| F-010 | High | Docs / contracts | Composition docs are stale and conflict with current code. | Integrators follow docs saying `BulkheadFull` and `Timeout` stop retrying, while code may retry them depending on classifier setup. | Documentation/test only | Treat docs as part of API contract; update composition docs alongside semantic changes and add doc tests for described behavior. |
| F-011 | Medium | Clock model | Time is fragmented across `Clock`, direct `Instant::now()`, `tokio::time`, and wall-time sleeps. | Tests for retry budgets, fallback TTL, adaptive rate limiting, and hedge latency require real sleeps and miss races. | Architecture correction | Standardize on injectable clock plus Tokio paused-time support and a shared `Deadline` type. |
| F-012 | Medium | Config validation | Several public configs accept invalid or production-dangerous combinations. | Exponential backoff with `NaN` multiplier or huge custom delays produces surprising delays; bulkhead default queues 100 tasks for 30s. | API correction | Move to typed constructors, private fields, validation on all builders, and bounded defaults for workflow use. |
| F-013 | Medium | Error context | `CallError` loses context needed by the workflow engine. | `RetriesExhausted` keeps only the last error; `CircuitOpen` has no breaker name or retry-after; fallback failure hides primary error. | API correction | Introduce structured policy errors with source chain, policy id, scope, retry-after, and attempt history summary. |
| F-014 | Low | Extensibility | `RateLimiter` is not object-safe and custom limiter integration is split between trait and closure API. | A tenant policy registry cannot store heterogeneous limiters as `Arc<dyn RateLimiter>`. | API correction | Decide whether rate limiters are static/generic only or add an object-safe erased trait for registries. |

## Architecture Risks

### F-001: Pipeline Retry Replays Unknown Operation Errors

Severity: Critical

Evidence:

- `PipelineBuilder::classifier` documents that without a pipeline classifier, retry
  treats every operation error as retryable (`crates/resilience/src/pipeline.rs:148`-`155`).
- In `run_retry_step`, an operation error with no per-retry or pipeline classifier maps
  to `ErrorClass::Transient` (`crates/resilience/src/pipeline.rs:699`-`707`).
- `classify_inner` turns `CallError::Operation(error)` into `RetryStepError::Operation`
  and retry handles it through the classifier above (`crates/resilience/src/pipeline.rs:823`-`838`).

What is wrong:

The pipeline's default retry semantics assume "unknown operation error = transient".
That is not a safe foundational default for a workflow engine, because many operation
errors are permanent or may happen after a partial side effect.

Why it matters:

Workflow actions are often authored by integrations. The API makes replay look like a
normal middleware choice, but the most important precondition, idempotency, is not in
the type system and is not required by builders.

Concrete failure scenario:

A webhook delivery action posts to a third-party endpoint, the endpoint persists the
request, then the client receives a connection reset. The action returns an operation
error. The pipeline retries by default, sending duplicate webhooks. A user sees two
workflow-triggered side effects while Nebula telemetry reports a successful retry.

Fix classification: Architecture correction. The retry abstraction is missing a
replay/idempotency contract; this cannot be solved honestly as a local patch.

Suggested design-level fix:

Make replay explicit. Options:

- Default `Operation(E)` to permanent unless an `ErrorClassifier<E>` is configured.
- Add `RetryPolicy::for_idempotent_operations(...)` and make retrying operation errors
  require `Idempotency::Safe` or a classifier.
- Provide Nebula workflow wrappers that bind retry to action/resource metadata:
  `RetryAllowed::Idempotent`, `RetryAllowed::ReadOnly`, `RetryAllowed::ExplicitKeyed`.

Implementation impact: API correction is likely part of the architecture correction,
plus docs and tests.

### F-005: Pipeline Composition Is Advisory, Not Enforced

Severity: High

Evidence:

- `build()` only calls `validate_order` and then builds the pipeline
  (`crates/resilience/src/pipeline.rs:272`-`297`).
- `validate_order` warns only when `timeout` or `rate_limiter` appears inside `retry`
  (`crates/resilience/src/pipeline.rs:330`-`364`).
- `build_recommended_order` sorts to `load_shed -> rate_limiter -> timeout -> retry ->
  circuit_breaker -> bulkhead` (`crates/resilience/src/pipeline.rs:278`-`286`).

What is wrong:

The API lets users create semantically dangerous policy orders and only emits runtime
warnings for two known bad cases. Warnings are easy to miss in production and do not
help schema/config-driven policy assembly.

Why it matters:

Resilience policy composition changes meaning. `timeout(retry(op))` is a total
deadline. `retry(timeout(op))` is per-attempt timeout. `rate_limit(retry(op))` rejects
once before retry. `retry(rate_limit(op))` can retry rate-limit rejections and amplify
overload.

Concrete failure scenario:

During a Slack API incident, hundreds of executions use a user-authored pipeline with
`retry` outside `rate_limiter`. Rate-limit rejections become retryable pattern errors
(`crates/resilience/src/pipeline.rs:838`) and retry delay follows `retry_after` when
available (`crates/resilience/src/pipeline.rs:809`-`817`), but the fleet still runs
retry loops instead of rejecting once. This increases timer load and pressure on the
limiter exactly during overload.

Fix classification: Architecture correction. The composition model is currently too
free-form for a foundational workflow runtime policy system.

Suggested design-level fix:

Expose two paths:

- `build_checked() -> Result<ResiliencePipeline<E>, ConfigError>` for strict validation.
- `build_recommended_order()` or `PipelineProfile::workflow_default()` as the normal
  safe entry point.

For unsafe orders, require explicit `UnsafeCompositionAcknowledgement` or equivalent
named API so misuse is searchable in code.

Implementation impact: API correction for checked builders/profiles, plus docs and
composition tests.

### F-010: Documentation Contradicts Current Semantics

Severity: High

Evidence:

- `crates/resilience/docs/composition.md:23` recommends
  `timeout -> retry -> circuit_breaker -> bulkhead`, while code recommends
  `load_shed -> rate_limiter -> timeout -> retry -> circuit_breaker -> bulkhead`
  (`crates/resilience/src/pipeline.rs:280`-`282`).
- The same doc says non-operation errors like `BulkheadFull` and `Timeout` stop retrying
  (`crates/resilience/docs/composition.md:133`-`144`).
- Current code treats retryable pattern errors as retryable unless an explicit retry
  classifier is present (`crates/resilience/src/pipeline.rs:710`-`714`,
  `:823`-`:839`).

What is wrong:

The docs describe a different contract from the implementation.

Why it matters:

For a resilience library, docs are part of the safety boundary. Users will configure
pipelines from docs, especially when the API permits multiple valid-looking orders.

Concrete failure scenario:

An integration author relies on the docs and places a bulkhead inside retry expecting
`BulkheadFull` to stop retries. In code, `BulkheadFull` is retryable by `CallError::is_retryable`
and can be retried by the pipeline. Under pool exhaustion this turns backpressure into
queued retry timers.

Fix classification: Documentation/test only for the stale documentation itself. If the
underlying composition semantics are changed, that separate work belongs under F-005.

Suggested design-level fix:

Update docs after resolving semantics. Add executable tests that assert every documented
composition example. Treat docs changes as required for any policy composition change.

Implementation impact: docs plus tests; API changes only if F-005 changes the chosen
composition semantics.

### F-009: Observability Cannot Explain Production Outcomes

Severity: High

Evidence:

- `ResilienceEvent` only contains coarse variants such as `RetryAttempt`,
  `TimeoutElapsed`, `RateLimitExceeded`, and `LoadShed`
  (`crates/resilience/src/sink.rs:25`-`56`).
- Events do not carry policy id, tenant/workflow/action/resource scope, operation name,
  attempt duration, final outcome, rejection reason, retry-after, or fallback metadata.
- `PipelineBuilder::with_sink` does not override sinks on pre-built circuit breakers and
  bulkheads (`crates/resilience/src/pipeline.rs:162`-`166`).
- Fallback can return a recovered value without emitting any event
  (`crates/resilience/src/fallback.rs:556`-`570`).

What is wrong:

Telemetry reports that something happened, but not enough about where, why, or what the
final semantic outcome was. It also cannot distinguish "primary succeeded" from
"fallback succeeded".

Why it matters:

In Nebula, operators need to answer runtime questions from traces and metrics alone:
which tenant is failing, which resource breaker opened, whether retries are storming,
whether fallbacks are masking an outage, and whether failures are user errors or
infrastructure protection.

Concrete failure scenario:

A Gmail resource outage causes timeouts, retries, circuit opens, and cache fallback.
Workflow steps appear successful because fallback returned cached data, while operators
only see coarse timeout/retry counters without policy scope or fallback-success events.

Fix classification: Architecture correction. The current observability abstraction is
too narrow for workflow-runtime operation and cannot be fixed by adding one counter.

Suggested design-level fix:

Move from event-kind counters to structured policy outcomes:

- `PolicyId`, `PolicyKind`, `PolicyScope`
- attempt number, duration, delay, retry-after, and classifier decision
- rejection reason and queue/permit stats for bulkhead/rate limiter/load shed
- fallback attempted/succeeded/failed with original primary error kind
- final outcome events for primary success, fallback success, and policy rejection

Implementation impact: API addition/refactor plus docs and observability tests.

## Logic/Semantics Risks

### F-003: RetryConfig Allows `max_attempts = 0` After Construction

Severity: Critical

Evidence:

- `RetryConfig` exposes `pub max_attempts: u32` (`crates/resilience/src/retry.rs:212`-`225`).
- `RetryConfig::new` rejects zero (`crates/resilience/src/retry.rs:248`-`260`), but callers
  can mutate the public field after construction.
- `retry_loop` uses only `debug_assert!` for the invariant (`crates/resilience/src/retry.rs:442`-`445`).
- If the loop executes zero times, it returns `CallError::cancelled()` via the
  `last_err == None` fallback (`crates/resilience/src/retry.rs:505`-`513`).

What is wrong:

An invalid retry config can exist in safe Rust, and the release-mode behavior is a
false cancellation.

Why it matters:

Nebula will likely store policies in schemas or construct them through multiple layers.
If a zero-attempt policy reaches runtime, the operation is skipped and classified as
cancellation instead of configuration failure.

Concrete failure scenario:

A tenant sets `max_attempts: 0` intending "do not retry". A builder creates
`RetryConfig::new(1)` and later overwrites the field from schema. The operation never
runs, and execution shutdown dashboards show cancellations rather than config errors.

Fix classification: API correction. The public config type can represent an invalid
state, so the invariant must be encoded in the API, not patched in one call site.

Suggested design-level fix:

Represent attempts as `NonZeroU32` or a domain type:

- `Attempts::one()`
- `Attempts::at_most(NonZeroU32)`
- `RetryMode::Disabled` for "no retry but one attempt"

Also revalidate in `retry_with` and pipeline `build_checked()` so externally mutated
configs fail closed.

Implementation impact: API change plus internal guard and tests.

### F-004: Retry Budget Is Not a True Deadline

Severity: High

Evidence:

- Docs on `RetryConfig::total_budget` claim the budget includes operation execution and
  sleep time (`crates/resilience/src/retry.rs:219`-`222`).
- Runtime only checks `start.elapsed() + delay > budget` after an attempt has already
  returned an error (`crates/resilience/src/retry.rs:488`-`499`).
- Each operation attempt is awaited directly (`crates/resilience/src/retry.rs:453`).

What is wrong:

`total_budget` limits whether the next sleep is allowed. It does not bound an in-flight
operation attempt and cannot stop a hung operation.

Why it matters:

Workflow engines need hard action deadlines during shutdown, queue lease handling, and
external API calls. A "budget" that waits for a hung attempt is not a runtime deadline.

Concrete failure scenario:

A Postgres operation enters a network stall. The retry config has a 5s total budget,
but the first attempt is awaited for 2 minutes. Nebula misses lease renewal or shutdown
deadlines while retry reports nothing.

Fix classification: Architecture correction. The crate needs a unified deadline model,
not a local elapsed-time check.

Suggested design-level fix:

Introduce a `Deadline` type with:

- monotonic `expires_at`
- `remaining() -> Option<Duration>`
- cancellation token integration
- `sleep_until_or_cancel`
- `run_attempt_or_deadline`

Retry should wrap each attempt and each sleep with the remaining budget. The API should
distinguish "budget exhausted before attempt", "attempt timed out", and "cancelled".

Implementation impact: internal refactor plus API/docs clarification and tests.

### F-006: Circuit Breaker Half-Open Recovery Is Too Weak

Severity: High

Evidence:

- `try_acquire` transitions `Open` to `HalfOpen` and admits one probe immediately
  (`crates/resilience/src/circuit_breaker.rs:722`-`737`).
- `max_half_open_operations` is treated only as concurrent admission limit
  (`crates/resilience/src/circuit_breaker.rs:714`-`720`).
- `record_half_open_success` closes the breaker after one successful probe
  (`crates/resilience/src/circuit_breaker.rs:822`-`829`).
- If `count_timeouts_as_failures == false`, a timeout in half-open only releases the
  probe and returns without state transition (`crates/resilience/src/circuit_breaker.rs:856`-`862`).

What is wrong:

Half-open lacks an explicit recovery state machine. One success closes the circuit even
when more probes were allowed, and ignored timeouts can keep the breaker half-open with
no terminal decision.

Why it matters:

Half-open is where a breaker protects downstream systems during recovery. Closing after
one success is optimistic; remaining half-open after ignored timeouts can create an
ambiguous state that admits sequential probes forever.

Concrete failure scenario:

A third-party API recovers one shard. The first half-open probe succeeds, the breaker
closes, and hundreds of workflow executions flood the API. Most calls fail, causing
breaker flapping and customer-visible latency spikes.

Fix classification: Architecture correction. The recovery state machine lacks the
states and thresholds needed to represent safe half-open behavior.

Suggested design-level fix:

Make half-open explicit:

- `HalfOpen { in_flight, successes, failures, ignored }`
- configured `half_open_success_threshold`
- configured `half_open_failure_threshold`
- a rule for ignored outcomes: either reopen, remain with max ignored probes, or close
  only after enough successes
- tests for concurrent probe races and ignored timeout behavior

Implementation impact: internal state-machine refactor plus API config addition and tests.

### F-012: Config Validation Is Incomplete

Severity: Medium

Evidence:

- `BackoffConfig::Exponential` accepts any `f64` multiplier and computes
  `base.as_millis() as f64 * multiplier.powi(...)`
  (`crates/resilience/src/retry.rs:80`-`87`, `:141`-`:148`).
- `RetryConfig::backoff`, `jitter`, and `total_budget` setters do not validate
  combinations (`crates/resilience/src/retry.rs:263`-`282`).
- `BulkheadConfig::validate` only checks `max_concurrency > 0`
  (`crates/resilience/src/bulkhead.rs:61`-`73`).
- `CircuitBreakerConfig::validate` checks several bounds but not all finite/cap/window
  relationships (`crates/resilience/src/circuit_breaker.rs:101`-`135`).

What is wrong:

Configuration objects encode important runtime invariants as public, loosely validated
fields. Several invalid or operationally dangerous combinations are representable.

Why it matters:

Nebula will likely deserialize policy configs. Config invalidity should be caught at
schema load time, not during an incident.

Concrete failure scenario:

A user supplies exponential retry with `multiplier = NaN`. Delay calculation casts an
invalid floating-point value into a duration path. Even when it does not panic, behavior
is not an intentional policy. Another user sets bulkhead queue to a huge value and
turns load-shedding into unbounded latency.

Fix classification: API correction. The public configuration surface must stop encoding
invalid or production-dangerous states as ordinary values.

Suggested design-level fix:

Use private fields plus validated constructors. Add domain types such as
`PositiveDuration`, `FiniteMultiplier`, `BoundedQueueSize`, `RateThreshold`, and
`Attempts`. Treat deserialization as "raw config -> validated config".

Implementation impact: API change plus validation tests.

### F-011: Time and Clock Model Is Fragmented

Severity: Medium

Evidence:

- Retry uses direct `std::time::Instant::now()` for budget measurement
  (`crates/resilience/src/retry.rs:449`).
- Rate limiter implementations use direct `Instant::now()` in several state updates
  (`crates/resilience/src/rate_limiter.rs:277`, `:324`, `:456`, `:653`, `:827`).
- Fallback cache entries use direct `std::time::Instant::now()`
  (`crates/resilience/src/fallback.rs:279`).
- Hedging measures adaptive latency with direct `std::time::Instant::now()`
  (`crates/resilience/src/hedge.rs:353`).
- A `Clock` abstraction exists, but it is not used consistently outside selected
  circuit breaker paths (`crates/resilience/src/clock.rs:50`-`99`).

What is wrong:

The crate has more than one time model: direct monotonic instants, Tokio timers, a local
`Clock` abstraction, and real sleeps in tests. This prevents deterministic reasoning
about deadlines and state transitions.

Why it matters:

Resilience correctness is time-dependent. Retry budget exhaustion, circuit reset,
fallback TTL, adaptive rate windows, and hedge delays should be testable without real
time sleeps and should share one deadline/cancellation contract.

Concrete failure scenario:

A test tries to prove that adaptive rate limiting decreases after a one-minute error
window, or that fallback TTL expires exactly after a configured duration. Because the
code uses real `Instant::now()` in multiple places, tests either sleep in real time or
skip edge cases, leaving production timing bugs untested.

Fix classification: Architecture correction. Time is a cross-policy semantic dependency,
so the crate needs one time/deadline abstraction rather than scattered local calls.

Suggested design-level fix:

Standardize on a crate-wide time contract:

- use `Clock` for state-machine time
- use Tokio paused time for async sleeps
- introduce `Deadline` for cross-policy remaining budget
- avoid direct `Instant::now()` outside `SystemClock`
- provide deterministic test clocks for retry, fallback, rate limiter, hedge, and circuit
  breaker tests

Implementation impact: internal refactor plus test infrastructure and docs.

## Async/Cancellation Risks

### F-002: Hedging Duplicates Side Effects Without Safety Contract

Severity: Critical

Evidence:

- `HedgeConfig::default` sends up to 2 duplicate requests after 50ms
  (`crates/resilience/src/hedge.rs:64`-`72`).
- `HedgeExecutor::call` spawns the primary operation and later spawns duplicate
  operations from the same closure (`crates/resilience/src/hedge.rs:184`-`222`).
- It returns the first successful result and aborts remaining tasks
  (`crates/resilience/src/hedge.rs:199`-`210`).
- The type signature only requires `F: Fn() -> Fut + Send + Sync`
  (`crates/resilience/src/hedge.rs:177`-`183`); it does not require idempotency,
  request keys, or cancellation safety.

What is wrong:

Hedging is exposed as a general executor, but it is only safe for idempotent,
duplicate-safe operations. The API does not encode that precondition.

Why it matters:

Hedging is more dangerous than retry because attempts overlap. Aborting the loser does
not undo side effects that already reached a downstream service.

Concrete failure scenario:

A workflow action sends a Slack message. The primary request is slow but succeeds. A
hedge fires and also succeeds before the primary result is observed. The workflow step
returns one message ID while the channel receives two messages.

Fix classification: Architecture correction. Hedging needs an idempotency and
duplicate-safety abstraction; adding a local warning would not make it safe.

Suggested design-level fix:

Introduce an explicit safety gate:

- `HedgeExecutor::for_idempotent(config, idempotency: IdempotencyKeyPolicy)`
- or an `IdempotentOperation` wrapper/marker created by workflow/resource layers
- default `max_hedges = 0` in workflow profiles unless a resource/action declares
  duplicate safety

Also record all hedge outcomes, not only `HedgeFired`.

Implementation impact: API correction is likely part of the architecture correction,
plus docs and tests.

### F-008: Cancellation Is Not Composed Through Policies

Severity: High

Evidence:

- `CancellationContext` has standalone `call` and `call_with_timeout`
  (`crates/resilience/src/cancellation.rs:109`-`157`).
- Retry sleeps with `tokio::time::sleep(delay).await` without a cancellation branch
  (`crates/resilience/src/retry.rs:498`-`499`).
- Pipeline timeout wraps an inner future with `tokio::time::timeout`, but pipeline has no
  cancellation token (`crates/resilience/src/pipeline.rs:595`-`603`).
- Hedging uses `JoinSet` and spawned tasks but no caller-owned cancellation/deadline
  parameter (`crates/resilience/src/hedge.rs:184`-`234`).

What is wrong:

Cancellation exists as a helper but not as a policy-wide contract. Callers must wrap
policies manually and the semantics differ depending on wrapper order.

Why it matters:

Nebula needs shutdown cancellation, workflow cancellation, tenant pause/cancel, and
lease-loss cancellation to stop promptly and consistently.

Concrete failure scenario:

During engine shutdown, a long retry delay is sleeping. Unless the caller wrapped retry
with `CancellationContext` outside the policy, shutdown waits for the sleep. If the
caller wraps only the operation, retry may still schedule the next attempt after the
operation observed cancellation.

Fix classification: Architecture correction. Cancellation must be part of policy
execution context, not an optional outer helper.

Suggested design-level fix:

Add `ExecutionContext` or `PolicyContext` carrying cancellation, deadline, scope, and
telemetry. Every policy should use it for sleeps, waits, spawned tasks, and attempts.
Tests should cancel at each await point: before acquire, during acquire, during
operation, during retry sleep, during hedge delay, and during fallback.

Implementation impact: API and internal refactor plus cancellation tests.

### Hedge Cancellation Safety Is Ambiguous

Severity: Medium

Evidence:

- `HedgeExecutor::call` says "Not cancel-safe" (`crates/resilience/src/hedge.rs:174`-`176`).
- The implementation stores tasks in a local `JoinSet`; dropping a `JoinSet` should abort
  tasks in Tokio's current behavior, but the crate contract does not state or test this.

What is wrong:

The contract and implementation intent conflict. If cancellation is unsafe, the API must
make that explicit and hard to use in runtime paths. If it is safe through `JoinSet`
drop behavior, the docs are stale and tests should prove it.

Why it matters:

Spawned tasks outliving workflow cancellation can leak side effects and runtime work.

Concrete failure scenario:

Nebula cancels a workflow step while a hedge is in flight. If a task outlives the future,
it may finish a downstream call after the engine has marked the step cancelled.

Fix classification: Documentation/test only unless tests prove leaked tasks. The current
claim is an unproven/unclear contract; if tests show leaks, it becomes an internal
refactor.

Suggested design-level fix:

Add a cancellation test with a task-side drop/abort signal. Document the proven behavior.
If using `JoinSet` drop behavior is part of the contract, encode it in tests.

Implementation impact: tests/docs, possibly internal refactor if the contract is false.

## API Misuse Risks

### F-007: Fallback Can Hide Primary Failures and Cancellations

Severity: High

Evidence:

- Pre-fix: `FallbackStrategy::should_fallback` recovered every error class by default,
  and `ValueFallback` could be called directly to convert cancellation into a value.
- Current code: `FallbackStrategy::fallback()` checks `should_fallback()` before calling
  recovery, and the default declines `Cancelled`, `LoadShed`, `RateLimited`,
  `BulkheadFull`, and fallback failure.
- Current code: `ChainFallback` and `PriorityFallback` dispatch nested strategies through
  the safe `fallback()` entry point, so later fallbacks do not accidentally recover
  fallback-side cancellation or contextual fallback failures.
- `FunctionFallback` erases `Operation(E)` to `Operation(())`, so fallback code cannot
  inspect the caller error type (`crates/resilience/src/fallback.rs:155`-`166`).
- Standalone `FallbackOperation` now has `with_sink()` / `with_shared_sink()` and emits
  fallback lifecycle events when fallback is actually attempted.

What is wrong:

Fallback is recovery, but recovery success changes the meaning of workflow success.
The most dangerous cancellation/overload default has been fixed; the remaining risk is
that custom recovery still consumes the primary error by value, so generic wrappers cannot
always preserve source context when an arbitrary fallback strategy fails.

Why it matters:

Fallback is dangerous in workflow engines because it changes the meaning of success.
A cached or default value may be acceptable for read paths but invalid for side-effecting
actions, cancellation, or overload.

Concrete failure scenario:

A workflow step is cancelled during engine shutdown. A fallback returns cached account
data and the step is recorded as successful. Downstream workflow steps continue based on
stale data after the engine intended to stop execution.

Fix classification: API correction. The fallback public contract and defaults allow
invalid recovery decisions; the invariant must be represented in the API.

Suggested design-level fix:

Make fallback opt-in by error class and preserve context consistently. Suggested defaults:

- no fallback for `Cancelled`, `LoadShed`, `BulkheadFull`, or `RateLimited` unless explicit (done)
- nested chain/priority fallbacks must honor selected strategy `should_fallback()` (done)
- preserve `primary_error` in a `FallbackOutcome`
- emit `FallbackAttempted`, `FallbackSucceeded`, and `FallbackFailed` for standalone fallback operations, not only pipeline fallback (done)
- keep the original `E` visible to typed fallback functions

Implementation impact: API change plus observability and tests.

### F-013: `CallError` Loses Runtime Decision Context

Severity: Medium

Evidence:

- `RetriesExhausted` stores only `attempts` and the last operation error
  (`crates/resilience/src/error.rs:36`-`42`).
- `CircuitOpen`, `BulkheadFull`, and `LoadShed` are fieldless
  (`crates/resilience/src/error.rs:30`-`33`, `:49`-`:50`).
- `FallbackFailed` has only an optional string reason (`crates/resilience/src/error.rs:56`-`61`).
- `source()` only exposes operation and last retry error (`crates/resilience/src/error.rs:87`-`94`).

What is wrong:

Policy errors are typed but under-contextualized. A workflow engine cannot reliably
decide whether to reschedule, fail permanently, pause a tenant, or mark an action as
infrastructure failure based only on these variants.

Why it matters:

Nebula needs to distinguish user-code failure from policy rejection, transient
infrastructure, tenant throttling, shutdown cancellation, and fallback recovery.

Concrete failure scenario:

A step fails with `CircuitOpen`, but the engine cannot tell which resource or tenant
scope opened, when it may retry, or whether the rejection came from a shared resource
breaker or an action-local breaker.

Fix classification: API correction. Higher-level crates need structured public error
data to make correct decisions.

Suggested design-level fix:

Add structured policy context:

- `PolicyId`, `PolicyKind`, `Scope { tenant, workflow, action, resource }`
- retry-after/opened-at/reset-after for circuit breaker and rate limiter
- compact attempt history for retry
- original primary error preserved through fallback failure

Implementation impact: API change plus telemetry integration.

### F-014: RateLimiter Trait Is Hard To Use In Dynamic Policy Registries

Severity: Low

Evidence:

- `RateLimiter::acquire` returns `impl Future`, and `RateLimiter::call` is generic
  (`crates/resilience/src/rate_limiter.rs:149`-`178`).
- `PipelineBuilder::rate_limiter_from` accepts `Arc<RL>` with a concrete generic type,
  while custom integration uses a closure API (`crates/resilience/src/pipeline.rs:246`-`267`).

What is wrong:

The trait is ergonomic for static generic use, but not object-safe for heterogeneous
runtime registries.

Why it matters:

Nebula may need tenant/workflow/resource policy registries loaded from config. Those
registries often want `Arc<dyn RateLimiterPolicy>`.

Concrete failure scenario:

A policy registry stores token bucket, sliding window, and adaptive limiters for
different tenants. It cannot use one trait object type and must wrap everything in
closures, losing type-level policy identity.

Fix classification: API correction if dynamic registries are a Nebula requirement. If
the crate intentionally supports only static generic limiter composition, this becomes
Documentation/test only.

Suggested design-level fix:

Either document the trait as static/generic only or add an object-safe erased adapter:
`dyn RateLimitPolicy { fn acquire_boxed(&self, ctx) -> BoxFuture<...> }`.

Implementation impact: optional API addition and docs.

## Missing Invariants

| Invariant | Currently encoded in types? | Currently tested? | Risk |
|-----------|-----------------------------|-------------------|------|
| Retry executes at least one attempt. | Yes. `RetryConfig` stores `NonZeroU32` behind private fields and exposes getters. | Yes: constructor, retry behavior, and doctests cover the valid shape. | Closed for current API. |
| Retried operation is idempotent or explicitly classified safe to replay. | Partially. Pipeline defaults unknown operation errors to permanent unless `Classify`/`retry_if` opts in, but idempotency is not a first-class type. | Yes for default non-retry behavior; not for an idempotency marker/profile. | Medium: explicit classifiers can still mark unsafe side effects retryable. |
| Hedged operation is idempotent and safe under concurrent duplicates. | Partially. `HedgeConfig` requires `HedgeSafety::Idempotent` when duplicate hedges are enabled. | Yes: unsafe hedge config is rejected and disabled hedging runs only the primary. | Medium: the marker is caller-attested, not proven by action metadata. |
| Unknown operation errors are not retried unless classified. | Yes in pipeline retry. Unknown `Operation(E)` maps to permanent by default. | Yes: pipeline misuse tests cover unknown operation errors and explicit retry hints. | Closed for pipeline; standalone `retry_with` still follows caller-provided classifier semantics. |
| Total retry budget bounds attempts and sleeps. | Yes for retry, whole-pipeline `PolicyContext`, and major standalone policy calls. `RetryConfig::total_budget` is enforced through `Deadline`; context deadlines bound primary plus fallback paths. | Yes: hung attempt, zero-delay retry, oversized backoff, whole-pipeline deadline, in-flight fallback deadline, timeout context deadline, load-shed context deadline, bulkhead operation deadline, rate-limited operation deadline, and circuit context deadline tests cover budget behavior. | Residual Low/Medium: custom integration paths can still ignore context. |
| Cancellation propagates to every await point. | Partially. `PolicyContext` and cancellation-specific APIs cover pipeline operation execution, major waits, fallback startup, in-flight fallback cancellation, timeout call, load-shed predicate/operation, bulkhead acquire, rate limiter acquire, and circuit breaker calls; hedge drop aborts spawned tasks. | Partial: retry, timeout, load-shed, bulkhead, circuit probe, hedge drop, context fallback, standalone context, whole-pipeline deadline, and pipeline cancellation tests exist. | Medium residual risk for custom limiter/fallback implementations. |
| Pipeline order is semantically valid. | Partially. `build_checked()` rejects non-recommended order and `build_recommended_order()` sorts config-driven pipelines; `build()` remains permissive with warnings. | Yes for checked/recommended builders and known unsafe orders. | Medium: callers can still choose permissive `build()`. |
| Circuit half-open closes only after sufficient recovery evidence. | Yes. Half-open tracks probe successes/failures/ignored outcomes and has a success threshold. | Yes: threshold, failure reopen, timeout handling, and probe-slot release tests exist. | Residual Medium: broader concurrent state-machine stress is still valuable. |
| Ignored circuit breaker outcomes do not leave state ambiguous. | Yes for half-open timeout classification. Ignored half-open timeouts are terminally handled. | Yes: ignored half-open timeout regression test exists. | Closed for identified scenario. |
| Fallback success is distinguishable from primary success. | Partially. Fallback lifecycle events and `PipelineCompleted` outcome distinguish fallback success/failure. | Yes for pipeline sink events; broader external telemetry adapters need coverage. | Medium: event schema still lacks policy id/duration for every policy. |
| Fallback does not run for cancellation/load shedding unless explicit. | Yes for default `ValueFallback`; priority/custom strategies can still choose otherwise. | Yes: default decline behavior is tested. | Low/Medium: custom strategies need documented contracts. |
| Config floats are finite and bounded. | Partially. Backoff multiplier, jitter factor, load snapshots, and token-bucket rate updates are sanitized/validated. | Partial validation tests exist. | Medium: remaining configs need the same constructor discipline. |
| Bulkhead queue size is bounded by workflow profile. | No. | Basic validation only. | Medium: latency and memory pressure. |
| Policy instances are shared at the correct scope. | Partially. `PolicyContext` carries `PolicyScope`, and rate limiters now have an object-safe registry facade, but scope-aware registries are not a full abstraction. | Partial tests cover erased rate limiter registries, builder scope, and context scope in pipeline completion. | Medium: per-action breakers can still fail to protect shared resources. |
| Events carry policy id, scope, reason, duration, and final outcome. | Partially. Scope and final pipeline outcome exist; fallback events and retry/rate-limit hints are richer. | Partial event tests exist. | High residual risk for production diagnosis until per-policy ids, durations, and reasons are uniform. |

## Policy Composition Matrix

| Outer Policy | Inner Policy | Expected Behavior | Risk | Fix Class | Recommendation |
|--------------|--------------|-------------------|------|-----------|----------------|
| Timeout | Retry | One deadline across all attempts. | Safer now: outer timeout, retry `Deadline`, and `PolicyContext` whole-call/standalone timeout deadlines can all bound work. | Architecture correction | Preferred workflow order; use `build_checked()`/`build_recommended_order()` and `PolicyContext` from workflow runtime. |
| Retry | Timeout | Each attempt gets a fresh timeout. | Can exceed total wall-clock expectation unless retry `total_budget` is also set. | API correction | `build_checked()` rejects this order; use only through permissive `build()` when intentionally modeling per-attempt timeout. |
| Rate limiter | Retry | Reject once before entering retry loop. | Correct for protecting downstream quota. | Documentation/test only | Preferred for tenant/API quotas; `rate_limiter_erased()` supports dynamic registries. |
| Retry | Rate limiter | Retry may loop on rate-limit rejection. | Timer amplification and thundering herd after outage, although `retry_after` is now preserved. | API correction | `build_checked()` rejects this order; only use with explicit retry hints and fleet budget. |
| Load shed | Retry | Overload rejects before allocating retry work. | Safe if load shed reflects current executor pressure. | Documentation/test only | Preferred outermost runtime protection. |
| Retry | Load shed | Load shed becomes a retryable/fatal inner error depending semantics. | Can turn overload into retry work. | API correction | Reject in checked workflow profile. |
| Retry | Circuit breaker | Breaker checked per attempt. | Can be valid, but retrying `CircuitOpen` must be tightly controlled. | API correction | Default pipeline classifier blocks inner `CircuitOpen` retries; keep explicit classifiers conservative. |
| Circuit breaker | Retry | Breaker sees final retry result only. | Hides per-attempt downstream failures and delays opening. | Documentation/test only | Use for "operation as a whole" protection only, document semantics. |
| Circuit breaker | Fallback | Breaker records fallback success if fallback is inside breaker. | Outage hidden from breaker; circuit may stay closed. | Architecture correction | Prefer breaker around primary only, fallback outside, with fallback events. |
| Fallback | Circuit breaker | Fallback handles `CircuitOpen`. | Can be acceptable for read-through cache; dangerous if custom fallback accepts cancellation/overload. | API correction | Use filtered fallback strategies; `FunctionFallback` now preserves primary+fallback context. |
| Bulkhead | Retry | One bulkhead permit covers all retries. | Long permit hold starves other executions. | Documentation/test only | Usually avoid for per-attempt external calls. |
| Retry | Bulkhead | Each attempt acquires separately. | Bulkhead rejections may be retried and queue more work. | API correction | Use bounded retry and explicit `BulkheadFull` classification. |
| Timeout | Fallback | Fallback runs only after total timeout. | Fallback may hide deadline miss if callers ignore outcome metadata; `PolicyContext` can bound fallback too. | API correction | Use `call_with_policy_context_and_fallback()` for workflow actions and consume fallback outcome events. |
| Fallback | Timeout | Timeout applies to primary plus fallback. | A slow fallback can consume the action deadline. | Documentation/test only | Use when fallback must be bounded by same action deadline. |
| Cancellation | Any policy | Cancellation should stop waits, sleeps, probes, hedges, fallback. | Improved through `PolicyContext` pipeline and major standalone APIs, but custom integrations can still ignore it. | Architecture correction | Thread `PolicyContext` into custom policy integration contracts next. |
| Hedge | Retry | Concurrent attempts, each possibly retrying. | Explosive fan-out, now gated by `HedgeSafety::Idempotent` but still caller-attested. | Architecture correction | Forbid in workflow profiles unless read-only, bounded, and explicitly idempotent. |
| Retry | Hedge | Retries batches of hedged calls. | Also explosive and side-effect dangerous. | Architecture correction | Forbid for workflow actions unless read-only and bounded. |

## Real Nebula Scenarios

1. Gmail polling action with transient HTTP errors
   - Current design can retry transient errors when `Classify`/`NebulaClassifier` marks
     them transient; unknown operation errors are now permanent by default in pipelines.
   - Risk: an overly broad custom classifier can still retry permanent OAuth/permission
     errors and slow the poll loop.
   - Fix classification: API correction.
   - Recommendation: require `Classify`/`NebulaClassifier`, action-level deadline, and
     polling backoff profile.

2. Postgres resource with pool exhaustion
   - Bulkhead can cap concurrent operations, but `BulkheadFull`/queue timeout context is
     sparse and default queueing waits up to 30s (`crates/resilience/src/bulkhead.rs:51`-`57`).
   - Risk: workflow steps pile up instead of failing fast or rescheduling.
   - Fix classification: Architecture correction.
   - Recommendation: resource-scoped bulkhead with small/no queue, retry classification
     that does not blindly retry pool exhaustion, and telemetry for active/waiting counts.

3. Slack API rate limits
   - Rate limiters now can return `retry_after`, and pipeline preserves it
     (`crates/resilience/src/pipeline.rs:626`-`632`).
   - Risk: if rate limiter is inside retry, rejections become retry-loop work.
   - Fix classification: API correction.
   - Recommendation: rate limiter outside retry with tenant/workspace scope and fleet-wide
     retry budget.

4. Webhook delivery retry
   - Retry can implement exponential backoff and retry hints.
   - Risk: explicit classifiers can still retry a delivery where the downstream committed
     but the response failed.
   - Fix classification: Architecture correction.
   - Recommendation: require idempotency keys or explicit delivery semantics before retry.

5. Long-running workflow step cancelled by engine shutdown
   - `PolicyContext` now carries cancellation/deadline/scope through pipeline calls;
     `call_with_policy_context_and_fallback` prevents fallback from recovering cancellation
     and bounds in-flight fallback by the same deadline.
   - Risk: custom limiter/fallback integrations can still miss the context until their
     contracts require it.
   - Fix classification: Architecture correction.
   - Recommendation: use `PolicyContext` from engine code and require custom integrations to
     honor it.

6. Circuit breaker around flaky third-party API
   - Half-open now uses explicit success thresholds and terminal handling for ignored
     half-open timeouts.
   - Risk: the state machine should still get more concurrent transition stress coverage
     before it protects high-volume shared resources.
   - Fix classification: Architecture correction.
   - Recommendation: resource-scoped breakers, conservative thresholds, and continued
     transition/stress tests.

7. Retrying non-idempotent payment-like operation
   - Pipeline retry no longer replays unknown operation errors by default, and hedging
     requires explicit `HedgeSafety::Idempotent` for duplicates.
   - Risk: explicit classifiers or idempotency attestation can still be wrong for payments.
   - Fix classification: Architecture correction.
   - Recommendation: default no retry/hedge for non-idempotent actions; require idempotency key.

8. Trigger loop with exponential backoff
   - `BackoffConfig` supports exponential and jitter.
   - Invalid multipliers and jitter factors are now sanitized; `MockClock` is deterministic.
   - Risk: not every time-dependent policy is injectable-clock driven yet.
   - Fix classification: API correction.
   - Recommendation: keep property tests and extend clock-driven tests to fallback TTL,
     circuit reset timeout, and adaptive rate limiter windows.

9. Hundreds of executions failing after downstream outage
   - Retry, rate limiter, load shed, and circuit breaker can all help if correctly ordered.
   - Risk: wrong order creates retry storms; observability cannot identify policy scope or
     final outcomes.
   - Fix classification: Architecture correction.
   - Recommendation: Nebula-provided workflow profile with enforced order and aggregate metrics.

10. Fallback returning cached data
   - `CacheFallback` and `ValueFallback` support fallback values.
   - Risk: fallback success is now observable in pipeline events and deadline-bound through
     `PolicyContext`, but callers that only inspect the returned value can still treat fallback
     data as primary data.
   - Fix classification: API correction.
   - Recommendation: typed fallback policies for read-only steps only, with fallback outcome
     events wired into workflow traces.

11. Bulkhead protecting a shared external resource
   - `Bulkhead` uses semaphore permits and RAII release.
   - Risk: if each action creates its own bulkhead instance, no shared protection occurs.
   - Fix classification: Architecture correction.
   - Recommendation: resource-scoped shared policy registry with policy IDs and metrics;
     `ErasedRateLimiter` shows the shape needed for dynamic registries, but bulkhead/circuit
     registries are not yet first-class.

12. Nested resource and action policies
   - Pipeline can compose multiple policies, but does not encode scope.
   - Risk: action-level retry around resource-level retry multiplies attempts invisibly.
   - Fix classification: Architecture correction.
   - Recommendation: max total attempt budget across nested policies and explicit policy tree
     telemetry.

## Test Plan

Fix classification for this section: Documentation/test only, except where a test is
listed as acceptance coverage for a separately classified API or architecture correction.

P0: must be covered before relying on this crate

Coverage already added in this branch:

- `RetryConfig` can no longer represent `max_attempts = 0`.
- Retry `total_budget` cancels hung attempts and bounds zero-delay retry loops.
- Pipeline default retry does not replay unknown operation errors without explicit
  classification or retry hint.
- Hedging duplicate execution is disabled by default and requires `HedgeSafety::Idempotent`.
- Circuit breaker half-open success threshold, failure reopening, ignored timeout, and
  dropped probe release have regression tests.
- Bulkhead queued acquire cancellation, standalone context cancellation/deadline,
  circuit probe release, context fallback cancellation,
  whole-pipeline context deadline, in-flight fallback deadline, and spawned hedge abort tests
  cover the highest-risk resource leaks.
- Fallback default decline for cancellation/overload and fallback outcome/context tests exist.

Remaining P0 gaps:

- Full cancellation/deadline injection at every await point in custom rate limiter/fallback
  integrations outside `ResiliencePipeline`.
- Composition coverage for every documented safe and rejected order, not only representative
  combinations.
- End-to-end observability assertions through the real telemetry/metrics adapters, not only
  `RecordingSink`.

P1: should add soon

- Property tests for backoff and jitter: finite, monotonic where intended, capped, no panic,
  no overflow, no negative/NaN behavior.
- Deterministic clock tests for fallback TTL, retry budgets, circuit reset timeout, adaptive
  rate limiter windows, and hedge latency tracking.
- Stress tests for circuit breaker state under concurrent `try_acquire`/`record_outcome`.
- Bulkhead cancellation tests proving waiting count and permits are released after cancellation.
- Rate limiter tests for retry-after propagation through pipeline and custom limiter closures.
- Tests for nested resource/action retry attempt budget.

P2: nice to have

- Loom tests for circuit breaker and bulkhead state if lock-free or atomic-heavy paths grow.
- Benchmark hot paths for allocation and lock contention under high-cardinality workflows.
- Fuzz schema-to-policy validation for invalid configs.
- Snapshot tests for docs examples and generated telemetry event shapes.

## Recommended Refactor Plan

Phase 1: clarify semantics and invariants

- [Done in this branch][Documentation/test only] Write a formal policy contract doc: attempts, idempotency,
  cancellation, fallback, deadline, circuit breaker recovery, policy scope, and composition order.
- [Done in this branch][Architecture correction] Default pipeline retry semantics for unknown
  operation errors are permanent unless explicitly classified or hinted retryable.
- [Partially done][Architecture correction] Pipeline order now has `build_checked()` and
  `build_recommended_order()`; permissive `build()` remains for callers that intentionally
  want custom order.

Phase 2: fix critical correctness issues

- [Done in this branch][API correction] Make `RetryConfig` fields private or typed; use `NonZeroU32`.
- [Done in this branch][Refactor] Add runtime validation before executing retry and pipeline configs.
- [Partially done][Architecture correction] Introduce `Deadline`/`PolicyContext` and enforce it in retry
  attempts, retry sleeps, whole pipeline calls, fallback paths, timeout/load-shed, and key standalone
  policies. Custom integrations still need context-aware contracts.
- [Done in this branch][Architecture correction] Add explicit half-open thresholds and terminal behavior for
  ignored outcomes.
- [Done in this branch][Architecture correction] Gate hedging behind idempotency/read-only metadata.

Phase 3: improve API misuse resistance

- [Remaining][API correction] Add workflow-safe builders: `WorkflowResilienceProfile`, `ExternalApiProfile`,
  `DatabaseProfile`, `PollingProfile`.
- [Partially done][API correction] Make unsafe composition require an explicit named method.
- [Partially done][API correction] Add fallback policies by error class, with cancellation/overload
  excluded by default.
- [Partially done][Architecture correction] Add scope-aware shared policy registry interfaces for
  resource/tenant/action policies. `ErasedRateLimiter` covers dynamic limiter registries;
  bulkhead and circuit breaker registries remain.

Phase 4: improve observability and docs

- [Partially done][Architecture correction] Replace or extend `ResilienceEvent` with structured events
  carrying policy id, scope, attempt number, duration, decision reason, retry-after,
  and final outcome.
- [Done in this branch][API correction] Emit fallback attempted/succeeded/failed events.
- [Done in this branch][Documentation/test only] Update composition, API reference, and examples to match code.
- [Partially done][Documentation/test only] Add docs tests for semantic examples.

Phase 5: performance cleanup

- [Refactor] Review hot-path allocation in pipeline boxing and event cloning after
  semantics are stable.
- [Done in this branch][API correction] Consider object-safe policy adapters for dynamic registries
  for rate limiters.
- [Architecture correction] Bound metric/log cardinality by using stable policy IDs and
  low-cardinality labels.
- [Documentation/test only] Benchmark high-concurrency retry/rate-limit/bulkhead paths
  with realistic workflow load.

## GitHub Issues

### Issue F-001: Make retrying operation errors explicit and idempotency-aware

Severity: Critical
Fix classification: Architecture correction

Body:

Current pipeline retry maps unknown `Operation(E)` errors to transient when no classifier is
configured (`crates/resilience/src/pipeline.rs:148`-`155`, `:699`-`:707`). This can replay
permanent or partially successful operations.

Failure scenario: a webhook action commits downstream, returns a connection error, and the
pipeline sends duplicate webhooks.

Acceptance criteria:

- Unknown operation errors are not retried by default, or retry requires an explicit
  classifier/idempotency policy.
- Workflow-safe builders expose idempotency-aware retry profiles.
- Tests cover permanent errors, unknown errors, and idempotent transient errors.
- Docs explain replay requirements and anti-patterns.

### Issue F-002: Gate HedgeExecutor behind explicit idempotency/cancel-safety contract

Severity: Critical
Fix classification: Architecture correction

Body:

`HedgeExecutor::call` spawns duplicate operations from `F: Fn() -> Fut` without requiring
idempotency (`crates/resilience/src/hedge.rs:177`-`222`). Default config sends two hedges
(`crates/resilience/src/hedge.rs:64`-`72`).

Failure scenario: a Slack/webhook/payment-like action is hedged and performs duplicate side
effects.

Acceptance criteria:

- Hedging requires explicit duplicate-safety metadata or a workflow-safe profile disables it.
- Docs state that hedging is only for idempotent/read-only operations.
- Tests prove default workflow profile sends no duplicate requests.
- Hedge outcome telemetry records fired, won, failed, and aborted attempts.

### Issue F-003: Prevent `RetryConfig` from representing zero attempts

Severity: Critical
Fix classification: API correction

Body:

`RetryConfig::new` rejects zero, but `max_attempts` is public and mutable
(`crates/resilience/src/retry.rs:212`-`260`). `retry_loop` uses only `debug_assert!`; in release,
zero attempts returns `CallError::cancelled()` without executing the operation
(`crates/resilience/src/retry.rs:442`-`513`).

Failure scenario: schema merge sets `max_attempts = 0`; operation is skipped and recorded as
cancelled.

Acceptance criteria:

- Public config cannot represent zero attempts, preferably via `NonZeroU32`.
- Runtime validates configs before execution.
- Tests mutate/deserialize invalid config and assert config error, not cancellation.

### Issue F-004: Replace retry total_budget with a real deadline

Severity: High
Fix classification: Architecture correction

Body:

`total_budget` claims to include execution and sleep time (`crates/resilience/src/retry.rs:219`-`222`)
but only checks elapsed time before sleeping after an attempt fails
(`crates/resilience/src/retry.rs:488`-`499`).

Failure scenario: first attempt hangs forever despite a 5s total budget.

Acceptance criteria:

- Introduce `Deadline` or equivalent monotonic budget object.
- Each attempt and each sleep are bounded by remaining time.
- Cancellation and budget exhaustion are distinguishable.
- Tests cover hung first attempt, over-budget sleep, and cancellation during sleep.

### Issue F-005: Add checked pipeline composition and safe profiles

Severity: High
Fix classification: Architecture correction

Body:

`PipelineBuilder::build()` only warns for `timeout` or `rate_limiter` inside retry
(`crates/resilience/src/pipeline.rs:330`-`364`). Other surprising orders are accepted.

Failure scenario: retry wraps rate limiter or load shed and converts overload into retry work.

Acceptance criteria:

- Add `build_checked() -> Result<_, ConfigError>` or make `build()` checked.
- Provide safe workflow/resource profiles with enforced order.
- Unsafe order requires explicit named opt-in.
- Composition docs and tests cover all supported combinations.

### Issue F-006: Redesign circuit breaker half-open state machine

Severity: High
Fix classification: Architecture correction

Body:

Half-open closes after one successful probe (`crates/resilience/src/circuit_breaker.rs:822`-`829`).
When timeouts are ignored, half-open timeout only releases the probe and returns
(`crates/resilience/src/circuit_breaker.rs:856`-`862`).

Failure scenario: one lucky success closes the circuit and floods a still-flaky downstream; ignored
timeouts can leave the breaker half-open indefinitely.

Acceptance criteria:

- Half-open state tracks in-flight probes, successes, failures, and ignored outcomes.
- Config supports success/failure thresholds.
- Ignored outcomes have an explicit terminal rule.
- Concurrent transition tests cover races.

### Issue F-007: Make fallback preserve primary failure and avoid cancellation by default

Severity: High
Fix classification: API correction

Body:

Pre-fix, `FallbackStrategy::should_fallback` recovered every error by default and
`ValueFallback` could be called directly to discard cancellation. Current code fixes those
defaults and routes chains/priority dispatch through the safe `fallback()` wrapper, but
`FunctionFallback` still erases `Operation(E)` and arbitrary custom recovery can still return
a fallback failure without a universal primary-error source chain.

Failure scenario: engine shutdown cancellation returns cached data and workflow continues as if
the primary operation succeeded.

Acceptance criteria:

- Fallback defaults exclude cancellation, overload, and policy rejections unless explicit. (done)
- Chain and priority fallbacks honor each selected strategy's `should_fallback()`. (done)
- Fallback result preserves primary error metadata for every strategy, including custom recovery.
- Events distinguish standalone fallback success from primary success, not only pipeline fallback. (done)
- Tests cover cancellation, timeout, circuit open, chain/priority dispatch, and fallback failure source preservation.

### Issue F-008: Thread cancellation through all resilience policies

Severity: High
Fix classification: Architecture correction

Body:

At baseline, `CancellationContext` was standalone and retry sleeps, pipeline timeout,
rate limiter waits, fallback, and hedging did not share a cancellation/deadline context.
This branch introduced `PolicyContext` for pipeline calls, but standalone policy APIs and
custom integrations still need equivalent context-aware entry points.

Failure scenario: workflow shutdown cancels the operation but retry/hedge policy work continues
until timers or spawned tasks finish.

Acceptance criteria:

- Add a shared execution/policy context with cancellation token and deadline. Done for
  `ResiliencePipeline` and the major standalone policy entry points in this branch.
- Every custom policy integration accepts or carries that context.
- Tests cancel or expire deadlines at each await point and assert no leaked permits/tasks/counters.

### Issue F-009: Replace coarse metrics events with structured policy outcomes

Severity: High
Fix classification: Architecture correction

Body:

`ResilienceEvent` has only coarse variants and lacks policy id, scope, reason, duration,
retry-after, final outcome, and fallback events (`crates/resilience/src/sink.rs:25`-`56`).

Failure scenario: workflows appear successful due to fallback, but operators cannot see that the
primary path is timing out or rate-limited.

Acceptance criteria:

- Events include policy id/kind, tenant/workflow/action/resource scope, attempt, duration,
  decision reason, retry-after, and outcome.
- Fallback attempted/succeeded/failed events exist.
- Metrics labels are low-cardinality.
- Tests assert telemetry can distinguish timeout, cancellation, retry exhausted, circuit open,
  bulkhead rejection, rate limit, load shed, fallback success, and primary success.

### Issue F-010: Bring composition documentation back in sync with implementation

Severity: High
Fix classification: Documentation/test only

Body:

`composition.md` recommends an order missing load-shed/rate-limit
(`crates/resilience/docs/composition.md:20`-`34`) and describes an old bail mechanism where
`BulkheadFull` and `Timeout` stop retrying (`crates/resilience/docs/composition.md:133`-`144`).
Current code may retry retryable pattern errors (`crates/resilience/src/pipeline.rs:710`-`714`,
`:823`-`:839`).

Failure scenario: users follow docs and deploy pipelines with retry semantics different from
what they expect.

Acceptance criteria:

- Docs match the chosen implementation semantics.
- Every composition example is backed by a test.
- API reference documents retry-after, classifier interactions, and safe/unsafe orders.
