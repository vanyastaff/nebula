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
- Action authors cannot select checkpoint cadence. The private serialized `checkpoint_policy` field preserves historical evidence; fresh factories stamp `inherit`, and non-default historical values cannot readmit. Never restore a public selector without complete runtime and recovery support.
- `ActionEffectContract` defaults to `Journaled(JournalProtocol::V1)`; `ReadOnly` keeps its frozen `"NoExternalEffects"` wire tag. Only handle-routed effects are journaled; a stateless, stateful or agent `Journaled` action on a turn with execution stores gets handles under its node's effect journal (engine `effect_driver::journal`, kind mapping `JournalShape`; a stateful one per iteration, an agent per turn — experimental: journaled turns, checkpointed after every `Continue`, bound by the determinism contract in `src/agent.rs`); storeless runs and control/stream actions get read-only handles (writes refused before any provider call, with a detail saying why). `AgentAction` / `AgentHandle` signatures are unchanged; never document a model tool-call id as an idempotency key part. Remote-effect factories use the execution-owned effect protocol, not generic dispatch or retry hints as invocation authority. See `tests/deferred_recovery.rs` and the engine's `effect_protocol` suite.
- This crate is NOT the execution driver (the engine dispatches in-process), execution state machine (`nebula-execution`), schema system (`nebula-schema`), or engine retry layer; process/WASM isolation is a canon §12.6 / ADR-0091 non-goal.
- `WebhookAction::config()` defaults to `SignaturePolicy::Required` (fail-closed); secret material never flows through the dyn `TriggerHandler` surface.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Traits and dispatch | [contracts](tests/contracts.rs), [instance_factory](tests/instance_factory.rs), [execution_integration](tests/execution_integration.rs). |
| Macros and slot shapes | [derive_action](tests/derive_action.rs), [derive_action_compile_fail](tests/derive_action_compile_fail.rs); also SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs) for generated paths. The macros crate sets `test = false`: its expansion unit tests run with `cargo nextest run -p nebula-action-macros --lib`. |
| Resource handle fields (`ResourceHandle<R>` / `Option<ResourceHandle<R>>`, `resource_handle_by_id`) and `#[action(read_only)]` | `src/context.rs` tests (typed row, type mismatch fatal naming only the type, invalid id not echoed, revoked retryable, default accessor refuses), the macros' `field_slots` unit tests, [derive_action](tests/derive_action.rs) `resource_handle_fields`, the `derive_lazy_resource_handle`, `derive_renamed_resource_handle`, `derive_read_only_value` and `derive_renamed_effect_flag` probes, `effect` tests (the `ReadOnly` contract keeps its frozen `"NoExternalEffects"` serde tag), engine [resource_integration](../engine/tests/resource_integration.rs) `resource_handle`, SDK perimeter `action_resource_handle`. A handle field resolves synchronously and checks nothing out; never accept `Lazy<ResourceHandle<R>>`, and never change the `ReadOnly` wire tag. |
| Webhook policy | [webhook_signature](tests/webhook_signature.rs), [webhook_request_limits](tests/webhook_request_limits.rs). |

## See also

- `README.md` — full design (v4 surface, migration recipe, contract/canon invariants)
- ADR-0081 (consolidates ADR-0042/0043/0044/0045); [docs/INTEGRATION_MODEL.md](../../docs/INTEGRATION_MODEL.md) §`nebula-action` (checkpoint and retry contracts); [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.5/§11.3/§13.4/§13.5
