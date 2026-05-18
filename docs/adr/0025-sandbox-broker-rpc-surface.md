---
id: 0025
title: sandbox-broker-rpc-surface
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [sandbox, plugin, security, credentials, audit]
related:
  - docs/adr/0006-sandbox-phase1-broker.md
  - docs/adr/0022-webhook-signature-policy.md
  - docs/adr/0023-keyprovider-trait.md
  - docs/plans/2026-04-13-sandbox-roadmap.md
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#126-isolation-honesty
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/STYLE.md#6-secret-handling
linear: []
---

# 0025. Sandbox Phase 1 broker — RPC surface and audit posture

## Context

[ADR-0006](./0006-sandbox-phase1-broker.md) landed the **transport**: duplex
line-delimited JSON over UDS / Named Pipe, plugin handshake, host-side
`PluginHandle` cache. Slices 1a (`c6b9d531`), 1b (`f3b6701b`), 1c (`b5723f28`)
are merged. ADR-0006's slice-1d paragraph is one sentence long:

> a host-side `Broker` in `crates/sandbox/src/broker/` will handle inbound
> `rpc_call` envelopes from the plugin for `log.emit`, `credentials.get`,
> `network.http_request`, `time.now`, `rand.bytes`, `cancel.check`, `env.get`,
> `metrics.emit`. All verbs are default-allow with audit log; no
> manifest-declared scope enforcement until Phase 2.

That sentence names the verbs but does not specify the **policy layer**. It
is the layer this ADR codifies. `security-lead` review of ADR-0006 flagged six
open questions:

1. `credentials.get` — what bounds which credential IDs a plugin can request?
2. `network.http_request` — is SSRF prevention in scope for Phase 1, or
   deferred to Phase 2 with the rest of capability enforcement?
3. Audit event schema — what fields, and which are redaction-safe?
4. `PluginSupervisor` reattach — how is plugin identity verified on reconnect?
5. `rand.bytes` — CSPRNG source, or `thread_rng`?
6. `env.get` — allowlist, denylist, or wide-open?

The roadmap (`docs/plans/2026-04-13-sandbox-roadmap.md`) addresses these
obliquely: D1 picks the broker model over raw syscalls; D4 explicitly defers
the `[permissions]` manifest to Phase 2+ in favour of "default-allow with
audit log" plus always-on sanity checks (anti-SSRF, byte cap, timeout); D6
introduces `(plugin_binary_path, credential_scope_hash)` as the process-
identity tuple for reattach; D7 names the sandbox as the sole runtime
gatekeeper, with OS permission granted once to the host. None of these name
the specific enforcement rules per verb. This ADR does.

The Canon context that binds us:

- **[§12.5 secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth)** —
  secret material must not cross a process boundary as plaintext unless it
  is strictly required for the outbound call, and even then it must not be
  materialised in plugin address space.
