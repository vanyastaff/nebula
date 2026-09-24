# Credential rotation sequence

How a credential refresh or revoke, decided entirely inside `nebula-credential`,
reaches a live resource instance's `Provider::on_credential_refresh` /
`on_credential_revoke` hook without ever handing this crate credential
material or rotation policy. Gated behind the `rotation` cargo feature
(`crate::credential_fanout`) — off by default so the base build pays no eventbus-subscriber
overhead.

These event buses carry ephemeral observations. Delivery may be lost, duplicated,
or reordered; fan-out is not durable revoke authority or an audit log. Persisted
credential state and its owning runtime remain authoritative.


## Material replacement recovery

With `spawn_with_resolver`, `MaterialReplaced` and `Refreshed` are wake hints for an
owner-qualified durable projection. The driver also reconciles live slot metadata
on startup and every 30 seconds, with at most 32 concurrent projections. This
recovers replacement observations lost before subscription, during subscriber lag,
or across driver restart. Scans run separately from event reception.

The production projection stores its credential ID, contract key and owner scope
alongside the slot's accepted material epoch. Derived credential slots preserve
this metadata. Hand-written `HasCredentialSlots` implementations must provide
`credential_slot_metadata` and install through `SlotCell::install_projected` to
participate in reconciliation. Legacy metadata without an owner cannot authorize
a reread and is reported as a failed reconciliation row.

Each projection has a 30-second deadline and a cancellation token cancelled on
timeout or driver shutdown. The target registration is pinned before projection;
the slot also rejects another credential or owner even at a higher epoch. Only a
newer epoch installs a guard and dispatches a hook. Repeated scans are no-ops for
unchanged epochs, including duplicate refresh events arriving before or after a
scan. Unqualified `store` and successful `install_at_material_epoch` writes clear
projection metadata so later scans cannot associate their values with an old
credential. Completed, timed-out, deferred and abandoned hook outcomes stay
distinct in the fan-out result. Reconciliation does not retry accepted hooks or
turn the event bus into durable command authority.

---

## Refresh sequence

`nebula-credential` decides *when* and *how* to refresh; this crate only
delivers the already-completed refresh to every resource row that resolved
the rotated credential.

```text
nebula-credential                nebula-resource
  facade persists                  credential_fanout::driver
  fresh material          ──▶      ResourceFanoutDriver
  emits                            subscribes EventBus<CredentialEvent>
  CredentialEvent::Refreshed
                                     │
                                     ▼
                          ResourceFanoutIndex::dispatch_refresh(cid, mgr, timeout)
                                     │  (per-row, concurrent, independently
                                     │   bounded — one slow row never
                                     │   blocks or fails a sibling)
                                     ▼
                          Manager::refresh_slot_for_identity(key, scope, slot, id)
                                     │
                                     ▼
                          engine swaps the rotated CredentialGuard into the
                          resource's SlotCell (self.<field>_slot() now
                          returns the fresh guard)
                                     │
                                     ▼
                          Provider::on_credential_refresh(&self, slot_name, instance)
                          — rebuild / blue-green swap acting on `instance`'s
                            interior mutability
                                     │
                                     ▼
                          ResourceEvent::SlotRefreshed { key, slot }
                          (or SlotRefreshFailed { .. } on Err/timeout)
```

---

## Revoke sequence

Revoke is **two-phase and cancellation-safe by construction** — the
synchronous taint always completes before any `.await`, so a dropped or
timed-out future can never leave a revoked credential silently servable.

```text
nebula-credential                nebula-resource
  LeaseLifecycle::revoke_for_credential  or  CredentialService::revoke
  emits LeaseEvent::LeaseRevoked
  and/or CredentialEvent::Revoked  ──▶  ResourceFanoutDriver
                                        (dedupes: one logical revoke can
                                         surface on both buses)
                                     │
                                     ▼
                          ResourceFanoutIndex::dispatch_revoke(cid, mgr, timeout)
                                     │
                    ┌────────────────┴─────────────────────┐
                    │ phase 1 — SYNCHRONOUS, before any await │
                    ▼                                        │
        Manager::taint_slot_for_identity(key, scope, slot, id)
          - sets the resource-scoped taint flag
          - bumps the per-row revoke epoch (Pooled: idle entries with a
            stale checkout epoch are evicted, never re-handed-out)
          ⇒ new acquires against this row are rejected from this instant
                    │
                    │ (drain and admitted hook execution have separate
                    │  budgets — the taint above is synchronous)
                    ▼
        Manager::drain_and_revoke(tainted, per_resource_timeout)
          - waits (best-effort) for in-flight leases on *this row only* to
            release — revoking resource A never blocks on unrelated
            resource B's traffic
          - runs Provider::on_credential_revoke(&self, slot_name, instance)
                    │
                    ▼
          ResourceEvent::SlotRevoked { key, slot }
          (or SlotRevokeFailed { .. } on Err/timeout — the row STAYS
           tainted either way; a timed-out hook never un-revokes)
```

Hook admission transfers execution ownership to the cleanup queue. Expiry of
the observer's deadline while the job has not started returns
`SlotDispatchOutcome::Deferred`; it does not cancel the admitted hook.
Once the worker starts, observation follows the independently bounded hook to
its terminal result. Queue abandonment is reported as `Abandoned`, never as
still-running work. Retained-generation cleanup settles separately: a cleanup
failure cannot change a successful author hook into a hook failure.

`RotationOutcome` exposes `success()`, `failed()`, `timed_out()`,
`deferred()`, and `abandoned()` through accessors. These classifications sum
to `dispatched()`; `drain_timed_out()` and `observation_timed_out()` are
orthogonal counts and must not be added to that total. A timed-out drain can
coexist with a successful hook. Deferred observations do not increment terminal
attempt metrics: the queue records the eventual terminal outcome exactly once.
All of these counts are observability signals, not durable audit records.

---

## What the fence guarantees, and where it is tested

- **No new lease is ever handed out on a since-revoked credential**, even
  under maximum adversarial timing (revoke landing mid-checkout, mid-probe,
  or mid-drain):
  - `tests/revoke_recycle_toctou.rs`
  - `runtime::acquire_loop::tests::probe_revoke_mid_probe_destroys_probed_entries_not_redeposited`
    (`src/runtime/acquire_loop/tests.rs`) — a revoke landing while idle entries are
    being health-probed destroys the probed entries instead of re-depositing
    them.
- **The taint is synchronous-before-the-first-await**, so cancellation of
  the subsequent drain/hook observer cannot undo the applied fence
  — see the [`manager`](../src/manager/mod.rs) module doc's "two-phase
  revoke / drain invariant" section for the canonical proof.
- **A slot name that does not match one of the resource's declared
  `#[credential]` slots is rejected before dispatch** (`Error::unknown_credential_slot`),
  so `on_credential_refresh` / `on_credential_revoke` never observe an
  undeclared slot.

---

## See also

- [`recovery.md`](recovery.md) — the separate thundering-herd gate for backend *failures* (not credential rotation).
- [`events.md`](events.md) — the `SlotRefreshed` / `SlotRevoked` / `SlotRefreshFailed` / `SlotRevokeFailed` event catalog entries.
- The crate-root "Guarantees" rustdoc section — the revoke-fence guarantee restated with its enforcing test.
