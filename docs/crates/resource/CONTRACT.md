# Resource & SDK Stable Contract (2026+, Decade of AI)

> **Version**: 1.0.0 | **Created**: 2026-03-02  
> **Scope**: Long-term stable API for nebula-resource, nebula-core (deps), and nebula_sdk.  
> **Audience**: Core maintainers, ecosystem authors, and AI-assisted tooling.

---

## Purpose

This document defines the **stable contract** that the Nebula platform guarantees for the next decade. Implementations may change; this contract should not. It is designed for:

- **Human authors** building custom Resources and Credentials via `nebula_sdk`.
- **AI-assisted development**: code generation, completion, and agents that need predictable names, types, and semantics.
- **Ecosystem crates**: SSH, Google Drive, Telegram, observability sinks, LLM providers — all consume the same surface.

Breaking this contract requires a **major** version bump and a MIGRATION.md entry.

---

## Baseline: Rust 1.93 and Edition 2024

- **MSRV**: Rust 1.93. All public APIs are designed against this compiler.
- **Edition**: 2024. Idiomatic patterns (native async traits where applicable, modern syntax) are used throughout; no legacy workarounds for older compilers.
- **Async**: Resource and credential traits use **native `async fn`** in their author-facing API. Internal use of `async_trait` or `impl Future` is an implementation detail only where `dyn` dispatch is required (e.g. manager/pool).

---

## 1. Resource Trait Contract

### 1.1 Required Surface

```text
Resource:
  - type Config: Config
  - type Instance: Send + Sync + 'static
  - fn metadata(&self) -> ResourceMetadata
  - async fn create(&self, config: &Self::Config, ctx: &Context) -> Result<Self::Instance>
  - async fn is_valid(&self, instance: &Self::Instance) -> Result<bool>   // default: Ok(true)
  - async fn recycle(&self, instance: &mut Self::Instance) -> Result<()>  // default: Ok(())
  - async fn cleanup(&self, instance: Self::Instance) -> Result<()>        // default: drop; Ok(())
```

- **Config**: Must implement `Config` (at minimum `validate()`). Validation runs before registration and before first `create`.

### 1.2 Lifecycle Guarantees

- **create**: Called when the pool needs a new instance. Must be deterministic from `(config, ctx)`; no hidden global state that changes semantics.
- **is_valid**: Called when an instance is considered for reuse. Return `false` to force disposal and a new `create`.
- **recycle**: Called before an instance is returned to the idle pool. Use for resetting state, not for heavy I/O.
- **cleanup**: Called when an instance is permanently removed. Must not fail in a way that leaks resources; best effort then log.

---

## 2. ResourceKey and ResourceMetadata

### 2.1 ResourceKey Convention

- **Domain**: Keys are in the `resource` domain (e.g. via `ResourceKey` from `nebula-core`). Normalized form: `a-z0-9_` only.
- **Naming**: Recommended pattern is `vendor_category` or `service_name` for discoverability and uniqueness:
  - Stdlib examples: `http_client`, `postgres`, `redis_cache`, `file_storage`.
  - Ecosystem examples: `google_drive`, `telegram_bot`, `openai_chat`, `anthropic_completion`.
- **Stability**: Once a key is used in production or in a published crate, it must not be reused for a different resource type. New resources must use new keys.

### 2.2 ResourceMetadata (Passport)

- **Required**: `key`, `name`, `description`.
- **Optional but recommended**: `icon`, `icon_url`, `tags`, and (if introduced) `category` enum.
- **Tags**: Free-form strings. **Stable vocabulary** for UI and AI discovery:
  - **Category**: `category:network`, `category:database`, `category:cache`, `category:messaging`, `category:storage`, `category:external-api`, `category:internal-service`, `category:credential`, `category:ai`.
  - **Protocol**: `protocol:http`, `protocol:https`, `protocol:ssh`, `protocol:grpc`, `protocol:websocket`.
  - **Service**: `service:postgres`, `service:redis`, `service:google_drive`, `service:telegram`, `service:openai`.
- **AI and LLM resources**: Use `category:ai` (or equivalent) and service tags so the platform and tooling can list and filter "AI resources" without special cases.

---

## 3. Context Contract

