---
name: credential SecretString wrapper removal — drop custom wrapper, use secrecy::SecretString directly
status: draft (deferred parallel-track — not yet started)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: parallel-track (does not block П3 kickoff; should not start until security-hardening Stage 4 lands)
scope: cross-cutting — nebula-credential, nebula-storage, nebula-engine, all workspace consumers of `nebula_credential::SecretString`
related:
  - docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md (parallel SEC track; SEC-07/SEC-08 subsumed once this lands)
  - docs/tracking/credential-audit-2026-04-27.md §XII Errata (SEC-07, SEC-08)
  - feedback_type_enforce_not_discipline.md (motivation: structural enforcement > runtime sentinel)
defers-to: none
---

# Credential `SecretString` Wrapper Removal

## §0 Meta

**Scope.** Drop `nebula_credential::secrets::SecretString` (custom wrapper around `secrecy::SecretString`). Migrate all call sites to `secrecy::SecretString` directly. Replace custom `serde_secret` module with `secrecy::SerializableSecret` opt-in pattern in storage layer.

**Why.** Current wrapper duplicates most of `secrecy::SecretString` while adding three differences — two of which are **strictly weaker** than secrecy's native behavior:

| Feature | Our wrapper | `secrecy 0.8` native | Strength comparison |
|---|---|---|---|
| Default `Display` | emits `"[REDACTED]"` (succeeds) | not implemented (compile error on `format!`) | **secrecy stricter** |
| Default `Serialize` | emits `"[REDACTED]"` (succeeds) | not implemented (must opt-in via `SerializableSecret`) | **secrecy stricter** |
| `Deserialize` rejects `"[REDACTED]"` sentinel | yes, but too narrow per Errata SEC-07 | not implemented | wrapper unique but flawed |
| `len() / is_empty()` helpers | yes | inline `.expose_secret().len()` | cosmetic |

Per `feedback_type_enforce_not_discipline.md`: secrecy's compile-error approach is structurally stricter than our runtime-sentinel approach. Removing the wrapper aligns the codebase with the «structural enforcement» principle that motivated the SEC-10 fix in the security-hardening spec.

**Non-goals.**
- Replacing `secrecy` crate with another. Stay on workspace `secrecy 0.8`.
- Changing zeroize semantics. Both wrappers delegate to zeroize identically.
- Redesigning encrypted-at-rest serialization end-to-end. Storage already has `EncryptionLayer`; this spec keeps that boundary intact.
- Migrating other secret types (`SecretBytes`, `OAuth2Token`, etc.) — separate sub-tasks if needed.

**Trade-offs.**
- **Removed:** «default `Serialize` → `[REDACTED]`» runtime safety net. Replaced by compile-error if anyone accidentally writes `serde_json::to_string(&secret)`. Strictly stricter, but breaks any code path that relied on the `[REDACTED]` sentinel emission as feature.
- **Removed:** `[REDACTED]` Deserialize rejection. Replaced by: don't emit `[REDACTED]` in the first place. Round-trip protection becomes structurally unnecessary.
- **Removed:** `len() / is_empty()` helpers. Call sites become `.expose_secret().len()` — verbose but explicit; expose is the canonical way to read content.

