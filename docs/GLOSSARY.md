# Glossary

Scope: canonical meaning of every typed identifier that `docs/PRODUCT_CANON.md` and its satellites refer to by name. One row per symbol; **crate README and source remain the mechanical source of truth** — this file exists so a newcomer can read the canon without guessing which names are Rust types, which are concepts, and where each lives.

**How to read this file:**

- **Crate** — the crate that owns the type (not every crate that re-exports it).
- **Kind** — `type` = concrete Rust item (struct/enum/trait); `concept` = canon-level term that may map to multiple types; `table` = persisted storage schema.
- **Status** — uses the canon §11.6 vocabulary (`implemented` / `best-effort` / `experimental` / `planned` / `demo-only` / `false capability`). Blank = not classified by the canon; treat as “see crate README.”
- **Canon** — the §s in `docs/PRODUCT_CANON.md` that govern the symbol’s contract.

If a row and the canon disagree, **the canon wins** and this file is wrong — fix it in the same PR.

---

## 1. Identifiers and keys (`nebula-core`)

Stable, opaque handles shared by every other crate. Changing any of these cascades across the workspace — extend `nebula-core` deliberately (canon §3.10).

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `ExecutionId` | type | `implemented` | Stable handle for a single execution run. Component of the idempotency key. | §3.10, §11.3 |
| `ActionKey` | type | `implemented` | Stable identity of a registered action across plugin loads. | §3.5, §3.10 |
| `CredentialKey` | type | `implemented` | Stable identity of a registered credential type. | §3.5, §3.10 |
| `PluginKey` | type | `implemented` | Typed plugin identity derived from `[plugin].id` or `[package].name`. | §7.1 |
| `AuthScheme` | type | `implemented` | Enum over the twelve universal auth schemes (OAuth2, API key, mTLS, …) plus extensibility. | §3.5, §3.10 |
| `AuthPattern` | type | `implemented` | Structural classifier for how a credential refreshes/rotates. | §3.10 |
| `SecretString` | type | `implemented` | Redacted wrapper used for credential material in logs and `Debug`. | §3.10, §12.5 |

---

## 2. Execution authority (`nebula-execution` + `nebula-storage`)

The single source of truth for what a run did and where it is. Canon §11.1 makes `ExecutionRepo` authoritative; handlers must not invent parallel lifecycles.

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `ExecutionRepo` | type | `implemented` | The only legitimate path to transition execution state. Uses optimistic CAS against a persisted `version`. | §11.1, §12.2 |
| `ExecutionRepo::transition` | method | `implemented` | CAS-protected state transition. No handler may mutate state except through this call. | §11.1, §12.2 |
| `executions` row | table | `implemented` (durable) | Authoritative per-run state + monotonic `version`. | §11.5 |
| `execution_journal` | table | `implemented` (durable) | Append-only replayable timeline of an execution. | §11.5 |
| `execution_control_queue` | table | `implemented` (durable) | Outbox for run/cancel signals. Writes happen **in the same logical operation** as the state transition (§12.2). | §11.5, §12.2 |
| `ExecutionControlQueue` | concept | `implemented` | Logical name for the outbox surface; backed by `execution_control_queue` + a consumer (`nebula_engine::ControlConsumer`). All five commands (`Start` / `Resume` / `Restart` / `Cancel` / `Terminate`) are dispatched end-to-end via `EngineControlDispatch` (ADR-0008 A2 + A3). | §12.2 |
| `ControlConsumer` | type | `implemented` | Engine-owned consumer that drains `execution_control_queue` via `ControlQueueRepo` and hands typed commands to `ControlDispatch`. See ADR-0008. | §12.2 |
| `ControlDispatch` | trait | `implemented` | Engine-owned dispatch surface. `ControlConsumer` translates storage rows → typed `ExecutionId` + command; implementors must be idempotent per `(execution_id, command)`. A2 wired `Start`/`Resume`/`Restart`; A3 wired `Cancel`/`Terminate` via the per-instance cancel registry (see ADR-0016). | §12.2 |
| `stateful_checkpoints` | table | `best-effort` (failure mode) | Resume anchor at checkpoint boundaries. Write failure logs and does not abort execution; work since last successful checkpoint may be replayed or lost. | §11.5 |
| `execution_leases` | table | `planned` / partial | Schema may exist before enforcement. Do not imply lease safety unless the engine consumes leases in the deployment path. | §11.5 |
| `Cancel` | variant | `implemented` | Control-queue command. Enqueue / observation / ack path is `implemented`; engine-side dispatch lands via ADR-0008 chip A3 (`EngineControlDispatch::dispatch_cancel` → `WorkflowEngine::cancel_execution`; ADR-0016). Consuming `Cancel` drives a run to a terminal state by signalling the live frontier's cancel token. | §12.2, §13 |
| `Cancelled` | state | `implemented` | Terminal status reached when cancel propagates end-to-end. | §13 |

