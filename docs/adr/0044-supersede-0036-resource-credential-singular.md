---
id: 0044
title: supersede-0036-resource-credential-singular
status: accepted
date: 2026-04-29
supersedes: [0036]
superseded_by: []
tags: [resource, credential, slot-binding, m11, supersession]
related:
  - .ai-factory/plans/m6-resource-finalization-integration-audit.md
  - docs/adr/0042-node-binding-mechanism.md
  - docs/adr/0043-dependency-declaration-dx.md
---

# 0044. Supersede ADR-0036 — Resource::Credential singular → slot fields

## Context

ADR-0036 (`resource-credential-adoption-auth-retirement`, external workspace
`C:\Users\vanya\RustroverProjects\docs\adr\0036-resource-credential-adoption-auth-retirement.md`)
locked a singular `Resource::Credential` associated type:

```rust
impl Resource for Postgres {
    type Credential = DatabaseCredential;
    // … create(config, credential, ctx) …
}
```

The model assumed each Resource binds to at most one credential. Resources
needing additional credentials had to resolve them internally:

```rust
async fn create(&self, config: &PgConfig, primary_cred: &DatabaseCredential, ctx: &dyn Ctx)
    -> Result<PgRuntime, PgError>
{
    let primary_cred = primary_cred;                // singular path (typed)
    let audit_cred = ctx.credential_store()        // secondary path (manual lookup)
        .resolve_erased::<AuditCredential>(...).await?;
    // …
}
```

Per session 2026-04-29 design dialogue (ADR-0043), the dependency-redesign
introduces typed slot fields on Action structs. The user requested **the same
slot-binding pattern for Resource → Credential** to eliminate API divergence:

```rust
#[derive(Resource)]
#[resource(key = "postgres", topology = "pool", config = PostgresConfig)]
struct Postgres {
    #[credential(key = "db_auth")]
    db_auth: CredentialGuard<DatabaseCredential>,

    #[credential(key = "audit", purpose = "Audit log auth")]
    audit: Option<CredentialGuard<AuditCredential>>,
}
```

The original ADR-0036 framing — singular associated type — cannot express
this pattern without trait widening or hidden internal lookups. The right
answer is supersession.

## Decision

**Supersede ADR-0036 in full.** Drop `Resource::Credential` associated type
from the `Resource` trait. Resources declare credential dependencies via
typed slot fields on their struct, parallel to Actions:

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Runtime: Send + Sync;
    type Lease: Send + Sync;
    type Error: ClassifyError + Send + Sync;
    // type Credential — DELETED

    fn metadata() -> &'static ResourceMetadata;
    fn config_schema() -> &'static ValidSchema;
    fn dependencies() -> &'static Dependencies;     // slot info

    async fn create(&self, ctx: &dyn Ctx) -> Result<Self::Runtime, Self::Error>;
    async fn check(&self, runtime: &Self::Runtime) -> Result<(), Self::Error>;
    async fn destroy(&self, runtime: Self::Runtime) -> Result<(), Self::Error>;
}
```

`create` no longer takes an explicit credential argument — slot fields are
already populated on `&self` before the engine calls `create`. The
single-credential case writes one `#[credential]` field; the multi-credential
case writes N fields; the no-credential case writes zero fields and replaces
the previous `type Credential = NoCredential;` opt-out with simple field
absence.

**Per-credential refresh hook.** ADR-0036's singular
`on_credential_refresh(&self, scheme: &<C::Scheme>)` becomes the
slot-aware:

```rust
async fn on_credential_refresh(&mut self, slot_name: &str) -> Result<(), Self::Error>;
```

Manager fan-out invokes the hook with the slot name that rotated; the
resource implementation pattern-matches on `slot_name` to decide whether to
recycle pool entries, refresh cached headers, or take a different action per
slot.

The supersession is a **hard break** per `feedback_hard_breaking_changes.md`
and `feedback_no_shims.md`. No deprecated `type Credential` aliases. Codemod
migrates existing Resource impls (Phase 4.4 of
`m6-resource-finalization-integration-audit.md`).

## Consequences

### Positive

- **API symmetry across Action / Resource / Credential** — same slot-binding
  pattern, same `#[derive(X)]` macro family, same field attribute conventions
  (per ADR-0043).
