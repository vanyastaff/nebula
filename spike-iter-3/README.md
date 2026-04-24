# Spike iter-3 — Gate 3 CP5/CP6 dyn-safety validation (reference artefact)

**Status:** reference-only. Preserved spike validating the CP5/CP6 trait
shape (sub-trait capability split × ADR-0035 phantom-shim × SchemeGuard).
Not part of the main workspace; `Cargo.toml` root `[workspace.exclude]`
keeps it out of the main build.

**Origin:**
- Worktree branch: `worktree-agent-afe8a4c6`
- Final commit: `f36f3739`
- Git tag: `spike-iter-3` → commit `f36f3739`

**What was validated** (Gate 3 §15.12.3):

Three credential types exercised:
- `ApiKeyCredential` — static, no sub-traits
- `OAuth2Credential` — `Interactive + Refreshable + Revocable`
- `SalesforceJwtCredential` — `Interactive + Refreshable`

Five questions answered empirically:

| # | Question | Verdict |
|---|----------|---------|
| (a) | `dyn Credential` object-safety under sub-trait split | **NO regression** — `const KEY` blocks `E0038` already in CP4 |
| (b) | Phantom-shim erases `C::Scheme` cleanly | **YES** — 3-assoc-type base ≡ CP4 4-assoc-type behavior |
| (c) | `dyn Refreshable` needs parallel phantom-shim | **YES** — `RefreshablePhantom` chain works; **ADR-0035 amendment 2026-04-24-C applied** (Pattern 4) |
| (d) | Capability-const downgrade path | **N/A** — spec-correct hard break; `C::REFRESHABLE` fails `E0599` |
| (e) | 4+ compile-fail probes fire | **YES (6/6)** — 4 mandatory per §16.1.1 + 2 bonus |

**Secondary finding (§15.7 refinement):** `SchemeGuard<'a, C>` with only
`PhantomData<&'a ()>` does NOT prevent retention. `'a` infers `'static`
without a pinning borrow. Fix: engine passes `SchemeGuard` alongside
`&'a CredentialContext<'a>` with shared lifetime. Refined
`on_credential_refresh` signature documented in Tech Spec §15.7.

**Reproduce:**
```
cd spike-iter-3
cargo check --workspace
cargo test  # 15 tests total (9 integration + 6 compile-fail), all green
```

Rust 1.95.0 (pinned via `rust-toolchain.toml`).

**Layout:**
- `credential-proto/` — CP5/CP6 trait shape (~550 LOC): `Credential` +
  `Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic` sub-traits;
  `AuthScheme`/`SensitiveScheme`/`PublicScheme` dichotomy; `SchemeGuard`
  + `SchemeFactory`; `CredentialRegistry::register` returning
  `Result<(), RegistryError>`.
- `credential-proto-builtin/` — 3 credential impls + phantom portfolio
  (service capability + lifecycle phantoms) (~430 LOC).
- `credential-proto-builtin/tests/integration.rs` — 9 integration tests.
- `compile-fail/tests/ui/*.rs+.stderr` — 6 compile-fail probes.
- `NOTES.md` — full writeup with iterations log + diagnostics verbatim.

**Referenced by:**
- [Tech Spec §15.4 / §15.7 / §15.12.3](../docs/superpowers/specs/2026-04-24-credential-tech-spec.md)
- [ADR-0035 amendment 2026-04-24-C](../docs/adr/0035-phantom-shim-capability-pattern.md)
- `docs/tracking/credential-concerns-register.md` rows
  `gate-spike-iter3-dyn-safety`, `arch-subtrait-phantom-compose-risk`,
  `arch-cp5-spike-validation`, `arch-phantom-shim-convention`.

**Supersedes:** iter-1/2 (`spike-iter-1-2/`) for CP5/CP6 trait-shape questions.
Retain iter-1/2 for history of original phantom-shim canonical-form derivation.