---

## 3. Action model (`nebula-action`)

Canon §3.5 makes actions typed and engine-dispatched by trait, not by a single metadata “kind” field. Canon §3.8 is the crate pointer.

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `StatelessAction` | trait | `implemented` | Action with no stored state between invocations. | §3.5, §3.8 |
| `StatefulAction` | trait | `implemented` | Action that owns durable state spanning invocations (checkpointed). | §3.5, §3.8 |
| `TriggerAction` | trait | `implemented` | Source of executions. Delivery is at-least-once unless explicitly stronger; dedup via stable event identity. | §3.5, §9, §11.3 |
| `WebhookAction` | trait | `implemented` | DX specialization over `TriggerAction` for HTTP webhooks. Exposes `config() -> WebhookConfig` so the HTTP transport can enforce signature policy before dispatch. | §3.5, §9, ADR-0022 |
| `WebhookConfig` | type | `implemented` | Opaque bag of webhook-transport settings returned by `WebhookAction::config`. `#[non_exhaustive]` — future slots (body-limit override, rate-limit override) land here without breaking the trait. | ADR-0022 |
| `SignaturePolicy` | enum | `implemented` | `Required` / `OptionalAcceptUnsigned` / `Custom(fn)` signature-verification policy enforced by the transport. Default is `Required` with empty secret — fail-closed. Supporting types: `RequiredPolicy`, `SignatureScheme`. | ADR-0022, §4.2 |
| `ResourceAction` | trait | `implemented` | Action bound to a graph-scoped resource node. | §3.5, §3.8 |
| `ActionResult` | enum | mixed — **see §11.2 debt** | Action return variants. Engine-level retry through a `Retry` variant is `planned` / `false capability` until persisted attempt accounting lands. | §11.2, §14 |
| `ActionMetadata` | type | `implemented` | Descriptor: key, ports, parameters, isolation, `ActionCategory`, `CheckpointPolicy`. Supplements but does not replace trait-based routing. | §3.5 |
| `ActionCategory` | enum | `implemented` | Data / Control / Trigger / … classifier for UI and validation. | §3.5 |
| `ActionError` | type | `implemented` | Typed error returned by actions; pairs with `nebula-resilience` retry hints. | §3.10 |
| `CheckpointPolicy` | type | `implemented` | Metadata-declared checkpoint behavior the runtime enforces. | §3.5, §11.5 |

---

## 4. Resource and credential (`nebula-resource`, `nebula-credential`)

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `DrainTimeoutPolicy` | type | `best-effort` (crash path) | Bounds how long the next process waits to drain orphaned resources left by a crash. | §11.4 |
| `ReleaseQueue` | concept | `best-effort` (crash path) | Surface through which orphaned resources are reclaimed on next-process start. Not a security boundary. | §11.4 |
| `SecretToken` | type | `implemented` | Redacted credential wrapper (one of several). `Debug` must stay redacted. | §12.5 |
| `Zeroize` / `ZeroizeOnDrop` | trait | `implemented` | Required on credential key material. Do not bypass “for debugging.” | §12.5 |

---

## 5. Schema and validation (`nebula-schema`, `nebula-validator`)

Canon §3.5 requires **one** schema system shared by actions, credentials, and resources. Canon §3.9 is the crate pointer for validators.

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `Field` | type | `implemented` | Consolidated field kind enum (string / number / bool / enum / nested / …). Single public surface for all schema fields. | §3.5 |
| `Schema` | type | `implemented` | Typed configuration schema built from `Field` enum; shared across Action / Credential / Resource config via `nebula-schema`. | §3.5 |
| `ValidValues` | type | `implemented` | Proof-token returned by `ValidSchema::validate` — indicates schema-time validation has succeeded. Required to call `resolve`. | §3.5, §4.5 |
| `ResolvedValues` | type | `implemented` | Proof-token returned by `ValidValues::resolve` — indicates runtime expression resolution has succeeded. Required to access resolved field values. | §3.5, §4.5 |
| `Rule` | type | `implemented` | Declarative validator composed into schema fields. | §3.10 |
| `enum_select` | `#[param(...)]` flag | `implemented` | On `#[derive(Schema)]`, maps a single enum field (with `#[derive(EnumSelect)]` and `HasSelectOptions`) to a static `SelectField`. **Not** supported on `Vec<...>`; for multi-value lists of enum options, build a `Field::list` manually and wire item `SelectField` via `SelectField::extend_options` (or static options) instead. | §3.5 |
| `validate_rules_with_ctx` | function | `implemented` | Runs `Rule` slice with an optional `PredicateContext` (required for `Rule::Predicate` to fail closed). Re-exported at the `nebula-validator` crate root next to `validate_rules`. | §3.10 |

