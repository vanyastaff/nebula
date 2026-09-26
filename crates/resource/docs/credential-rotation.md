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

With `spawn_with_resolver`, `MaterialReplaced` queues an owner-qualified durable
projection outside the event receive loop, while `Refreshed` requests a coalesced
durable scan. On startup and every 30 seconds, the driver reconciles live slot metadata
only when the same credential, slot and exact resource row has a published reverse-index
binding. A `SlotBinding` without a credential ID therefore remains explicitly opted out
of rotation even if its slot contains projection metadata. Direct material dispatch and
reconciliation share one limit of 32 concurrent
credential projections; the permit is released after installation and hook admission,
before hook observation. Together, these paths recover replacement observations lost before
subscription, during subscriber lag, or across driver restart. Ordinary refresh hints
are coalesced by credential ID and scanned independently, while startup, periodic,
or bounded-queue overflow requests a full scan. At most one material scan runs at a time.
Queue-rejected revoke admissions run in a separate periodic task, so a batch of drain or
hook-observation budgets cannot delay unrelated material projection. Slow projections do
not block credential or lease revoke reception.

Every published rotation binding retains its durable owner and credential key.
Material replacement context is retained in the reverse index until projection
succeeds. Its key space is bounded by live credential bindings, so queue overflow
cannot lose the owner and key needed to recover a bound but still-empty slot.
Startup and periodic scans enumerate those bindings directly, including generation-zero
empty slots and metadata-cleared slots. A live reread installs only into an initially
empty or matching projection; a tombstone can still taint a metadata-cleared bound row.

If an owner-qualified reread finds a durable credential tombstone, reconciliation
uses the same terminal path as a revoke observation: it synchronously taints the
exact registered resource row before awaiting its drain and revoke hook. A lost
`Revoked` event therefore cannot leave the old credential-backed resource acquirable.
Physical absence and cross-owner lookups remain indistinguishable to public callers.

The production projection stores its credential ID, contract key and owner scope
alongside the slot's accepted material epoch. Owner metadata excludes interactive
authentication bindings. Derived credential slots preserve
this metadata. A published rotation binding remains the participation authority.
Hand-written `HasCredentialSlots` implementations must provide
`supports_credential_slot_projection` for each participating slot,
`credential_slot_projection` (an atomic generation/metadata snapshot), and
`install_credential_slot_at_generation` plus
`fence_credential_slot_at_generation`, forwarding to the matching `SlotCell`
ports, to participate in reconciliation. Registration rejects a rotation binding when
that complete projection contract is absent. Metadata without an owner cannot authorize a
reread and is reported as a failed reconciliation row; metadata without a published
binding is skipped as an opt-out.

Each projection has a 30-second deadline and a cancellation token cancelled on
timeout or driver shutdown. The target registration is pinned before projection;
the slot also rejects another credential or owner even at a higher epoch. The
observed slot generation is checked under the writer lock before installation
or a tombstone-driven taint,
so a concurrent `store`, `take`, or other transition fences the in-flight
projection. Superseded projections report `ProjectionChanged` without mutation. After I/O,
the exact registration is revalidated under the manager lifecycle admission lock.
Validation, slot installation and synchronous hook admission share this lock with
revoke, row replacement, removal and shutdown; none of these sections awaits
provider I/O or hook completion. Retired or tainted rows reject refresh admission
with a typed error, failure metric and lifecycle event. Terminal slot revoke clears
projection routing metadata, and scans skip tainted rows, so neither is repeatedly
resolved by startup or periodic reconciliation.
Only a newer epoch installs a guard. Hook admission is tracked separately: queue rejection
leaves that installed epoch and slot generation pending, so a later scan retries
admission only while the same projection is live. Unqualified writes invalidate
the pending attempt. Acceptance consumes the pending state before any await, even if the hook later fails or its
observer is cancelled. Repeated scans are no-ops for unchanged, admitted epochs,
including duplicate refresh events arriving before or after a scan. Unqualified
`store` and successful `install_at_material_epoch` writes clear
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

### Same-material blocks: credential suspension

A credential can deny use without changing its material: it needs
reauthentication, a revoke is in flight, or an operation's outcome is unknown
and awaits reconciliation. That is not a rotation, so nothing is re-projected;
instead every row bound to the credential is **suspended**
(`Manager::suspend_credential_row`, reason
`CredentialUnavailableReason::{ReauthRequired, OperationBlocked}`):

- New acquires, `Manager::until_accepting` and `Limited` waits end with
  `ErrorKind::CredentialUnavailable` (retryable; 30 s hint for
  reauthentication, 1 s for an operation block). It never trips the recovery
  gate.
