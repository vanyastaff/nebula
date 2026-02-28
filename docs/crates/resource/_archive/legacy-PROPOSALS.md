# Proposals

## P001: Strongly typed registration keys

Idea:
- add optional typed key wrapper for registration (`ResourceKey<T>`)
- keep string id compatibility for dynamic runtime paths

Benefit:
- fewer id mismatch bugs, clearer compile-time intent.

Potential break:
- if made mandatory, existing string-only registration code breaks.

## P002: Unified resource state machine

Idea:
- formalize instance states (`Created`, `Ready`, `Borrowed`, `Recycling`, `Quarantined`, `Destroyed`)
- expose state transition metrics/events consistently

Benefit:
- easier debugging and correctness auditing.

Potential break:
- event schema and hook contracts may change.

## P003: Back-pressure policy profiles

Idea:
- add explicit acquire policies: `FailFast`, `WaitWithTimeout`, `Adaptive`
- map policies to queue length and timeout behavior

Benefit:
- predictable behavior for high-load workflows.

Potential break:
- default acquire behavior may change if profile defaults are updated.

## P004: Credential refresh hook contract

Idea:
- define first-class hook for pre-acquire credential freshness check
- integrate with `nebula-credential` provider for short-lived tokens

Benefit:
- safer long-running workloads with rotating secrets.

Potential break:
- resources that assume static credentials may need explicit opt-out.

## P005: Config reload without full pool swap

Idea:
- support partial config update categories:
  - runtime-safe (timeouts, limits) applied in place
  - destructive (dsn/auth) requiring staged replacement

Benefit:
- lower disruption for operational tuning.

Potential break:
- `reload_config` semantics and guarantees become more complex.