---

## 6. Errors (`nebula-error`, `nebula-api`)

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `NebulaError` | type | `implemented` | One workspace-wide error taxonomy. Library crates use it (not `anyhow`). | §3.10, §12.4 |
| `Classify` | trait | `implemented` | Classifies an error into categories/codes — the decision point for transient vs permanent. | §3.10 |
| `ErrorClassifier` | concept | `implemented` | The pattern of using `Classify` (or equivalent) to move retry decisions out of folklore. | §4.2 |
| `ApiError` | type | `implemented` | API boundary error mapped to RFC 9457 `problem+json`. No new ad-hoc `500`s for business logic. | §12.4 |

---

## 7. Plugin (`nebula-plugin`)

Canon §7.1 sets the three-layer rule: `Cargo.toml` (build graph) + `plugin.toml` (trust boundary) + `impl Plugin` (runtime registry).

| Name | Kind | Status | Role | Canon |
| --- | --- | --- | --- | --- |
| `impl Plugin` | trait impl | `implemented` | The only runtime source of truth for what a plugin registers (`actions()`, `resources()`, `credentials()`, locales). | §7.1 |
| `PluginManifest` | type | `implemented` | Bundle descriptor authoritative **after** load: human name, icon, categories, long description, maturity, deprecation. Does not compose `BaseMetadata<K>` (plugin is a container, not a schematized leaf — ADR-0018). Not the signed blob. | §7.1 |

---

## 8. Status legend

Copied from canon §11.6 for convenience. If the two diverge, the canon wins.

| Status | Meaning |
| --- | --- |
| `implemented` | Works end-to-end, covered by tests, safe to rely on as a current contract. |
| `best-effort` | System attempts it, but does not guarantee success under all failures. |
| `experimental` | Shipped but unstable; API/semantics may change; not default production guidance. |
| `planned` | Not implemented yet; do not promise to operators as current behavior. |
| `demo-only` | Works in examples/dev flows; explicitly not a product contract. |
| `false capability` | Type/endpoint exists but engine does not own behavior end-to-end; remove or implement. |

---

## 9. Architectural patterns

Named patterns Nebula uses. Shared vocabulary with the industry corpus (EIP, DDIA, Release It!). Canon rules refer to these by name — this section is the authoritative source for each pattern's Nebula implementation.

| Pattern | Book reference | Nebula implementation |
|---|---|---|
| **Transactional Outbox** | DDIA ch 11; EIP "Guaranteed Delivery" | `ExecutionControlQueue` (`crates/execution/src/control_queue.rs`). Signals written in the same tx as state transitions. |
| **Write-Ahead Log** | DDIA ch 3, 11 | `execution_journal` append-only table; replayable event history. |
| **Idempotent Receiver** | EIP | `crates/execution/src/idempotency.rs` — deterministic per-attempt key checked before side effect. |
| **Optimistic Concurrency Control** | DDIA ch 7 | CAS on `version` column via `ExecutionRepo::transition`. |
| **Bulkhead** | Release It! | `crates/resource/src/release_queue.rs` — scope-bounded resource release; failure in one scope does not cascade. |
| **Circuit Breaker + Timeout + Retry-with-Backoff** | Release It! | `nebula-resilience` composable pipelines — applied at outbound call sites inside actions. |
| **Layered Architecture with cross-cutting infrastructure** | Fundamentals of SW Architecture | `CLAUDE.md` layer direction: API → Exec → Business → Core, cross-cutting below. |
| **Sealed trait + typestate** | Rust for Rustaceans, ch Designing Interfaces | Integration extension points (`Action`, `Credential`, `Resource`) and execution lifecycle (`Execution<State>`). |
| **Make illegal states unrepresentable** | Domain Modeling Made Functional | Applied to public surfaces (§4.5): a type exists ⇔ engine honors it. |

---

## See also

- `docs/PRODUCT_CANON.md` — normative product truth
- `docs/ENGINE_GUARANTEES.md` — durability matrix
- `docs/INTEGRATION_MODEL.md §7` — plugin packaging (`Cargo.toml` + `plugin.toml` + `impl Plugin`), discovery / load lifecycle, ABI policy
- `docs/UPGRADE_COMPAT.md` — compatibility surfaces and pre-1.0 policy
- `crates/*/README.md` — mechanical API truth per crate