- Every lease admitted since the previous suspension observes
  `ResourceGuard::closing` — the whole admission span, not only the current
  generation (Design CONTRACT "same-material block"; review ADM-N1). A create
  in flight across the suspension is refused at hand-out, even if the row
  reopened meanwhile.
- No instance is built (warmup, min-idle refill), idle entries are kept but
  not health-probed (a probe authenticates), and stale or lifetime eviction
  continues. The resident master and retained owners stay.
- A taint still wins: a tainted row reports `Tainted` and never reopens.

The row **reopens** when the credential is usable again at the same material
(`Manager::reopen_credential_row`, presenting a `CredentialGateTicket`
captured before the credential was observed; a suspension recorded after the
ticket supersedes it). It reuses its physical owners under a fresh admission
generation; leases admitted before the suspension stay closed. A material
advance goes through the ordinary install, which also reopens when the
installer captured a ticket. The public `install_and_refresh_slot_for_identity`
never reopens.

#### Use revision

Every observation carries where it was made (`CredentialObservedAt`): the
material epoch and, from an `Open` credential status, the **use revision** —
the admission epoch the credential backend advances in the same write as every
transition that closes use (a won revoke claim, a refresh sentinel, a
reauthentication flag, any material advance). Two `Open` reads at one use
revision saw no denial between them; a higher one at the same material means
use was closed and reopened in between (Design CONTRACT "old use revision does
not admit"). The row's gate orders observations by `(material_epoch,
admission_epoch)`, per slot:

| Observation | Row suspended | Row admitting |
|---|---|---|
| Denial read with its revision (reauthentication) | floor := that revision; only a **strictly newer** one reopens | suspend at that revision |
| Denial read without one (operation in flight, reconciliation, resolver error) | floor := admitted revision; equal or newer reopens | suspend |
| Usable, older than the admitted revision | `StaleObservation`, nothing changes | `StaleObservation` |
| Usable, at the admitted revision | reopens an unwitnessed floor only | `NotSuspended`, nothing changes |
| Usable, newer, same material | reopens | **`Readmitted`**: fresh generation, predecessor stays open |
| Usable, another material than installed | `StaleObservation` (new material installs) | `StaleObservation` |

The ticket is checked first: a suspension recorded after it was captured
supersedes the reopen whatever the revision. Refusals mutate nothing. A
readmission is the answer to a **missed interval**: an abandoned revoke claim
acquired and lapsed between two scans or activations. Nothing observed the
denial, so nothing was closed; new work is admitted under a fresh generation,
and the leases admitted before stay open and are not rebuilt. That is a
conscious relaxation of "do not revive cancelled units" for a true block that
was missed. A strict manager (below) reads before every acquire, so a block
lasting across any acquire is observed and closes admitted work; only an
interval with no acquire, create, activation or fan-out scan at all is
missed. Readmission is logged (`info`), not published as a `ResourceEvent`.

Witnessed floors come only from reads that carry the revision, and every
backend (the reference store included) advances it when a reauthentication
flag flips or material advances, so a strict floor always clears. A resolver or
observer that reports no revision falls back to the ticket-only rule; one with
a constant revision never readmits. A display rename moves the credential's
aggregate revision only and neither re-registers nor readmits a row.

Who suspends and reopens:

- **Engine activation** re-checks a registered row's credentials on every
  activation, availability before material, and tracks each binding at
  `(material_epoch, admission_epoch)`. A same-material block suspends the kept
  registration and fails the turn; a newer use revision at the same material
  reopens it (or readmits an admitting row) without registering again, while a
  row still suspended after the check (an `Available` read at the denial's own
  revision) fails the turn with the credential error. The gate is asked only
  when the row is suspended or a revision advanced, so a steady row costs no
  gate call. Only a material advance registers again; only a credential that
  can no longer be resolved (revoked, missing, refused) retires the row.
- **The rotation fan-out** (feature `rotation`) re-observes bound rows on its
  30 s scan and on a `CredentialEvent::ReauthRequired` hint (a targeted
  availability scan, which never dispatches the legacy refresh hook to
  context-less bindings). With a resolver exposing
  `CredentialAvailabilityObserver` the check reads the credential's
  operational head only and never decrypts; only an advanced material is
  projected. Every usable observation at the installed material goes through
  the gate (reopen or readmit); without an observer a projection at the
  installed material does the same through the install path.

### Strict per-acquire admission

A manager configured with a credential availability observer
(`ManagerConfig::with_credential_observer`; the worker takes it from its
credential resolver) is **strict**: every new unit of work on a
credential-bound row reads the bound credentials' availability first (Design
CONTRACT: every new credentialed unit reads availability first; no cached
admission; an outage denies). Each row reports its profile
(`CredentialAdmissionProfile`, in the health snapshot and on
`ManagedResourceView`):

| Profile | When | New work on a bound row |
|---|---|---|
| `Unbound` | the resource declares no credential slots | nothing is read |
| `StrictPerAcquire` | the manager has an observer | each acquire, `warmup_pool` and background create pass reads availability first |
| `InterimRowGate` (interim) | no observer | admitted until activation, the fan-out or a caller suspends the row; one warning per manager |

The strict read of one acquire:

1. After the rate-limit wait, outside every lock: capture the row's
   `CredentialGateTicket`, snapshot each bound slot's installed projection
   (an unbound slot is skipped; material without owner-qualified metadata is
   unobservable and refuses), and read every slot concurrently.
2. Reads are **join-next** coalesced per credential lane (credential id,
   owner, contract key): a caller only takes a read issued at or after it
   arrived, at most one read per lane is in flight, and a dropped or
   timed-out leader hands the lane to a waiter. A burst of acquires during
   one read costs one more read. There is no freshness window.
3. Each read is bounded by the caller's deadline and 2 s. A refresh crossing
   the provider boundary is joined — re-read after 25 ms, doubling to 400 ms,
   until the credential crate's 5 s join wait or the deadline.
4. Under `Manager.admission`, after the post-count taint/shutdown re-check,
   the installed material is re-read and each slot decided:

| Slot read | Gate | Acquire |
|---|---|---|
| usable at the installed material, row admitting, revision not newer | — | admitted |
| usable at the installed material, newer revision | readmit (fresh generation, predecessor open) | admitted |
| usable at the installed material, row suspended | reopen under the floors above | admitted if the row admits afterwards |
| usable or refreshing at newer material | — | `Rebinding` (install first) |
| usable at older material | — | `CheckUnavailable` |
| refresh still in flight after the join | — | `RefreshInFlight` |
| reauthentication required | suspend (witnessed) | `ReauthRequired` |
| operation in flight / reconciliation | suspend | `OperationBlocked` |
| absent, another contract | — | `Absent` |
| store or source unavailable, invalid state, timeout | — | `CheckUnavailable` |

Every blocked slot is suspended; the reported reason is the highest of
`Absent` > `ReauthRequired` > `OperationBlocked` > `Rebinding` >
`RefreshInFlight` > `CheckUnavailable`. No refusal takes a recovery-gate
ticket. Background creates (the maintenance refill, the registration
warmup) read once per pass and build nothing unless every slot is usable;
they change no gate state. A strict manager refuses to register a row whose
declared slot lacks the projection port.

**Availability coupling.** Credentialed egress is no more available than the
credential store: while the store or source cannot answer, new credentialed
work is refused (`CheckUnavailable`, retry after 1 s) and nothing is
suspended; work already admitted continues.

**Latency.** Each new credentialed unit costs one secret-free operational-head
read (no decryption), shared by concurrent acquires of the same credential.
The engine's activation re-check and the fan-out keep their roles — installing
new material, retiring terminal credentials, failing a blocked turn early,
cache and cleanup — but the safety of a new call is decided by the acquire's
own read; the 30 s scan interval is not a safety parameter on a strict
manager.

### What suspension does not cover

- **It is cooperative.** Closing a lease stops no work, revokes no borrow and
  rolls nothing back remotely; an already-authenticated session is not
  terminated.
- **A missed interval closes nothing.** A denial that came and went between
  two observations is detected by its higher use revision, but only new work
  is affected (readmission); units admitted before it keep running. On a
  strict manager only an interval with no acquire, create, activation or
  scan at all is missed.
- **Latency on an interim manager.** A row is suspended at the next
  activation of the stored row or at the next fan-out scan (30 s, sooner on a
  `ReauthRequired` event); until then it keeps admitting. A strict manager
  observes the block at the next acquire.
- **A long wait after the read.** An acquire that waits for capacity after its
  read is not read again; a per-call facade re-reads per attempt.
- **Outages decide no suspension.** A credential store or source outage
  changes no gate: an admitting row stays admitting and a suspended row stays
  suspended. On a strict manager new credentialed work is refused while it
  lasts; on an interim one it is admitted.

---

## See also

- [`recovery.md`](recovery.md) — the separate thundering-herd gate for backend *failures* (not credential rotation).
- [`events.md`](events.md) — the `SlotRefreshed` / `SlotRevoked` / `SlotRefreshFailed` / `SlotRevokeFailed` event catalog entries.
- The crate-root "Guarantees" rustdoc section — the revoke-fence guarantee restated with its enforcing test.