- **Multi-credential resources are first-class** — declarative, visible in
  `Dependencies` builder, included in catalog / UI dependency tree.
- **No hidden CredentialStore lookups inside `create`.** Engine resolves all
  credentials before passing the populated `&self`; resource bodies are pure
  business logic.
- **`Resource::Credential = NoCredential;` opt-out goes away** — express
  no-credential by writing zero `#[credential]` fields.
- **Per-slot refresh policy** — multi-credential resources can rotate
  individual creds without invalidating the whole runtime (the resource
  decides per-slot whether to recycle).

### Negative

- **All existing Resource impls migrate.** Workspace-wide impact: every
  `impl Resource` with `type Credential = …` rewrites. The migration
  scope was scanned during the v4 design dialogue and is bounded;
  Phase 4.4 codemod handles it.
- **`Resource::create` signature changes.** Contract test fixtures, mocks,
  and any internal call sites adapt.
- **Refresh hook signature changes.** Existing
  `on_credential_refresh(&self, scheme: &Scheme)` impls migrate to the
  slot-name-aware form; mostly mechanical.
- **External plugin authors who built against ADR-0036 must migrate.** No
  shim path. Migration guide ships with cascade (Phase 11.1).

### Follow-up work

- Phase 4 of `m6-resource-finalization-integration-audit.md` implements the
  trait shape change, the `#[derive(Resource)]` macro, and the codemod.
- ROADMAP §M11 (dependency redesign) records this supersession alongside
  ADR-0042 / ADR-0043.
- `nebula-credential::NoCredential` opt-out type may be retained for a
  release as a no-op marker (zero field reference) to ease the migration;
  remove after one minor cycle.

## Alternatives considered

### Alternative A — Keep ADR-0036 + add a `Resource::AdditionalCredentials` array

Rejected: parallel API surface (singular primary + multi-cred bag) violates
single-declaration-path principle from ADR-0043. Plugin authors get two ways
to declare credentials with confusing precedence rules. Drift inevitable.

### Alternative B — Soft deprecation with `#[deprecated]` `type Credential`

Rejected per `feedback_no_shims.md`. ADR-0036 framing is structurally
incompatible with slot-binding; preserving it as deprecated keeps the
shadow attack vector (silent dual-API) and ships migration cost across two
release cycles instead of one.

### Alternative C — Internal `CredentialStore::resolve_erased` (status quo)

Rejected: hides credential dependencies from the `Dependencies` builder, so
catalog / UI / registration-time validation cannot see them. Multi-cred
resources need declarative visibility.

## Seam / verification

- **Trait shape lock.** `nebula-resource::Resource` (current
  `crates/resource/src/resource.rs:229`) moves to slot-binding form in
  Phase 4.1. Old `type Credential` deleted.
- **Macro emission.** `#[derive(Resource)]` (Phase 4.2) emits
  `Dependencies` builder pairing each `#[credential]` slot field with its
  declared `key`.
- **Manager registration update.** `Manager::register<R>` (Phase 4.3) resolves
  every credential slot before invoking `Resource::create`.
- **Refresh hook contract.** Per-slot `on_credential_refresh(&mut self,
  slot_name: &str)` invariant: implementer must handle every declared
  credential slot name; engine emits `WARN [resource]` if rotation event
  arrives for an unhandled slot.
- **Test gate.** Phase 4.4 migration adds an integration test in
  `crates/resource/tests/multi_credential_resource.rs` exercising a
  two-credential Resource (primary DB cred + audit cred) with both rotating
  independently; assertions cover the per-slot refresh path.

## Migration note

Existing impls of the form:

```rust
impl Resource for Foo {
    type Credential = MyCred;
    async fn create(&self, config: &Config, cred: &MyCred, ctx: &dyn Ctx) -> Result<…> {
        // use cred
    }
}
```

migrate mechanically to:

```rust
#[derive(Resource)]
#[resource(key = "foo", topology = "…", config = FooConfig)]
struct Foo {
    #[credential(key = "auth")]
    auth: CredentialGuard<MyCred>,
}

impl Resource for Foo {
    // type Credential — gone
    async fn create(&self, ctx: &dyn Ctx) -> Result<…> {
        let cred = &self.auth;   // already typed and resolved
        // use cred
    }
}
```

Codemod scripted in Phase 4.4; manual review for non-trivial cases.