- **[§12.6 isolation honesty](../PRODUCT_CANON.md#126-isolation-honesty)** —
  process isolation is the boundary; WASM / WASI is an explicit non-goal.
- **[§4.5 operational honesty](../PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities)**
  — do not advertise a capability (permission manifest, scope declaration)
  the engine does not enforce end-to-end.
- **[STYLE.md §6 — Secret handling](../STYLE.md#6-secret-handling)** — the
  log-redaction test helper is the CI gate for "secret never reached a
  tracing span at any level."

The failure mode ADR-0006 alone creates: a plugin calls
`credentials.get { id: "prod-stripe-live-key" }` in a workflow that doesn't
own that credential, and the host obligingly resolves it because "the audit
log will tell us later." Audit-after-the-fact is not a defense for secret
exfiltration or for SSRF to `169.254.169.254`; those must be prevent-not-
detect. That is the gap this ADR closes.

## Decision

### 1. Verb inventory and default policy

The Phase 1 broker serves exactly eight verbs. Each has a minimum enforcement
below "default-allow with audit log."

| Verb                   | Default | Minimum Phase 1 enforcement                                                                                                  |
|---|---|---|
| `log.emit`             | allow   | plugin_id + workflow_id injected by broker (not trusted from plugin); log level mapped through host's level filter.          |
| `credentials.get`      | allow   | plugin may only name slots bound at workflow-config; broker never returns raw secret material (see §2).                      |
| `network.http_request` | allow   | SSRF defenses on resolve, always on (see §3); byte cap (default 10 MiB); per-call timeout (default 30 s).                    |
| `time.now`             | allow   | monotonic clock source; broker does not expose host wall clock offset corrections.                                           |
| `rand.bytes`           | allow   | OS CSPRNG only (see §5); byte cap per call (default 4 KiB); no user-supplied seed accepted.                                  |
| `cancel.check`         | allow   | reads host cancel registry ([ADR-0016](./0016-engine-cancel-registry.md)); no plugin influence on cancel state.              |
| `env.get`              | allow   | per-plugin-process allowlist (see §6); default allowlist is empty.                                                           |
| `metrics.emit`         | allow   | per-plugin namespace prefix enforced by broker; metric name is validated against a character class to prevent tag injection. |

"Default-allow" here is the D4 model: no per-plugin manifest declares these;
the list itself is the surface. A verb not in this table is a protocol error
and the broker closes the stream with `unknown_verb` — this is the closed
default that keeps the surface honest.

### 2. `credentials.get` scoping — scope hash bounds slot space, raw secret never crosses IPC

Per roadmap D6, every plugin process is keyed by
`(plugin_binary_path, credential_scope_hash)`. The **scope hash** is a SHA-256
of the sorted list of credential slot names bound in the workflow-config at
spawn time. On `credentials.get { slot }`:

1. Broker checks `slot` against the per-process scope manifest (not a plugin-
   declared manifest — the workflow-config binding, committed in the host).
   A slot outside the scope returns `CredentialError::UnknownSlot` with no
   side-channel distinguishing "doesn't exist" from "not in your scope".
2. Broker resolves the slot through [`KeyProvider`](./0023-keyprovider-trait.md)
   and returns a **`CredentialRef`**, not plaintext bytes. The ref is an
   opaque handle (`u64` token plus monotonic generation) that the plugin
   passes back on outbound verbs (`network.http_request`'s `Authorization`
   field, future `net.tcp_connect` credential binding). Broker substitutes
   the real secret on the host side at use site.
3. The only verb that returns **resolved secret material** to the plugin is
   the explicit `credentials.get_value { slot }` escape hatch used by
   integrations that must compute a derived value (e.g. AWS SigV4). That
   escape hatch is **not in the Phase 1 verb list above** — it is deferred
   to Phase 2 behind an explicit per-slot `allow_plaintext_fetch` opt-in
   recorded in the workflow-config. Phase 1 plugins either consume the ref
   or the workflow does not run.

This forecloses the "plugin in workflow A reads `prod-stripe-key` bound in
workflow B" class by construction: the scope hash differs, so the plugin
runs in a different process, and its scope manifest lacks the slot entirely.

### 3. `network.http_request` SSRF defenses — prevent, not detect

The broker's reqwest client is configured with a **resolve hook** that runs
on every DNS resolution, before the connection is attempted. The hook
rejects, with the same `NetworkError::ForbiddenDestination` error for all
cases (no side channel):

- loopback — `127.0.0.0/8`, `::1`
- link-local — `169.254.0.0/16`, `fe80::/10` (blocks cloud metadata endpoints)
- RFC 1918 private — `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
- IPv6 ULA — `fc00::/7`
- broadcast / multicast — `224.0.0.0/4`, `ff00::/8`
- unspecified — `0.0.0.0`, `::`
- any resolution that returns **zero** public addresses (belt-and-braces
  against resolver-shimming)

Operators may extend the blocklist via engine config. The allowlist path
(`net.allow_hosts = ["*.company.internal"]`) is **out of scope for Phase 1**
— that is the exact "scope declaration" shape D4 defers to Phase 2. No
operator can whitelist a private range in Phase 1; the minimum posture is
"public internet only."

Additional always-on constraints per roadmap D4 #3:

- **Per-call timeout**: 30 s default, engine-configurable, hard upper bound
  120 s (broker rejects request-level overrides above the ceiling).
- **Response body cap**: 10 MiB default. Broker streams through a capped
  reader; overage is `NetworkError::ResponseTooLarge`, body bytes already
  received are dropped (not forwarded to plugin).
- **Redirect policy**: broker follows up to 5 redirects; each redirect target
  re-runs the SSRF resolve hook (a 200 → 302 → `http://169.254.169.254/` is
  refused on the second resolve).

### 4. Audit event schema — redaction-safe by construction

Every broker RPC emits one structured event to the host EventBus. The schema
is fixed at the broker boundary; plugin authors cannot influence field
contents beyond the whitelisted fields named below. **Delivery is
fire-and-forget**: broker emits the event to the EventBus and returns the
RPC response to the plugin without blocking on delivery. The EventBus owns
downstream durability; any proposal to make broker audit synchronous-on-RPC
must land as a superseding ADR.

```rust
BrokerAuditEvent {
    verb:          &'static str,          // from the fixed verb table
    plugin_id:     PluginId,              // from process identity, not plugin-supplied
    workflow_id:   WorkflowId,            // from execution context
    action_key:    ActionKey,             // current action on the call stack
    started_at:    SystemTime,            // broker-captured, monotonic-derived
    latency_ms:    u32,                   // broker-measured
    outcome:       Outcome,               // Ok | Denied(reason) | Error(class)
    verb_detail:   VerbDetail,            // verb-specific, redacted per rules below
}
```

Verb-specific detail redaction rules (non-negotiable; enforced by the
STYLE.md §6 log-redaction test helper):

- **`credentials.get`** — `verb_detail = { slot_name: String }`. No credential
  key, no resolved value, no `CredentialRef` token. Slot name is safe: it is
  host-authored workflow config, not plugin-influenced.
- **`network.http_request`** — `verb_detail = { method, host, path,
  status_code, response_bytes }`. **Never** query string (tokens bleed there),
  **never** request body, **never** response body, **never** request or
  response headers except `Content-Type` and `Content-Length`.
- **`env.get`** — `verb_detail = { key: String }`. Value never logged. Key
  is safe because the broker only resolves keys in the allowlist (§6), which
  is host-authored.
- **`rand.bytes`** — `verb_detail = { byte_count: u16 }`. Byte content never
  logged.
- **`log.emit`** — `verb_detail = { level, target, message_len: u32 }`.
  **The message itself is forwarded to the host tracing layer, not captured
  in the audit event** — plugin logs already run through the host's existing
  redaction filters. Duplicating the message in the audit event would give
  plugins a second channel to leak data past redaction.
- **`metrics.emit`** — `verb_detail = { metric_name, label_key_count: u8 }`.
  Label values not logged (they are untrusted and high-cardinality).
- **`time.now` / `cancel.check`** — `verb_detail = Empty`.

The log-redaction test helper from `docs/STYLE.md §6` becomes the CI gate:
one test per verb, firing a secret-bearing input (credential value, query
string with a token, env var containing a secret, rand output) and asserting
the full event emission — structured event, tracing span, metrics label —
contains no substring of the secret. The test must live in
`crates/sandbox/tests/broker_redaction.rs` and every new verb adds a row.

### 5. `rand.bytes` — OS CSPRNG, not `thread_rng` drift

The broker serves `rand.bytes` from the `getrandom` crate (`getrandom::fill`,
which routes to `getrandom(2)` / `BCryptGenRandom` / `/dev/urandom`
appropriately). `rand::thread_rng` is **not permitted** as a backing source:
thread_rng is auto-seeded from OS CSPRNG at thread start, but its long-run
guarantees are weaker than a direct-from-OS read and it is easy to
accidentally swap for a seeded PRNG in tests.

No plugin-supplied seed is accepted — the verb shape is `rand.bytes {
count: u16 }` returning `{ bytes: Bytes }` with no `seed` field. A plugin
that needs deterministic randomness for testing uses an in-plugin PRNG; it
does not ask the broker for one. `getrandom` is already transitive (via
`rand` via `uuid` via workflow ID generation), so no new workspace dep.

### 6. `env.get` — per-process allowlist, default empty

The broker stores a **per-plugin-process env allowlist** built at spawn time
from the workflow-config. Default allowlist is empty: a plugin that has not
declared env needs reads zero keys. The broker returns
`EnvError::NotInAllowlist` (indistinguishable from "not set" — no side
channel between "host has `AWS_SECRET_ACCESS_KEY` set but plugin not allowed"
and "`AWS_SECRET_ACCESS_KEY` not set on host").

The allowlist entry shape is `{ key: String, redact_in_audit: bool }`; the
`redact_in_audit` bit is for keys whose **name** is sensitive (rare — an
env var named `STRIPE_LIVE_SECRET` already failed at the name level, but
the case exists for dynamic deployment tooling). Default is `false`; value
is never in the audit event regardless.

This blocks the "host injects a secret into its own env for config loading,
plugin reads it via `env.get`" class. The allowlist must be declared
host-side; plugins cannot request new keys at runtime.

### 7. `PluginSupervisor` reattach identity — triple verification

On engine restart, the supervisor's reattach path (roadmap D6) reads the
persisted `{pid, socket_path, binary_path, credential_scope_hash}` tuple
and tries to reconnect. Before reusing the handle for **any** RPC, the
supervisor verifies all three:

1. **PID still alive and the same process**: on Linux, compare
   `/proc/{pid}/stat`'s starttime with the persisted starttime (not just
   "pid alive"; pids recycle); on macOS, compare `proc_pidinfo` start time;
   on Windows, compare `GetProcessTimes` creation time. Mismatch → respawn.
