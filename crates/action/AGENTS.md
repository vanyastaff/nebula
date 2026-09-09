# nebula-action — Agent orientation
> Local guide for `crates/action/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Defines the typed action trait family (`StatelessAction`/`StatefulAction`/`TriggerAction`/`ResourceAction` + DX specializations) and `ActionMetadata` the engine uses for discovery, validation, and dispatch.
**Layer:** Business — depends only downward (root AGENTS.md → Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new action type | 1. Implement one of the trait variants (`StatelessAction`, etc.) 2. Define `Input`/`Output` types with `HasSchema` 3. Add `#[resource]`/`#[credential]` slots if needed 4. Register in `PluginRegistry` |
| Add a webhook action | Implement `WebhookAction` — defaults to `SignaturePolicy::Required` (fail-closed). Secret material never flows through dyn `TriggerHandler`. |
| Add retry hints | Use `ActionError` + `RetryHintCode` in `src/error.rs` — retryable vs fatal. The engine's Layer 2 retry handles the rest. |

## Commands

- Derive contract: `cargo nextest run -p nebula-action --test derive_action --test derive_action_compile_fail`.
- If trybuild times out, diagnose with `cargo test -p nebula-action --test derive_action_compile_fail`, then rerun the required nextest check; do not assume a cold-cache timeout is harmless.

## Key files

- `src/lib.rs` — public re-export surface + module map (`#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`)
- `src/action.rs` — base `Action` trait (`Sized`, `type Input/Output: HasSchema`, static `metadata()`/`dependencies()`); NOT object-safe
- `src/handle.rs` + `src/factory.rs` — `ActionHandle` enum + per-variant `XxxHandle` trait objects + `ActionFactory`/`Generic*Factory` engine-side dispatch
- `src/from_workflow_node.rs` — `FromWorkflowNode` async slot-binding factory (derive emits the body)
- `src/error.rs` — `ActionError` + `RetryHintCode` (retryable vs fatal)
- `src/result.rs` / `src/output.rs` — `ActionResult` flow-control intent + `ActionOutput` (inline/blob/stream)
- `src/webhook/` — `WebhookAction` + HMAC signature primitives (ADR-0022 fail-closed)

## Conventions & never-do

- `Action: Sized` is **not** object-safe — never write `dyn Action`; engine dispatch goes through `Arc<dyn ActionFactory>` + `Box<dyn XxxHandle>`.
- No `schema` method — `Input`/`Output: HasSchema` is the single source of truth; read via `nebula_schema::schema_of::<A::Input>()` (ADR-0052 P3). Don't add per-trait `*_schema`.
- Action structs hold **only** slot fields (`#[resource]`/`#[credential]`); user form data lives on a separate `Self::Input` companion struct. `#[credential]` slots hold `CredentialGuard<C::Scheme>`, not `CredentialGuard<C>`.
- `CheckpointPolicy` is a field on `ActionMetadata` (`checkpoint_policy`, default `Inherit`); engine enforcement of non-`Inherit` cadences is not yet wired — treat a non-default policy as declared intent, not a runtime guarantee.
- Durable compilation requires an explicit `ActionEffectContract`; the default `Undeclared` is rejected. Remote-effect factories use the execution-owned effect protocol, not generic dispatch or retry hints as invocation authority. See `tests/deferred_recovery.rs` and the engine's `effect_protocol` suite.
- This crate is NOT the execution driver (the engine dispatches in-process), execution state machine (`nebula-execution`), schema system (`nebula-schema`), or engine retry layer; process/WASM isolation is a canon §12.6 / ADR-0091 non-goal.
- `WebhookAction::config()` defaults to `SignaturePolicy::Required` (fail-closed); secret material never flows through the dyn `TriggerHandler` surface.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Traits and dispatch | [contracts](tests/contracts.rs), [instance_factory](tests/instance_factory.rs), [execution_integration](tests/execution_integration.rs). |
| Macros and slot shapes | [derive_action](tests/derive_action.rs), [derive_action_compile_fail](tests/derive_action_compile_fail.rs); also SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs) for generated paths. |
| Webhook policy | [webhook_signature](tests/webhook_signature.rs), [webhook_request_limits](tests/webhook_request_limits.rs). |

## See also

- `README.md` — full design (v4 surface, migration recipe, contract/canon invariants)
- ADR-0081 (consolidates ADR-0042/0043/0044/0045); [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md) §`nebula-action` (CheckpointPolicy status); [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.5/§11.3/§13.4/§13.5
