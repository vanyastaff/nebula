# nebula-credential-builtin — Claude Code orientation
> Agent quick-map for `crates/credential-builtin/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** First-party concrete `Credential` impls (`bearer_token`, `shared_key`, `signing_key`) plus `register_builtins()` and the canonical `mod sealed_caps` — so plugin authors depend only on the `nebula-credential` contract.
**Layer:** Business (credential backend) — depends only downward (root CLAUDE.md -> Layered Dependency Map): `nebula-credential`, `-core`, `-schema`, `-error`.

## Commands
- `cargo check -p nebula-credential-builtin`
- `cargo nextest run -p nebula-credential-builtin`  ·  doctests: `cargo test -p nebula-credential-builtin --doc`
- `cargo test -p nebula-credential-builtin --test <name>` — trybuild compile-fail fixtures (dev-dep `trybuild`)

## Key files
- `src/lib.rs` — re-exports the three credentials + `register_builtins`; hosts canonical `pub(crate) mod sealed_caps` (per-capability inner sealed traits)
- `src/registry.rs` — `register_builtins(&mut CredentialRegistry)`: fail-closed on duplicate KEY (Tech Spec §15.6), not first-wins
- `src/bearer_token.rs` — opaque token → `SecretToken`; `State = Scheme` identity projection
- `src/shared_key.rs` — shared-secret credential
- `src/signing_key.rs` — secret key + algorithm → `SigningKey` (HMAC/SigV4/webhook)

## Conventions & never-do
- Each credential is static / non-interactive: all five `plugin_capability_report::Is*` consts are `false`; `resolve()` returns `ResolveResult::Complete` and never refreshes/revokes.
- Concrete types live here so the contract crate's stability surface stays trait-only (Strategy §2.4); generic auth shapes still live in `nebula-credential` — don't move or duplicate them here.
- `sealed_caps` inner traits are crate-private; external crates declare their own `mod sealed_caps` (ADR-0035 §3) — never impl these by hand or pub-export them.
- `register_builtins` is NOT idempotent: re-registering returns `RegisterError::DuplicateKey`. Callers handle the collision; don't add silent skip-if-present.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `nebula-credential::CredentialError` (Provider + `SecretFreeMessage`); no panicking unwrap/expect/panic in lib code (the `.expect()` in `metadata()` is on a const-valid builder).

## See also
- `README.md` — full design + plugin-author onboarding · ADR-0028–0035 · `docs/INTEGRATION_MODEL.md` § Credential