2. **Binary path hash**: SHA-256 of the binary at `binary_path` must match
   the persisted hash. An attacker who replaces the binary between engine
   restarts and relies on pid reuse loses here. Mismatch → respawn, log a
   `PluginBinaryChanged` audit event at `WARN`.
3. **Credential scope hash**: must match the persisted hash. A workflow-
   config change that alters slot bindings respawns the process; an in-
   flight plugin cannot survive a scope change.

If all three match, the supervisor issues a handshake `ping` over the
existing socket before considering the reattach successful. Any of the
checks failing → kill(pid) → respawn fresh. The "reuse on partial match"
path does not exist.

## Consequences

**Positive**

- Each security invariant in the ADR is citable from the code that upholds
  it: SSRF from the broker resolve hook, credential scoping from the spawn-
  time manifest plus `CredentialRef`, reattach from the triple-check.
  Auditing "is this invariant alive?" is a grep, not an archeology session.
- Plugin authors get a concrete contract: eight verbs, documented redaction
  rules, no hidden manifest. The verb list is the surface.
- Audit surface is **redaction-safe by construction** — the schema simply
  does not carry bodies, values, or secrets. A future "log everything"
  operator toggle cannot accidentally flip that bit because the fields
  aren't there.
- `credentials.get` returning `CredentialRef` not plaintext means a plugin
  memory dump after exit still reveals only a token; the secret only
  materialises on the host.
