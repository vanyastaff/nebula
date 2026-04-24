# Spike iter-1/2 — ADR-0035 phantom-shim validation (reference artefact)

**Status:** reference-only. This is a preserved spike used to validate
ADR-0035's phantom-shim capability pattern (Pattern 2 / Pattern 3 dyn-safety
for `CredentialRef<dyn ServiceXBearerPhantom>`). Not part of the main
workspace; `Cargo.toml` root `[workspace.exclude]` keeps it out of the main
build.

**Origin:**
- Worktree branch: `worktree-agent-a23a1d2c`
- Iter-1 commit: `acfec71f` (blanket sub-trait form, first Bitbucket triad
  validation)
- Iter-2 commit: `1c107144` (amended per-capability sealed form, final
  canonical shape per ADR-0035 amendment 2026-04-24-B)
- Git tag: `spike-iter-1-2` → commit `1c107144`

**What was validated:**
- `Credential` trait dyn-safety via `AnyCredential` + `TypeId` registry path
  (Strategy §3.2).
- Pattern 2 phantom-shim: `CredentialRef<dyn BitbucketBearerPhantom>` compiles
  and enforces `Scheme: AcceptsBearer` via blanket-impl chain.
- Per-capability sealed convention (`mod sealed_caps { pub trait BearerSealed
  {} }`) resolves coherence collisions in multi-capability crates (ADR-0035
  §3 amendment 2026-04-24-B rationale).
- 11 integration tests + 7 compile-fail probes (E0277 × 3, E0271 × 2,
  E0599 × 2).
- 4 Criterion perf benches, all ~150× under 1µs budget per Strategy §3.4.

**What was NOT covered (motivated iter-3):**
- Capability sub-trait split (Interactive/Refreshable/Revocable/Testable/Dynamic).
- `SensitiveScheme`/`PublicScheme` dichotomy.
- `SchemeGuard<'a, C>` + `SchemeFactory<C>` lifecycle.
- Fatal duplicate-KEY registration.

These were added in CP5/CP6 and validated by iter-3 (see `spike-iter-3/`).

**Reproduce:**
```
cd spike-iter-1-2
cargo check --workspace
cargo test -p credential-proto-builtin
```

Rust 1.95.0 (workspace-inherited; no dedicated `rust-toolchain.toml`).

**Referenced by:**
- [ADR-0035](../docs/adr/0035-phantom-shim-capability-pattern.md) — canonical
  form + amendments 2026-04-24-B.
- `docs/tracking/credential-concerns-register.md` row
  `arch-phantom-shim-convention`.

**Supersede.** iter-3 (`spike-iter-3/`) supersedes iter-1/2 for CP5/CP6
trait-shape validation. iter-1/2 retained for history of the phantom-shim
canonical-form derivation.
