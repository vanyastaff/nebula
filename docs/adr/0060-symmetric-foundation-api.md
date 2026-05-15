# ADR-0060: Symmetric Foundation API (`Acquirable` / `Resolvable` / `Handle<T>` / `#[require]`)

**Status:** Proposed (2026-05-14)
**Tags:** action, resource, credential, sdk, dx, breaking

## Context

Charter F9: *"Symmetric API surface across Foundation Five."* Author
should not pay cognitive tax for `ResourceGuard<R>` vs
`CredentialGuard<C::Scheme>` vs different attributes — same concept
("typed handle to acquired infrastructure"), different words.

Today (asymmetric):
```rust
#[resource(key = "bot")]    bot: ResourceGuard<TelegramBot>,
#[credential(key = "auth")] token: CredentialGuard<<TelegramCredential as Credential>::Scheme>,
```

Charter goal (symmetric):
```rust
#[require("bot")]    bot: Handle<TelegramBot>,
#[require("auth")]   token: Handle<TelegramCredential>,
```

## Decision

### `Acquirable` trait + `Handle<T>` alias

```rust
pub trait Acquirable: Send + Sync + 'static {
    type Handle;
    type Error;
    
    fn acquire<'a>(
        ctx: &'a dyn AcquireContext,
        key: &'a str,
    ) -> impl Future<Output = Result<Self::Handle, Self::Error>> + Send + 'a;
}

// Two blanket impls (one per kind):
impl<R: Resource> Acquirable for R {
    type Handle = ResourceHandle<R>;
    type Error  = ResourceError;
    /* via ctx.resources().acquire::<R>(key) */
}

impl<C: Credential> Acquirable for C {
    type Handle = CredentialHandle<C>;
    type Error  = CredentialError;
    /* via ctx.credentials().acquire::<C>(key) */
}

// Convenience type alias:
pub type Handle<T> = <T as Acquirable>::Handle;
```

Type discrimination at compile time. Sealed pattern (or
`#[diagnostic::do_not_recommend]`) prevents one type implementing
both `Resource` and `Credential`.

### Single `#[require(...)]` attribute

```rust
#[require("bot")]                    // string key only
#[require("metrics", on_failure = "degrade")]   // with options
#[require]                           // type-keyed, when single instance of type
```

Kind (Resource vs Credential) inferred from `T` in `Handle<T>` via
`Acquirable` blanket impl.

### Modifier composition via `Resolvable` trait

```rust
pub trait Resolvable: Sized {
    type Output;
    type Error;
    
    async fn resolve(ctx: &dyn AcquireContext, key: &str) -> Result<Self, Self::Error>;
}

// Direct (required + eager):
impl<T: Acquirable> Resolvable for T::Handle { /* via T::acquire */ }

// Optional (returns None on NotFound):
impl<H: Resolvable> Resolvable for Option<H> { /* try-acquire-or-None */ }

// Lazy (defer acquisition until .get(ctx).await):
impl<H: Resolvable> Resolvable for Lazy<H> { /* defer */ }
```

Composition automatic — `Option<Lazy<Handle<R>>>` works via
blanket-on-blanket.

### Four supported modifier combinations

| Type | Semantics |
|---|---|
| `Handle<X>` | required + eager |
| `Option<Handle<X>>` | optional + eager (None if key not registered) |
| `Lazy<Handle<X>>` | required + lazy (acquired on first `.get(ctx).await`) |
| `Option<Lazy<Handle<X>>>` | optional + lazy |

### Conscious non-support in v1.x

| Wrapper | Rationale |
|---|---|
| `Vec<Handle<X>>` | Multi-instance via multiple field declarations or fan-out resource |
| `Refresh<H>` | Becomes `Refreshable` trait impl on Credential, not wrapper |
| `Pooled<H, N>` | Becomes resource topology in `nebula-resource` |
| `Cached<H>` | Niche, defer |
| `Failover<[H; N]>` | Niche, defer |

`#[diagnostic::on_unimplemented]` on `Resolvable` provides helpful
error suggesting alternatives.

### Derive macro emit

`#[derive(Action)]` emits one uniform line per `#[require]` field:

```rust
field_name: <FieldType as Resolvable>::resolve(ctx, "key").await?,
```

Same call signature for all four modifier combinations. No
match-on-modifier-combination logic in macro.

### Removed (legacy, hard breaking)

- `CredentialFor<C>` type alias — drop.
- `<C as Credential>::Scheme` from author-facing API — drop (still
  internal to engine bridging).
- Separate `#[resource]` / `#[credential]` attributes as default form
  — kept as opt-in explicit form for emphasis cases (per dtolnay
  compromise Day 5 late session).

## Consequences

### Positive

- Author cognitive load drops to single mental model for both
  Resource and Credential acquisition.
- Generic helper code (e.g., `fn acquire_n<T: Acquirable>(...)`)
  works over both kinds.
- Modifier composition through standard Rust wrappers (`Option`,
  `Lazy`) — zero learning curve for Rust developers.
- Standardized observability (per F9): same `tracing::span!(target =
  "nebula::acquire", kind, key, ...)` shape for both resource and
  credential acquisitions.
- Cross-kind retry semantics via shared `AcquireFailure` trait
  (Eliza Weisman Day 5 late session).

### Negative

- **Hard breaking change** for plugin authors. Migration table in
  VISION.md §5.
- Sealed pattern adds boilerplate to `Resource` / `Credential` trait
  declarations.
- `Handle<T>` type alias resolution noise in compile errors —
  mitigated by `#[diagnostic::on_unimplemented]`.

### Neutral

- Existing legacy attributes (`#[resource]` / `#[credential]`)
  preserved for explicit-form opt-in. No churn for emphasis cases.
- TypedDAG forward-compat (per F18 / ADR-0056) — `#[require]`
  declarations stable across runtime → compile-time binding modes.

## Migration table

| Aspect | Before (v0.x) | After (this ADR) |
|---|---|---|
| Resource handle | `ResourceGuard<R>` | `Handle<R>` |
| Credential handle | `CredentialFor<C>` / `CredentialGuard<C::Scheme>` | `Handle<C>` |
| Resource attribute | `#[resource(key = "...")]` | `#[require("...")]` (default) |
| Credential attribute | `#[credential(key = "...")]` | `#[require("...")]` (default) |
| Optional resource | `Option<ResourceGuard<R>>` | `Option<Handle<R>>` |
| Lazy resource | `Lazy<ResourceGuard<R>>` | `Lazy<Handle<R>>` |
| Acquisition method | `ctx.acquire_resource_by_id(...)` / `ctx.resolve_credential_by_id(...)` | `ctx.acquire("key")` |
| Error trait | distinct `ResourceError` / `CredentialError` | both impl shared `AcquireFailure` |
| Observability target | `nebula::resource` / `nebula::credential` | `nebula::acquire` with `kind` field |

## References

- Conference Day 5 late session (CONFERENCE-NOTES.md) — Tokio
  maintainers Carl Lerche / Alice Ryhl / Eliza Weisman.
- Conference Day 5 breakfast — Resolvable composition decisions.
- ADR-0042 (node binding mechanism) — partially superseded.
- ADR-0043 (dependency declaration DX) — `Refresh`, `Pooled` now
  out-of-scope per this ADR.

## Out of scope

- Performance benchmarks of trait dispatch — `cargo bench` work,
  separate.
- IDE plugin support for `#[require]` autocomplete — tooling work,
  separate.