**Phase note.** Parallel-track. Does not block П3 kickoff. Should not start until security-hardening Stage 4 lands (would conflict with that spec's SEC-08 visibility tightening; cleaner to start after).

**Reading order.** §0 (this) → §1 (migration scope inventory) → §2 (stages) → §3 (tests) → §4 (migration/rollout) → §5 (open questions).

## §1 Migration scope inventory (Stage 0 deliverable)

To execute, the following must be known:

1. How many call sites of `nebula_credential::SecretString` exist?
   - Verify command: `grep -rn "nebula_credential::SecretString\|use nebula_credential::secrets::SecretString\|SecretString::new" crates/`
2. How many sites rely on the **default** Serialize-as-`[REDACTED]` behavior (intentional or accidental)?
3. How many sites use `serde_secret` helper module (`#[serde(with = "...")]`) for real-value serialization?
4. How many sites use `len() / is_empty()` helpers?
5. Are there externally-visible API surfaces (`pub use ...::SecretString` from other crates)?

**Stage 0 of execution = workspace audit + investigation report.** The decision tree branches:

- If default `Serialize`-as-`[REDACTED]` is relied on **outside** `serde_secret` opt-in (i.e., real production usage of the runtime sentinel) → need a replacement newtype (likely `RedactedSecret(secrecy::SecretString)` with manual `Serialize` impl).
- If only `serde_secret` opt-in is used for real serialize → straightforward migration to `secrecy::SerializableSecret`.

## §2 Stages

**Stage 0 — Investigation (Day 1).** Workspace audit per §1. Output: written report listing all call-site classes and recommended migration target per class. PR description records the report inline.

**Stage 1 — Replace declarations (Day 2-3).** Workspace-wide find-and-replace `nebula_credential::secrets::SecretString` → `secrecy::SecretString`. Update `Cargo.toml` deps (add `secrecy` direct dep where currently transitive via `nebula-credential`). `expose_secret()` signature unchanged. Default `Display` calls become compile errors → fix sites where Display was relied on (audit each: explicit redaction or remove the format).

**Stage 2 — Migrate serde — option A or B based on Stage 0 outcome.**

- **Option A** (only `serde_secret` opt-in is used for real serialize):
  - Define `nebula_storage::credential::EncryptableSecret(String)` with `impl SerializableSecret for EncryptableSecret`.
  - Storage layer fields requiring encrypted-at-rest serde wrap their `String` in `secrecy::Secret<EncryptableSecret>` instead of `nebula_credential::SecretString` + `#[serde(with = "serde_secret")]`.
  - Drop our `serde_secret` module entirely.

- **Option B** (default Serialize-as-`[REDACTED]` is intentionally relied on somewhere):
  - Define `nebula_credential::secrets::RedactedSecret(secrecy::SecretString)` with manual `Serialize` impl emitting `[REDACTED]`.
  - Migrate the relying call sites to `RedactedSecret`.
  - All other call sites use `secrecy::SecretString` directly.
  - Drop the custom umbrella wrapper which had this behavior baked into all secrets.

**Stage 3 — Drop wrapper files (Day 4).** Delete `crates/credential/src/secrets/secret_string.rs` and `crates/credential/src/secrets/serde_secret.rs`. Update `crates/credential/src/secrets/mod.rs` re-exports. Update `crates/credential/src/lib.rs` prelude. Workspace `cargo check --workspace` must pass.

**Stage 4 — Doc sync (Day 5).** Update MATURITY/UPGRADE_COMPAT/CHANGELOG; flip register rows for SEC-07 + SEC-08 to `decided (subsumed by wrapper removal PR <sha>)`; update audit Errata §XII.E footer.

## §3 Test strategy

**Compile-fail probes (mandatory landing gates):**

- `crates/credential/tests/compile_fail_format_secret.rs` — `format!("{}", secret)` on `secrecy::SecretString` fails with `E0277 Display not implemented`.
- `crates/credential/tests/compile_fail_serde_secret_default.rs` — `serde_json::to_string(&secret)` on `secrecy::SecretString` fails with `E0277 Serialize not implemented`.

**Runtime tests:**

- Existing `zeroize_drop_oauth2_bearer.rs` (from security-hardening Stage 2) must still pass — verifies zeroize semantics unchanged.
- New `secrecy_serializable_secret_roundtrip.rs` — encrypt → store → read → decrypt path through storage's `SerializableSecret` opt-in works for OAuth2State and other secret-bearing structs.

**Workspace check:**
- `cargo nextest run --workspace` post-migration: all existing tests pass.
- `cargo clippy --workspace -- -D warnings` green.

## §4 Migration / rollout

**Breaking changes (active dev mode, semver-major bump on next release):**

- `nebula_credential::SecretString` removed. Callers use `secrecy::SecretString`.
- `nebula_credential::serde_secret` module removed. Encrypted-at-rest fields use `secrecy::SerializableSecret` opt-in pattern (Option A) OR `RedactedSecret` newtype (Option B).
- `format!("{}", secret)` and `serde_json::to_string(&secret)` become compile errors at sites that previously succeeded with `[REDACTED]` runtime sentinel emission.
- Call sites relying on the runtime sentinel must either: (a) explicitly emit `"[REDACTED]"` via custom code, (b) use the new `RedactedSecret` newtype, or (c) intentionally `secret.expose_secret()` (security-lead review required).

**Rollback contract:** Stage 1 (workspace-wide type swap) is invasive — touches many call sites. Minimize regression risk via Stage 0 investigation report. Rollback per stage is straightforward but Stage 1 has the largest blast radius.

## §5 Open questions (resolve in Stage 0)

| Question | Resolution at |
|---|---|
| Does any code outside `serde_secret` opt-in rely on default Serialize-as-`[REDACTED]`? | Stage 0 audit (decides Option A vs B in Stage 2) |
| Does any code rely on Display-as-`[REDACTED]`? (e.g., user-facing error messages) | Stage 0 audit (count + classify) |
| Storage layer pattern: per-state-struct `SerializableSecret` impls, or one umbrella `EncryptableSecret(String)` newtype? | Stage 2 design |
| PR strategy: one PR per stage, or single big PR for the whole migration? | Stage 1 execution choice |
| Are there other workspace crates with their own `pub SecretString` re-export that need updating? | Stage 0 audit |

---

**Spec complete.** Implementation plan to follow if wrapper removal is prioritized post security-hardening Stage 4 landing. Plan path: `docs/superpowers/plans/2026-04-27-credential-secret-string-wrapper-removal.md` (TBD).