- **Flat structure**: No generic parameters. Same `Context` type for all resources.
- **Fields**: `scope`, `workflow_id`, `execution_id`, `tenant_id`, `cancellation`, `metadata` (string map), `credentials` (optional provider).
- **Credentials**: The only way to pass secrets into resource creation is `Context::credentials()` → `CredentialProvider::get(key)`. No backdoors. Resource implementations that need a credential must declare it in `Deps` (when credential registry is integrated) and use the provider at `create` time.
- **Helpers**: Convenience methods (`tenant()`, `region()` from metadata, `is_cancelled()`) are part of the stable surface so authors and generated code can rely on them.

---

## 4. Error Model

- **Type**: `nebula_resource::Error` (or equivalent). Enum is **non_exhaustive**.
- **Stable classification**:
  - **Configuration / Validation**: Not retryable. Caller must fix config or inputs.
  - **Timeout, PoolExhausted, Unavailable { retryable }, CircuitBreakerOpen**: Retryable; `is_retryable() == true`.
  - **ScopeViolation, Quarantined, Internal, …**: Semantics documented and stable so that SDK, engine, and AI tooling can map to "show form" vs "retry" vs "escalate".
- **Resource key**: When applicable, errors carry `resource_key()` so that logs and UI can attribute failure to a specific resource type.

---

## 5. Dependencies (deps)

- **Source of truth**: `Resource::Deps: FromRegistry`. Implementations use `deps![CredentialType, OtherResource]` (or `()` for none).
- **Semantics**: At registration/startup, the platform may resolve `Deps` to ensure required credentials and resources are registered and to enforce ordering. No hidden or string-based dependency list; everything is typed.
- **Ecosystem**: Third-party resources that need HTTP must depend on the stdlib `HttpResource`; those that need credentials depend on the appropriate credential type. This keeps the model uniform and discoverable.

---

## 6. nebula_sdk as the Single Facade

- **Stable surface**: Authors of Resources and Credentials **should** depend on `nebula_sdk` (or the minimal set of crates re-exported by it), not on `nebula-resource` / `nebula-core` directly unless they need internals.
- **Exports**: At least: `Resource`, `Config`, `Context`, `Scope`, `ResourceMetadata`, `ResourceKey`, `deps!`, `Requires`, `FromRegistry`, credential traits, and error types. What is in the facade is stable; what is only in inner crates may change.
- **Derives and macros**: The contract allows (and encourages) `#[derive(Resource)]` and similar to reduce boilerplate. The *generated* code must satisfy this document; the macro is an implementation detail.

---

## 7. AI-Decade Considerations

- **Predictable names and signatures**: Types and methods keep stable names and semantics so that code generation and RAG over docs/code produce correct usage.
- **Discoverability**: `metadata()` and tags provide enough information for "list all resources", "list all AI resources", "what does this resource need (deps)" without parsing source code.
- **Schema-first future**: The contract does not mandate JSON Schema or OpenAPI today, but the design (typed Config, stable metadata, explicit errors) allows adding machine-readable schemas later without breaking authors.
- **Single pattern**: One resource = one type, one key, one Config, one Deps. No special cases so that both humans and AI can learn one pattern and apply it everywhere.

---

## 8. Non-Negotiables (Summary)

| Area            | Non-negotiable |
|-----------------|----------------|
| Resource trait  | Native async lifecycle; `Deps: FromRegistry`; no legacy `dependencies()`. |
| Keys & metadata | Stable key convention; documented tag vocabulary; metadata as passport. |
| Context         | Flat, credentials-only for secrets; helpers stable. |
| Errors          | Non-exhaustive enum; `is_retryable()` and `resource_key()`; clear semantics. |
| Deps            | Typed only; resolution and ordering at startup. |
| SDK             | Single facade for authors; internals can evolve behind it. |
| Rust            | MSRV 1.93; edition 2024; idiomatic async. |

---

## 9. Out of Scope (Can Change Without Breaking Contract)

- Internal pool implementation (data structures, locking, strategies).
- Feature flags and optional dependencies (metrics, tracing, credentials backend).
- Event payload details beyond "there is an EventBus and typed events".
- Exact shape of `ResourceStatus` and observability APIs (can grow additively).

---

## 10. Versioning and Changes

- **Patch**: Bug fixes, doc fixes. No API or contract change.
- **Minor**: Additive only (new optional fields, new error variants behind `#[non_exhaustive]`, new helpers). No removals, no signature changes.
- **Major**: Any breaking change to this contract. Requires MIGRATION.md and clear upgrade path.

This document is the **contract**. Code and other docs (CONSTITUTION, DECISIONS, API.md) must align with it; where they conflict, this document wins for the purpose of stability guarantees.
