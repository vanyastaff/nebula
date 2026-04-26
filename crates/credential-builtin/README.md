# nebula-credential-builtin

Built-in concrete credential types for Nebula. Plugin authors depend on
`nebula-credential` (the contract crate); first-party concrete types
(`SlackOAuth2`, `BitbucketOAuth2`, `BitbucketPat`, `BitbucketAppPassword`,
`AnthropicApiKey`, `AwsSigV4`, ...) live here.

## Why split

Per Strategy §2.4 (frozen Checkpoint 1, commit `4316a292`):

> Plugin authors depend only on the contract crate (`nebula-credential`);
> built-in concrete types live in a separate crate so the trait-only
> dependency surface stays clean for third-party consumers and so
> built-in types can evolve (add credential types, bump dependencies,
> refactor concrete impls) without touching the contract crate's
> stability surface.

## Plugin-author onboarding

1. Depend on `nebula-credential` (contract). Do **not** depend on
   `nebula-credential-builtin`.
2. Declare your own `mod sealed_caps { pub trait MyCapSealed {} }` at
   crate root, one inner trait per capability you introduce. See
   ADR-0035 §3.
3. Use `#[plugin_credential(...)]` on the credential struct and
   `#[capability(scheme_bound = ..., sealed = ...)]` on each capability
   trait you introduce.
4. Register concrete types at plugin init via
   `registry.register(MyCred, env!("CARGO_CRATE_NAME"))?` — pass the
   credential instance and the registering crate name. Duplicate `KEY`
   is fatal in both debug and release.
5. The contract's `nebula-credential::sealed::Sealed` is emitted by
   `#[plugin_credential]`; do not impl by hand.

## What's here

П1 ships the empty scaffold. Concrete types land in П3. See the
phase plan at `docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md`.
