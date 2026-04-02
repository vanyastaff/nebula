# nebula-config — Reliability Reference

## Failure Modes and Recovery

### Validation failure

When a new configuration candidate fails validation (validator rejection),
nebula-config **does not apply** the new config. Instead, it:

1. Logs the validation failure with the full error context.
2. Preserves the last-known-good active snapshot unchanged.
3. Returns the `ValidationRejected` error to the caller.

**Recovery:** Fix the invalid field values in the source (file/env) and
trigger a reload. The service continues operating with the previous config.

### validation failure triggered by validator rejection

Validator rejection occurs when a field value violates a declared constraint
(e.g. an integer below the allowed minimum). The reject-and-preserve behavior
ensures the running service is never left in an inconsistent state.

## Last-Known-Good Preservation

The config manager maintains a **last-known-good** snapshot:

- On startup: the initial validated config becomes the first snapshot.
- On successful reload: the snapshot is atomically updated.
- On failed reload: **preserve last-known-good active snapshot** — the
  snapshot is untouched and the service continues as before.

```
Load candidate → Validate → Pass → Atomic swap → New active config
                          ↓ Fail
                   preserve last-known-good active snapshot
                   Return ValidationRejected error
```

## Interactions with Downstream Consumers

See [INTERACTIONS.md](INTERACTIONS.md) for how downstream consumers should
handle config change notifications and validation failures.