- Eliminates the "plugin reads cloud metadata service" foot-gun at the
  resolve layer, which is where SSRF defense belongs (pre-connect, not
  post-hoc in the audit log).

**Negative**

- Slice 1d scope grows beyond ADR-0006's one-sentence spec. Engineering
  cost: env allowlist bookkeeping, credential scope hash plumbing, resolve-
  hook wiring, per-verb redaction tests. Estimated 3-5 days on top of the
  existing slice 1d estimate.
- `CredentialRef` substitution on outbound calls is a broker-side
  complication — every outbound verb that takes credential-bearing data has
  to know how to rewrite the ref into the real value. Worth it; the
  alternative is plaintext-in-plugin-memory.
- The "no plaintext credential fetch" Phase 1 posture blocks some
  integration patterns (AWS SigV4 in-plugin signing). Deferred to Phase 2
  behind an explicit opt-in — this is a known trade.
- `env.get` default-empty allowlist means every first-party plugin that
  wants (e.g.) `HTTPS_PROXY` has to declare it. Fine — the declaration is
  one line in the workflow-config and it is the right default.

**Neutral**

- Transport stays exactly as ADR-0006 specifies: line-delimited JSON over
  UDS / Named Pipe, `DUPLEX_PROTOCOL_VERSION = 2`. This ADR only talks
  policy.
- No new workspace dependencies. `getrandom` is already transitive; the
  resolve hook uses `reqwest::Client::builder().dns_resolver(...)` which
  is already in the tree.
- Does not change ADR-0023's `KeyProvider` shape — the broker is a
  consumer of `KeyProvider` via `CredentialRef` resolution, not a peer
  of it.

## Alternatives considered

- **"Default-allow everything, rely on the audit log" (ADR-0006's wording
  taken literally).** Rejected. Audit-after-the-fact is not a defense for
  `credentials.get { id: "any-id" }` returning a secret from another
  workflow: by the time the audit event reaches the EventBus, the plugin
  has already sent the secret over its next `network.http_request`. SSRF
  is the same story — an audit log of "plugin contacted
  169.254.169.254" is an incident report, not a defense. Prevent, don't
  detect, for these two verbs specifically.
- **Per-plugin `[permissions]` manifest in `plugin.toml` (Cargo-style
  capability scope).** Rejected per roadmap D4. Phase 1–4 deliberately does
  not ship this: research in
  `.project/context/research/sandbox-permission-formats.md` found the design
  space unresolved without real community plugins and operator feedback,
  and the roadmap already gives us process isolation + broker + audit +
  signed manifest + OS jail. Re-cited here to close the alternative
  cleanly; the ADR-0025 model is strictly stronger than "default-allow +
  audit" without requiring the manifest.
