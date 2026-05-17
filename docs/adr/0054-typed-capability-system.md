# ADR-0054: Typed capability system

**Status:** Proposed (2026-05-14)
**Tags:** security, types, action, credential, resource

## Context

Charter F3 principle states: *"Capability-gated, not permission-gated.
Security flows through the type system: an action that doesn't declare
`Capability<ReadCredential<X>>` in its signature cannot compile code
that reads credential `X`."*

Current model: action declares slot-binding fields
(`#[require("auth")] token: Handle<MyCredential>`). Once `Handle` is
present, action body has full access. There is no compile-time
restriction on **what** the action can do with the credential beyond
"hold a typed reference."

Sam Scott (Oso) raised at Day 3: workflow tools traditionally enforce
permissions at runtime (RBAC, ABAC). Rust type system can do better —
**capability-based access control via types**, where compiler refuses
to compile actions that exceed their declared scope.

## Decision

Introduce a typed **capability family** in `nebula-credential` and
`nebula-resource`. Each capability is a phantom-typed marker that
gates access to a sensitive operation.

```rust
// In nebula-credential:
pub trait Capability: Sealed + Send + Sync + 'static {}

pub struct ReadCredential<C: Credential>(PhantomData<C>);
impl<C: Credential> Capability for ReadCredential<C> {}

pub struct WriteCredential<C: Credential>(PhantomData<C>);
impl<C: Credential> Capability for WriteCredential<C> {}

pub struct RotateCredential<C: Credential>(PhantomData<C>);
impl<C: Credential> Capability for RotateCredential<C> {}

// In nebula-resource:
pub struct AcquireResource<R: Resource>(PhantomData<R>);
pub struct MutateResource<R: Resource>(PhantomData<R>);

// In nebula-action (or sdk):
pub struct NetworkAccess<const DOMAIN: &'static str>;   // const-generic domain
pub struct FilesystemAccess<const ROOT: &'static str>;
pub struct SubprocessExec;                              // unit marker
```

Action declares required capabilities in metadata or via marker trait:

```rust
#[derive(Action)]
#[action(
    key = "stripe.charge",
    capabilities = [
        ReadCredential<StripeKey>,
        NetworkAccess<"api.stripe.com">,
    ],
)]
struct StripeCharge {
    #[require("api_key")] key: Handle<StripeKey>,
}
```

Sensitive operations (read credential value, open network connection,
spawn subprocess) take **`Capability` token as parameter**:

```rust
impl CredentialHandle<StripeKey> {
    pub fn expose<'a>(
        &'a self,
        _cap: &ReadCredential<StripeKey>,
    ) -> &'a SecretString { /* ... */ }
}
```

Without `_cap` token in scope, `expose()` cannot be called. Token comes
from action metadata declaration via macro-emitted `Capabilities`
struct injected into context.

## Consequences

### Positive

- Compile-time enforcement: action declaring no capabilities cannot
  call sensitive ops. Refactoring catches removed capabilities
  immediately.
- Audit-friendly: all capabilities listed in `ActionMetadata` —
  operators grep for `ReadCredential<StripeKey>` to find every action
  touching Stripe credentials.
- Network/filesystem scoping: const-generic domains enable
  per-domain network access lists.
- Aligns with capability-based security literature (KeyKOS, EROS,
  modern Wasmtime/WASI).

### Negative

- Authoring cognitive load: capabilities added to `#[action]`
  declaration. Mitigated by derive macro defaults (e.g.,
  `#[require("auth")]` auto-adds `ReadCredential<AuthCred>` to
  capability list).
- Const-generic string params (`<const DOMAIN: &'static str>`) require
  Rust 1.59+ — already past MSRV.
- Capability tokens passed everywhere where sensitive ops happen —
  parameter clutter. Mitigated by type alias / context injection.

### Neutral

- Doesn't replace runtime checks for dynamic policy (rate limits,
  per-tenant quotas) — capability is **structural**, runtime checks are
  **policy**.
- Requires `negative_impls` (when stable) or sealed-trait pattern for
  capability-set arithmetic.

## Migration

Pre-capability code: actions can call sensitive ops freely.
Post-capability code: actions must declare capabilities. Breaking
change for plugin authors.

Mitigation: phase 1 — capabilities optional, runtime-warned when
missing. Phase 2 — mandatory, compile error. Two minor releases
between.

## References

- Conference Day 3 (CONFERENCE-NOTES.md) — Sam Scott proposal.
- KeyKOS / EROS capability-based OS literature.
- Wasmtime WASI capability model.

## Out of scope

- Capability-aware policy engine (runtime decision: this user can
  invoke this action with these capabilities) — separate ADR if needed.
- Cross-process capability propagation (e.g., spawned sandbox
  processes inheriting subset of caps) — phase 2 work.
