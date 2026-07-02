# Credential rotation sequence

How a credential refresh or revoke, decided entirely inside `nebula-credential`,
reaches a live resource instance's `Provider::on_credential_refresh` /
`on_credential_revoke` hook without ever handing this crate credential
material or rotation policy. Gated behind the `rotation` cargo feature
(`crate::credential_fanout`, relocated from `nebula-engine` per ADR-0092
step 5) — off by default so the base build pays no eventbus-subscriber
overhead.

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
                                     │   timeout-wrapped — one slow row never
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
                    │ (only the tail below is wrapped in
                    │  tokio::time::timeout — the taint above never is)
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

`RotationOutcome` aggregates `success` / `failed` / `timed_out` counts across
every affected row and is a metrics/observability signal only — not an audit
record.

---

## What the fence guarantees, and where it is tested

- **No new lease is ever handed out on a since-revoked credential**, even
  under maximum adversarial timing (revoke landing mid-checkout, mid-probe,
  or mid-drain):
  - `tests/revoke_recycle_toctou.rs`
  - `runtime::acquire_loop::tests::probe_revoke_mid_probe_destroys_probed_entries_not_redeposited`
    (`src/runtime/acquire_loop.rs`) — a revoke landing while idle entries are
    being health-probed destroys the probed entries instead of re-depositing
    them.
- **The taint is synchronous-before-the-first-await**, so a caller (the
  engine fan-out) that wraps only the drain/hook tail in
  `tokio::time::timeout` can never drop the future before the taint applies
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