- **WASM / WASI sandbox as the boundary.** Rejected per Canon §12.6 (sandbox
  non-goal) and roadmap D2 (process isolation, full stop). WASM would
  displace the broker model entirely and is not the Phase 1 direction.
- **Plaintext `credentials.get` by default with a "just don't log it" rule.**
  Rejected. Any verb that materialises a secret in plugin address space is
  a secret-in-memory-dump incident waiting to happen. `CredentialRef` with
  host-side substitution is the only shape that keeps the plaintext out of
  the plugin process.
- **Global `env.get` wide-open with a denylist (inverse of the chosen
  allowlist).** Rejected. Denylists are fundamentally open-world: a new
  secret env var added by an operator next quarter becomes readable until
  someone remembers to add it to the denylist. Allowlist defaults the new
  case to "no."

## Seam / verification

Files that will carry the invariants (none exist yet; this ADR precedes the
implementation):

- `crates/sandbox/src/broker/mod.rs` — broker module root, verb dispatch.
- `crates/sandbox/src/broker/verbs/credentials.rs` — `credentials.get`,
  scope manifest check, `CredentialRef` minting.
- `crates/sandbox/src/broker/verbs/network.rs` — `network.http_request`,
  SSRF resolve hook, byte cap, timeout.
- `crates/sandbox/src/broker/verbs/env.rs` — `env.get`, allowlist.
- `crates/sandbox/src/broker/verbs/rand.rs` — `rand.bytes`, `getrandom`.
- `crates/sandbox/src/broker/audit.rs` — `BrokerAuditEvent` shape and
  per-verb redaction rules.
- `crates/sandbox/src/process.rs` — credential scope hash computation and
  plumbing to `ProcessSandbox::spawn_and_dial` (existing file gets the
  hash plumbing, not the policy).
- `crates/sandbox/src/supervisor.rs` — `PluginSupervisor` reattach triple
  check.
- `crates/sandbox/tests/broker_redaction.rs` — STYLE.md §6 log-redaction
  test helper, one case per verb; CI-gated.
- `crates/sandbox/tests/broker_ssrf.rs` — one case per forbidden range
  (loopback, link-local, RFC 1918, ULA, broadcast, unspecified,
  zero-public-resolve); plus a redirect-to-forbidden case.
- `crates/sandbox/tests/supervisor_reattach.rs` — the pid-recycle case,
  the binary-changed case, the scope-changed case.

CI signals that catch regressions:

- Redaction tests: a new verb with a verb-detail field that carries a
  plugin-controlled string fails the fuzz-style test that injects
  high-entropy tokens through all inputs and greps all outputs.
- SSRF tests: the resolve-hook table is regenerated from `ipnet` constants;
  a PR that weakens the blocklist fails the table-equality test.
- Reattach tests: the triple check has three separate failing cases; a PR
  that collapses them to a single "pid alive" check fails at least two.

## Follow-ups

- **Phase 2 — capability enforcement migration.** Once seccomp + cgroups +
  namespaces land, a subset of these policies moves from broker-enforced
  to kernel-enforced (e.g. SSRF becomes "plugin has no network namespace at
  all, broker is literal gatekeeper"). The broker policy does not go away
  — it becomes defense-in-depth behind the OS jail.
- **Phase 2 — `credentials.get_value` escape hatch.** The per-slot
  `allow_plaintext_fetch` opt-in for integrations that must compute a
  derived value on the plugin side (AWS SigV4 is the canonical case).
  Opens a dedicated ADR when the first real integration needs it; the
  shape is likely "plugin receives a time-limited derived token, not the
  raw secret."
- **Phase 2 — `[permissions]` manifest revisit.** If community plugin
  authors and operators converge on a scope vocabulary richer than
  "default-allow with audit," this ADR becomes the migration target. The
  scope hash mechanism generalises: today it hashes the credential-slot
  list; tomorrow it can hash the full permissions declaration. Open a new
  ADR that `supersedes: [0025]` when that happens — this ADR body stays
  immutable per the project ADR convention.
- **ADR-0006 frontmatter.** Once this ADR is `accepted`, ADR-0006's
  `related:` field gets a pointer to ADR-0025 (frontmatter-only
  maintenance, body immutable). Out of scope for this PR; the author of
  the supersession pointer in ADR-0006 is the ADR-0006 reviewer.
- **`docs/MATURITY.md` sandbox row.** Tighten the wording from "broker
  transport landed; RPC verbs TBD" to reflect the policy surface locked
  in here.
