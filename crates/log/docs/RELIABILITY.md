# Reliability

## Guarantees

- Deterministic startup precedence.
- Writer fanout behavior follows explicit failure policy.
- Hook panics are isolated and do not crash normal emit path.
- Compatibility checks protect config schema evolution.

## Failure behavior by subsystem

| Subsystem | Failure mode | Outcome |
|---|---|---|
| Filter parsing | invalid directive | initialization fails with `LogError` |
| Writer setup | destination unavailable | initialization fails |
| Writer emit | sink fails | behavior depends on destination policy |
| Hook dispatch | hook panic/error | isolated; event path continues |
| Telemetry setup | exporter/layer error | initialization fails |

## Degradation expectations

- Core logging remains available without telemetry features.
- Multi-destination setups can be tuned for resilience vs strictness via policy.

## Known limitations

- Inline hook execution may increase tail latency under heavy hooks.
- Process-global tracing subscriber requires disciplined test init order.

## Validation coverage

- config precedence tests
- writer fanout policy tests
- hook policy tests
- config schema snapshot/compat tests
- doctest examples for public API
