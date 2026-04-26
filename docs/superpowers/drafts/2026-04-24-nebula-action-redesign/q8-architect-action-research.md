# Q8 Phase 1 — Architect Deep Research Read (Action Core + Durable Execution)

**Phase:** 1 of 3 (Phase 2 = synthesis; Phase 3 = decision package).
**Mode:** DEEP RESEARCH READER, not synthesizer.
**Sources read line-by-line:** `docs/research/n8n-action-pain-points.md` (601 lines); `docs/research/temporal-peer-research.md` (370 lines).
**Cross-reference target:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` FROZEN CP4 (3522 lines incl. Q1+Q6+Q7 amendments-in-place); `final_shape_v2.rs`; `crates/action/src/{stateless,stateful,resource,trigger,control}.rs`.
**Constraint:** Identify gaps; do NOT propose Tech Spec amendments; do NOT commit.

---

## §1 n8n action pain points cataloged

Severity scale: 🔴 Tech Spec FROZEN CP4 may need amendment OR escalation; 🟠 partial coverage / open item; 🟡 covered with acceptable-deferral or noted in correlation; 🟢 already addressed.

For brevity, n8n issues are grouped by pain class (each row aggregates the n8n research's bullet list); the right column maps to the Tech Spec section that addresses (or fails to address) the class.

| Source line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| L39-44 | Code-node task-runner timeouts (10+ issues: #25171, #25319, #25986, #23381, #23430, #25356, #27148, #27353, #27838, #28625) — external task broker fragile | Out of scope: Tech Spec is action-trait shape, not Code-node sandbox. Strategy §3.4 line 168 N7 explicitly defers `nebula-sandbox` cascade per `01b-workspace-audit.md`. n8n correlation table line 407 maps to wasmtime/deno_core sandbox (not in this cascade) | 🟡 |
| L48-54 | `pairedItem` lineage breakage (10+ issues: #14568, #27767, #15981, #24534, #4507, #12558 + 5 forum threads) — author-responsibility model | **Tech Spec has NO `ItemLineage` typed surface.** §2 trait shapes do not include item-tracking. n8n correlation L408 names `ItemLineage<InputId>` as engine-managed typed edge — Nebula has no equivalent in §2.5 ActionHandler enum or §7.4 ActionResult routing. **`MultiOutput` / `Branch` variants (§2.7.2 L763, L757) carry no lineage metadata back to input items.** | 🔴 |
| L56-64 | Node versioning silent breakage (#27131, #17481, #22489, #27726 + forum 190031) — workflow pins type version at save time, no migration framework | **Partial coverage.** §2.2.2 R1 amendment (Q7) restored `migrate_state(_old: Value) -> Option<Self::State>` for STATEFUL action persistence. **No migration mechanism for the action's `parameters` / `metadata.version` itself.** n8n correlation L411 names `migrate_v(old, new, params) -> Result<Params>` as required at registration. Tech Spec §13.1 deprecation policy is at the trait level only; per-action workflow-pin migration is undocumented. CR2/CR8/CR11 close emission bugs but do NOT close versioning. | 🔴 |
| L66-75 | HTTP Request edge cases (#26925, #27439, #27735, #27782, #27724, #26044, #16005, #27533, #23815, #27122 + forum 72427, 110925) | Out of scope at trait level; this is integration-node implementation territory. n8n correlation L416 names typed HTTP client in core. ResourceAction §2.2.4 (Q7 R2 `configure`/`cleanup`) provides DI hook for an HttpClient resource but the typed pipeline is integration-cascade scope | 🟡 |
| L77-84 | AI Agent / Tool wrappers churn (#24397, #21740, #18561, #28215, #26202, #27805) — sub-node `supplyData()` wiring fragile to LangChain upgrades | **No coverage.** Tech Spec has no concept of sub-node wiring, no `trait ChatModel` / `trait Memory` / `trait Tool` typed traits. §2.2 four primaries do not enumerate AI-agent shape. n8n correlation L412 names typed traits per sub-node role — entirely unaddressed | 🔴 |
| L86-94 | Merge node behavioral regressions (#7775, #3949, #13182, #19001, #19624, #14986, #15334, #17529, #18429, #18465, #19393, #26853, #26859, #26863) — mode-specific JS with mutable state | Out of scope at trait level; this is integration-node implementation. Tech Spec ActionResult `MultiOutput` (L763) names port routing but Merge semantics are integration-cascade | 🟡 |
| L86-94 | Google Sheets crashes / OAuth disconnects (#26460, #20946, #15490, #17261, #20372) | Out of scope at trait level; integration-cascade | 🟡 |
| L86-94 | If/Switch bugs (#23862, #27693, #27070, #25971) | ControlAction §2.6 / §12.2 sealed DX wraps Stateless via adapter; engine-side branch routing is engine-cascade. Tech Spec §2.7.2 `Branch { selected, output, alternatives }` is the typed surface (lines 757-760) — better than n8n's stringly mode-flag | 🟡 |
| L86-94 | Binary data memory leaks (#8028, #21746) | n8n correlation L413: content-addressed blob + reaper. **Tech Spec has NO binary-data surface.** ActionOutput is `serde_json::Value`-typed. §6.1 depth-cap addresses JSON depth, not binary size | 🟠 |
| L86-94 | Community-node credential leaks across workflows (#27833) | §6.2 hard removal of `CredentialContextExt::credential<S>()` no-key heuristic per security 03c §1 VETO closes the cross-plugin shadow attack (S-C2 / CR3) | 🟢 |
| L99-134 | INodeType declarative vs programmatic / VersionedNodeType pin discipline | §2.6 sealed DX adapter pattern (Q7 R6 — Webhook/Poll as peers of Action with own State/Cursor/Event) provides typed alternative to declarative routing. ADR-0036 / 0037 lock macro emission. **`#[action(version = "X.Y[.Z]")]` per §4.6.2 is set at compile time, not workflow-save-time pin.** n8n's "workflow pins type version at save time" model is structurally absent | 🟠 |
| L137-146 | Core taxonomy ~400 integrations + LangChain | Out of scope; per `docs/COMPETITIVE.md` line 29 explicit non-goal of n8n surface parity. §2.9.1c (Q3 post-freeze) cites COMPETITIVE.md L29 verbatim | 🟢 |
| L181-191 | Native stream processors / stream windowing missing (the n8n research's own coverage-gap table) | **No Stream/StreamingAction in Tech Spec §2.2.** PollAction §2.6 (Q7 R6) is closest analog (cursor-driven event emission) but designed for triggers, not mid-graph stream operators | 🔴 |
| L181-191 | Reliable bulk-ops (#26569 WooCommerce 20× amplification, #26571 Apify Cartesian product) | BatchAction §2.6 sealed DX (`fn extract_items` / `fn process_item` / `fn merge_results`) is the typed surface. **Idempotency invariants for bulk dedup are NOT in §2.6 BatchAction shape — author-responsibility.** Compare Temporal's idempotency-key plumbing (L284) — Nebula has none at action surface | 🔴 |
| L208-218 | Node versioning hot — no migration framework, breaking-change doc exists but doesn't gate save | Same as L56-64 row above. Tech Spec §13.1 trait-level deprecation; no workflow-save-time gate. **CR2 closes macro-emission bugs; does NOT close workflow-version migration class** | 🔴 |
| L221-234 | `execute()` returns `[[...]]` outer = output branch; `continueOnFail()` per-author; `alwaysOutputData` separate flag | Tech Spec §2.7.2 `ActionResult` enum unifies the routing surface (Continue/Skip/Drop/Branch/Route/MultiOutput/Wait + feature-gated Retry/Terminate) — better than n8n's per-author convention. n8n correlation L414 confirms: "Core contract: `Result<Outputs, NodeError>`; engine decides routing; `onError: Fail | Route | Retry` as schema" — Tech Spec §2.8 ActionError taxonomy + §2.7 ActionResult provides this | 🟢 |
| L223-227 | continueOnFail inconsistent (#25581, #20321, #23813, #18908, #26199, #15272) | Tech Spec §6.3 `ActionError` Display sanitization + §7.3 propagation table + ActionResult routing addresses author-responsibility class. Engine decides routing per ActionResult variant | 🟢 |
| L235-249 | `pairedItem` 1:1 auto-fill, custom Code/aggregating loses it | Same as L48-54 — **NO `ItemLineage` in Tech Spec** | 🔴 |
| L250-264 | Binary data 3 modes; `#8028` / `#21746` filesystem fills, exec state too large; `#26968` S3 socket exhaustion; nodes silently drop binary | Same as L86-94 binary row — **no binary-data surface** | 🟠 |
| L265-280 | Code node task runner #1 regression, sandbox prototype pollution, security `#26708` proposal | Out of scope (sandbox cascade); §1.2 N7 `S-I2 sandbox phase-1 cascade` deferred | 🟡 |
| L281-296 | AI Agent / Tool wrappers high churn — `supplyData()` plug | Same as L77-84 — **NO sub-node typed traits** | 🔴 |
| L298-306 | Expression scope `$input.all()`, `$('Node Name').item`, `$json`, `$binary`, `$now` | Out of scope; expression engine is sub-node territory. ActionContext §2.1 lifetime-bound `&'a self` configuration carrier (Q3 §2.9.1c) is the closest analog | 🟡 |
| L308-312 | Node rename consequences — references break, autocomplete breaks (#21982) | n8n correlation L418: stable internal node IDs. Nebula's `ActionMetadata.key` is stable per `#[action(key = "slack.send")]` ADR-0037 §1 — name-rename does not affect engine routing because actions are referenced by key, not by Rust struct name. **However:** the Tech Spec has no `key` rename / supersession story documented. Plugin author who renames `key` from `slack.send` to `slack.post` has no guidance in §13.1 | 🟠 |
| L314-321 | Batch size / concurrency — no framework-level knob, per-node author UI; #21376, #20630, #21817, #28488 freezes/OOM | n8n correlation L420: per-action-type semaphore in engine config from day one. **Tech Spec §1.2 N4 defers cluster-mode coordination to engine cascade; per-action concurrency limit is NOT in §2 trait shapes.** Strategy §3.4 row reserves cluster-mode hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`). Per-action-type concurrency cap (Temporal #7666 14 upvotes) is unaddressed at action layer | 🔴 |
| L325-334 | Version-bump silent breakage (#22489, #17481, #27131, #27070, #18992 + forum 249698) | Same as L56-64 / L208-218 versioning class | 🔴 |
| L336-342 | Code sandbox / prototype (#27734, #16404, #28222, #26865) | Out of scope; sandbox cascade | 🟡 |
| L344-352 | Binary data corruption (#21746, #8028, #19405, #28354) | Same as L86-94 binary row | 🟠 |
| L353-358 | Item linking lost → wrong data routing (#18181, #18180) | Same as L48-54 / L235-249 — **NO `ItemLineage`** | 🔴 |
| L359-379 | Integration-node drift (Sheets / Postgres / LinkedIn / Monday / Todoist deprecations); core ships snapshot of provider API | n8n correlation L417: node manifest pins provider-API version + freshness check + deprecation warnings. Tech Spec `#[action(version = "X.Y[.Z]")]` (§4.6.2) is the action's own version, NOT a tracked provider-API version. **No freshness-check infrastructure** | 🟠 |
| L382-393 | Memory leaks with large datasets (#20124, #16862, #15269, #15628, #1583, #27980) | n8n correlation L421: streaming item pipeline + bounded backpressure. Tech Spec **has no streaming primitive** at action layer. PollAction (§2.6) is event-by-event but not a generic stream-processor shape. nebula-eventbus exists per memory but action surface does not expose stream-shape | 🔴 |
| L395-399 | Merge node cluster (~14 issues) — highest churn | Same as L86-94 Merge row | 🟡 |
| L406-422 | n8n correlation table — root causes summarized | All addressed except: `ItemLineage`, sub-node typed traits (ChatModel/Memory/Tool), binary-as-blob, streaming pipeline, per-action concurrency knob, provider-API freshness | 🔴 (collected) |
| L425-456 | Quick wins for Nebula (10 items): pairedItem-as-typed, `#[derive(Migration)]`, typed OnError, content-addressed binary, wasmtime sandbox, stable IDs, typed HTTP, streaming pipeline, sub-node typed traits, op-per-file | **Tech Spec absorbs:** typed OnError (§2.7.2 ActionResult), stable IDs (`key` attribute). **Tech Spec does NOT absorb:** `ItemLineage`, `migrate_v` for parameters, binary blobs, sandbox-with-wasmtime, typed HTTP, streaming, sub-node traits, op-per-file. Six of ten quick wins are unaddressed at the action-trait layer | 🔴 |
| L460-477 | Meta-idea: engine-managed contracts vs n8n author-responsibility | Tech Spec direction aligns: typed traits per §2 + macro emission per §4 + sealed DX per §2.6. **Pillars realized: vocabulary + macro + dispatch.** **Pillars unrealized: lineage + binary + streaming + sub-node traits + per-action concurrency + provider-API freshness** | 🟠 |

**Pain count.** ~57 distinct n8n pain items cataloged (de-duplicated by class; ~150 underlying GitHub issues / forum threads). Severity distribution:
- 🔴 (Tech Spec FROZEN CP4 needs amendment OR escalation): **8 classes** (pairedItem / lineage; AI sub-node typed traits; binary-data surface absent at trait level; per-action concurrency knob; streaming pipeline absent; bulk-op idempotency; workflow-version migration; quick-wins gap aggregate)
- 🟠 (partial / open / acknowledged-deferred but warrants surface flag): **6 classes** (binary-data corruption; node-version save-pin; provider-API drift freshness; rename-key supersession; meta-idea pillars unrealized partial; node renames at key-stability layer)
- 🟡 (out-of-scope-with-cascade-home): **9 classes** (sandbox; HTTP edge cases; AI agent churn detail; expression-engine; Code-node detail; merge-mode detail; sheets / Postgres detail; node-version-silent-break behavioral)
- 🟢 (already addressed): **3 classes** (`continueOnFail` discipline → ActionResult routing; cross-plugin shadow attack → §6.2 hard removal; non-goal-aligned with COMPETITIVE.md)

---

## §2 Temporal architectural comparison

Side-by-side mapping of Temporal core concepts against Nebula §2 trait family + §3 runtime model + §6 security floor.

| Axis | Temporal | Nebula (Tech Spec FROZEN CP4) | Gap / verdict |
|---|---|---|---|
| **Activity = StatelessAction analog?** | Activity: side-effecting, retried, timed out (research L84). Side-effects allowed; retry policy declarative; activity input always JSON-serialized into history (research L129) | **StatelessAction §2.2.1** L160-170: pure function `execute(&self, ctx, input) -> Result<Output, Error>`. ActionContext is the side-effect carrier; retry surface is `ActionResult::Retry` (§2.7.2) feature-gated. **Different from Temporal:** retry is action-surface-output, not declarative-policy at registration. Idempotency-key plumbing absent. | **Activity ≈ StatelessAction at execute() level.** Diverges on retry-policy declaration (Nebula = output-driven; Temporal = registration-policy-driven) and idempotency. **🟡 Acceptable divergence — Nebula's design intent.** |
| **Workflow = StatefulAction analog?** | Workflow: deterministic, replayable, host-language code; runtime records every side effect as event in history; replays code from history on crash (research L60-64). Workflow is the workflow itself, NOT a single node | **StatefulAction §2.2.2** is a node with iteration state, NOT a workflow. Workflow itself in Nebula is a graph of actions. **Conceptual mismatch:** Temporal Workflow ≠ Nebula StatefulAction; Temporal Workflow ≈ Nebula's graph + engine. | **Different concept entirely.** Nebula's "workflow" is the graph; Temporal's "workflow" is the node-language. **🔴 No durable-execution-as-workflow story in Tech Spec.** Nebula relies on graph-edge persistence, not code-replay. This is the load-bearing architectural difference. |
| **Saga / compensation patterns** | Not first-class in Temporal (research has no Saga line); custom patterns via Workflow + Activity composition with try/catch | **Tech Spec has NO Saga / compensation.** §2.7.2 ActionResult enum has no `Compensate` / `Rollback` variant. ResourceAction (Q7 R2) `cleanup` is per-resource scope-exit, not workflow-Saga. n8n correlation L414: "engine decides routing" — but routing is forward-only in Tech Spec | **🔴 Saga is absent.** Critical for n8n-class workflow-engine parity (some users need rollback semantics — bulk-op partial failure, payment-then-ship). Out of cascade scope per §1.2 OUT but no cascade-home named. |
| **Determinism guarantees (Temporal load-bearing)** | Determinism is Temporal's load-bearing tax (research L36-39 + L135-150). NDE (`TMPRL1100`) is #1 user-facing failure even at 5+ years SDK maturity. Caused by `Date.now()`, `Math.random()`, nested `Promise.all`, ordering. SDK forces `ctx.now()`/`ctx.random()`/`ctx.uuid()` discipline | **Tech Spec has NO determinism contract.** §2.2.1 StatelessAction body is `impl Future + Send + 'a` — author can `SystemTime::now()`, `rand::random()`, etc. freely. No "ctx.now()" in spike `final_shape_v2.rs:205-207` ActionContext. **Cancellation §3.4 + ZeroizeProbe §6.4** are strong but orthogonal to determinism. | **🔴 Determinism is a deliberate non-goal under current Tech Spec posture but UNFLAGGED.** Temporal correlation L283 prescribes `ctx.now()`/`ctx.random()`/`ctx.uuid()` + clippy ban on `SystemTime::now()`. **None are in Tech Spec §1.2 N1-N7 non-goals or §16.5 cascade-final precondition.** Either Nebula commits to no-replay (acceptable; then this gap is intentional and should be canon-stated) OR commits to replay-future (then determinism contract must land). |
| **Versioning + replay safety** | Temporal `Patch / GetVersion` first-class API (research L91, L153-159, L240-242). `workflow.patched("v2-retry-logic")` records patch marker in history. All running workflows continue on old branch; new on new branch. Permanent code branches accumulate; users ask for tooling to clean | **Tech Spec §13.1 / §13.2** — pre-1.0 hard breaking changes acceptable; post-1.0 deprecation cycle + ADR-0035 amend-in-place precedent. **No code-level patch API.** No equivalent of `workflow.patched(name)` at action-body level. Migration is at the macro/codemod layer per §10. | **🟠 Versioning is at deploy-time (codemod), not run-time (patch).** Temporal's pain is over-burdened branches forever; Nebula's design avoids this by requiring all running executions to finish on old version before deploy. **Acceptable for Nebula's posture — but no executions-in-flight migration story is documented.** §16.5 cascade-final: paths (a)/(b)/(c) lockstep cassumes batch deploy + soak + go. |
| **Side-effect handling (idempotency, retries)** | Temporal Activity retries are first-class; idempotency is via Activity authoring discipline + retry-policy `MaximumAttempts` + `RetryPolicy::NonRetryableErrorTypes` (research L84). Activity input always stored in history (research L129 — `temporal#4389` blowing up history) | **Tech Spec has retry hint via `RetryHintCode` (§2.8) + ActionResult::Retry (§2.7.2 feature-gated wire-end-to-end per §1 G6 + §2.7.1).** **No idempotency-key plumbing at action surface.** Action body must hand-roll idempotency. n8n correlation L418 named "stable IDs"; Strategy §3.4 reserves cluster-mode `IdempotencyKey` hook — **deferred to engine cascade per §1.2 N4**. | **🟠 Retry surface is wired-end-to-end (§1 G6 lock); idempotency-key plumbing deferred.** Temporal correlation L286 names per-action-type semaphore as Nebula day-one win; **deferred to engine cascade** is a real gap if user-facing bulk-ops are common (n8n WooCommerce 20× amplification class). |
| **Long-running executions (durable timers)** | Temporal Timers: server-side, up to ~100 years (research L84). `workflow.sleep(Duration::days(30))` survives crashes | **Tech Spec §2.7.2 `ActionResult::Wait { condition, timeout, partial_output }`** (lines 766-770) is the surface. **No "100-year timer" semantics committed.** Wait variant is engine-cascade scope (§7.4 + §8.3 cite ExecutionRepo via canon §11.3). Durability of Wait state is engine concern. | **🟠 Surface exists; durability story is engine-cascade.** Acceptable; Tech Spec scope ends at action surface. **But: there's no §16.5 readiness check that engine cascade absorbs Wait-survives-crash invariant.** |
| **Child workflows / hierarchical execution** | Temporal Child Workflow + Continue-As-New (research L84, L240) | **Nebula has no child-workflow primitive at action layer.** Engine-cascade scope. Tech Spec §2.5 ActionHandler 4-variant enum is single-action-dispatch only. | **🟡 Acceptable deferral.** Engine cascade owns workflow composition. |
| **Search attributes / signal handling** | Temporal Search Attributes (indexed, typed, queryable via SQL-like) (research L84-90). Signal (async message to workflow), Query (read-only RPC), Update (RPC with validator + handler + durable result) | **Tech Spec has NO Signal / Query / Update / Search Attribute analog at action surface.** ActionMetadata.parameters provides schema-as-data per Q3 §2.9.1c, but it's design-time, not run-time queryable. | **🟡 Deferred to engine cascade per §1.2 N4 cluster-mode coordination.** Acceptable; engine cascade scope. |
| **Side-effect: `ctx.now()` / `ctx.random()` / `ctx.uuid()`** | Temporal mandates these at every SDK (research L283) — clippy lint banning `SystemTime::now()` is the recommendation | **Tech Spec ActionContext (spike `final_shape_v2.rs:205-207`)** carries only `creds: &'a CredentialContext<'a>`. **No now/random/uuid surface.** | **🔴 If Nebula ever moves toward replay, this is a missing primitive.** If Nebula commits to no-replay, this should be canon-stated. **Either way, the Tech Spec §16.5 cascade-final precondition does NOT name a determinism posture.** |

### §2.1 Verdict on Temporal-comparison

**Nebula and Temporal occupy structurally different positions.** Temporal is replay-based durable execution where workflow code IS replayed after crash; Nebula is graph-based execution where state is persisted at edges (per Strategy §2.5 / canon §11.3 idempotency). The mismatch is architecturally honest and aligned with `docs/COMPETITIVE.md` line 41 ("Typed Rust integration contracts + honest durability").

**However, three Temporal-prescribed primitives are missing from Tech Spec FROZEN CP4 and have no cascade-home named:**

1. **Determinism contract** — `ctx.now()` / `ctx.random()` / `ctx.uuid()` on ActionContext, OR explicit canon statement that Nebula does NOT commit to replay.
2. **Idempotency-key plumbing at action surface** — for bulk-ops dedup (n8n WooCommerce class). Deferred to engine cascade per §1.2 N4 but warrants explicit cross-cascade naming.
3. **Per-action-type concurrency knob** — Temporal still doesn't have it (`temporal#7666` 14 upvotes); Nebula could ship it. Deferred to engine cascade per §1.2 N4.

**Saga / compensation / child workflows are deferred-with-cascade-home (engine cascade or workflow cascade); acceptable for Phase 8 framing.**

---

## §3 Action variants completeness check

Current Tech Spec primary trait family per §2.2: 4 primaries (Stateless, Stateful, Trigger, Resource); 5 sealed DX (Control, Paginated, Batch, Webhook, Poll).

### §3.1 Are there action SHAPES in n8n / Temporal that Nebula's 4-primary doesn't capture cleanly?

**Examples in n8n that fit awkwardly:**

| n8n shape | Best-fit primary | Awkwardness |
|---|---|---|
| Code node (JS/Python) | Stateless | Fine if pre-validated input; awkward if author wants per-execution state without explicit `type State` declaration. n8n `#15269` significant memory leaks from Code use. **Nebula has no "scriptable code action" primitive** — would require sandbox cascade |
| Set / Edit Fields / Filter / Sort | Stateless | Fine; pure functions. Tech Spec's `parameters = T` covers schema |
| SplitInBatches / Loop | Stateful | Cursor-driven; n8n bug class L548-552 (#21376, #21817, #20630). Stateful's `init_state` / `migrate_state` (Q7 R1) gives the persistence shape; **bounded-iteration declaration is missing.** n8n correlation L419 names "Loop — typed operator with bounded iteration count in schema; engine-enforced." Tech Spec BatchAction (§2.6) has `batch_size()` (default 50) but no max-iteration cap |
| Merge | Stateless or new primary | Tech Spec §2.7.2 has `MultiOutput` for branching but **fan-IN merge-by-position / merge-by-key is not a typed primary.** Awkward fit. Highest-churn n8n node class (L395-399 ~14 issues) — structural mismatch with current 4-primary |
| If / Switch (control flow) | ControlAction (sealed DX) | Fine; §12.2 community plugin authoring path. ActionResult `Branch` is the typed routing |
| HTTP Request | Stateless or ResourceAction (with HttpClient resource) | Fine; ResourceAction (Q7 R2 paradigm restoration) provides DI lifecycle. n8n correlation L416 typed HTTP client |
| Wait | ActionResult::Wait variant | Surface exists but durability is engine-cascade (§7.4) |
| Execute Workflow / Child Workflow | NOT IN TECH SPEC | Engine-cascade scope; Tech Spec §2.5 4-variant enum doesn't include. **Awkward gap if user-facing recursive workflows are common.** |
| AI Agent | NOT IN TECH SPEC | Sub-node `supplyData()` pattern absent. **Major n8n surface area uncovered.** |
| Tool / Integration | StatelessAction with credentials zone | Fine for typed integrations; AI sub-node binding absent (see AI Agent row) |

**Examples in Temporal:**

| Temporal shape | Best-fit primary | Awkwardness |
|---|---|---|
| Activity | Stateless | Good fit |
| Workflow (whole) | Graph itself | Conceptual mismatch — Temporal Workflow ≈ Nebula graph, NOT a single action |
| Local Activity (in-worker) | StatelessAction with no credentials zone | Tech Spec doesn't distinguish local vs network; Temporal's local-activity cache trap (L161-164) is absent in Nebula's model |
| Schedule | TriggerAction (Poll DX) | Q7 R6 PollAction has `poll_config` + cursor — adequate. Temporal correlation L320 names "Schedule entity separate from Trigger" — Tech Spec PollAction couples them but the user-facing shape may differ at schedule cascade scope |
| Update API | NOT IN TECH SPEC | Engine-cascade; absent at action layer (acceptable) |

### §3.2 Is there room for new primary?

Three candidates surface under Phase 1 reading:

**StreamingAction / ProcessorAction.** n8n correlation L421 + research L181: stream processors / windowing / aggregation. Nebula's PollAction is event-by-event but designed for trigger-time consumption, not mid-graph stream operators. A primitive like:
```
trait StreamingAction {
    type Input; type Output; type Error;
    fn process<'a>(&'a self, ctx: &'a ActionContext<'a>, stream: &'a mut Stream<Self::Input>)
        -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```
would close the gap, but it's a 5th primary (canon §3.5 revision territory per §0.2 invariant 1 + ADR-0038 precedent). **Not in Tech Spec.**

**AgentAction / SubGraphAction.** AI Agent shape with sub-node typed traits (ChatModel, Memory, Tool, Retriever) per n8n correlation L412. Either a 5th primary OR a richer DX over Stateless. **Not in Tech Spec.**

**DurableAction.** Replay-safe action with `ctx.now()`/`ctx.random()`/`ctx.uuid()` discipline, opting into Temporal-style replay. **Not in Tech Spec.** Either a 5th primary OR a marker-trait on existing primaries.

### §3.3 Verdict on completeness

The 4-primary + 5-DX shape is honest and well-grounded but **leaves three n8n / Temporal-prescribed shapes uncaptured.** Each is canon-§3.5-revision territory per ADR-0038 §2 — adding them is paradigm change, not amendment-in-place. **Phase 8 framing should explicitly disclaim or schedule these to keep §16.1 path (a/b/c) honest about "what Nebula commits to vs defers."**

---

## §4 Determinism + retry semantics

### §4.1 Temporal's retry policy model — Nebula adequacy

Temporal has `RetryPolicy { initial_interval, backoff_coefficient, maximum_interval, maximum_attempts, non_retryable_error_types }` declared at registration time. Failed activities retry per policy without author intervention.

**Nebula:** §2.7.2 `ActionResult::Retry { after, reason }` is **output-driven** — the author returns Retry from execute body. Engine consumes the variant per §7.4. No `RetryPolicy` declarative shape at registration. RetryHintCode (§2.8) is hint-only metadata.

**Verdict: 🟠 Adequate for hand-rolled retry; less ergonomic for declarative.** Author who wants "retry on rate-limit up to 5 times exponential backoff" must implement state-machine in execute body OR rely on engine's defaults (engine-cascade scope per §7.4 forward-track). Temporal-style declarative retry policy at `#[action(retry = ...)]` is absent. **Acceptable per Strategy §4.3.2 wire-end-to-end pick** but worth flagging in Phase 8.

### §4.2 n8n's retry pain points

n8n research L66-75 + forum 110925 — "Retry-on-Fail doesn't work for HTTP Request node." Class is integration-node-author-responsibility per L420 ("each author writes retry differently"). Nebula's typed `OnError` + ActionResult routing is structurally better. **🟢 Adequate.**

### §4.3 Saga rollback / compensation handling

**Tech Spec has NO Saga.** Already flagged in §2 above. **🔴 Significant gap if Nebula ever targets bulk-op or financial use cases.**

### §4.4 Idempotency key plumbing

Strategy §3.4 reserves `IdempotencyKey` cluster-mode hook on TriggerAction; deferred to engine cascade per §1.2 N4. **At action-execute-body level there is NO idempotency-key API.** Author must hand-roll dedup via state JSON (StatefulAction) or external store. n8n WooCommerce 20× amplification class (L188 + L26569) is unaddressed at action surface.

**🟠 Acceptable if cluster-mode cascade lands soon; gap persists otherwise.**

---

## §5 ResourceAction pool integration completeness

Q7 R2 restored ResourceAction `configure` / `cleanup` paradigm (§2.2.4) per production `crates/action/src/resource.rs:36-52`. ResourceAction is graph-scoped DI primitive — engine runs `configure()` before downstream nodes; downstream consumer actions read resource via `ctx.resource()`; engine calls `cleanup()` on scope exit.

### §5.1 Temporal's ResourceClient pattern

Temporal does NOT have a first-class ResourceClient / shared-pool primitive. Activities own their own clients (e.g., HTTP client per activity instance). Long-lived resources are user-managed via worker-side singletons. **Nebula's ResourceAction is structurally ahead of Temporal here.** Temporal's `temporal#9563` (sticky-queue blocked event loop) traces back to user-code blocking; Nebula's separation prevents that pattern.

### §5.2 n8n's connection pool pain points

n8n has NO connection-pool primitive. Each integration node opens its own HTTP/DB client. n8n research L382-393 memory leaks (`#27980` MongoDB Chat Memory leaks MongoClient instances) are direct consequence. **Nebula ResourceAction is structurally ahead.**

### §5.3 Pooling lifecycle (creation, eviction, sharing)

Tech Spec §2.2.4 R2 amendment:
- `configure(&self, ctx) -> Future<Self::Resource>` is creation. ✅
- `cleanup(&self, resource, ctx) -> Future<()>` is scope-exit consumption (resource is moved). ✅
- **Sharing across actions:** Tech Spec narrative says "consumer actions in the subtree borrow it via `ctx.resource()`" (§2.2.4 line 396-397). **Mechanism is graph-scope-bound** — resource lives for the subtree's lifetime; engine owns it. Cleanup runs once per configure call. ✅
- **Eviction:** No eviction policy in trait shape. Engine decides (engine-cascade scope per §1.2 N1 / §1.2 N4). **🟡 Acceptable deferral.**

### §5.4 Credential refresh interaction

Resource-credential composition: `Resource::Credential: Credential` per §2.2.4 line 384. `SchemeFactory<C>` per credential Tech Spec §15.7 line 3438-3447 — long-lived resources hold the factory; consumer actions ALWAYS acquire fresh `SchemeGuard<'a, C>` per request.

§1.2 N1 explicitly defers `Resource::on_credential_refresh` full integration to credential cascade. §1.2 N1-extended (Q7 I2) names hard ordering dependency: paths (b)/(c) MUST sequence credential cascade leaf-first.

**🟢 Adequate.** Resource-credential composition is well-thought-out and deferred-with-cascade-home.

### §5.5 Verdict on pool integration

**ResourceAction (post-Q7 R2) is the strongest piece of the Tech Spec relative to peer ecosystems.** Both Temporal and n8n lack equivalent typed surface. Eviction, refresh, sharing semantics are deferred-with-cascade-home (acceptable). **No 🔴 surfaced.**

---

## §6 §2.9 Input/Output reconsideration #6 (post-Phase 1)

§2.9 has been REJECTED 5 times across CP iterations + Q1+Q2+Q3 post-freeze rationale tightenings. Verdict: REJECT preserved with four-axis distinction (Method-Input/Output, Configuration, Trigger-purpose, Schema-as-data).

Phase 1 research evidence:

### §6.1 Does Temporal force typed Input/Output on all activity types?

Temporal Activity inputs/outputs ARE always typed at the SDK level — `async fn my_activity(ctx, input: T) -> Result<O, E>` is the signature in Go/Java/TS/Python SDKs. Workflow-side, `workflow.execute_activity(my_activity, input)` is generic over input/output. **Activity surface is uniformly typed.**

But Workflow (the orchestrating code) does NOT have a top-level `type Input / type Output`. Workflow methods are top-level Rust/Go/TS functions; the Temporal Worker registers them with their own input/output per registration call.

### §6.2 Does Temporal's workflow.Input differ from activity.Input?

Yes — there's `workflow.execute_activity(...)` with `activity.input` and `workflow.signal_workflow(...)` with `signal_payload`. Different lifecycle phases carry different input types. **Temporal does NOT consolidate them into a base trait.**

This is precedent for Nebula's §2.9 REJECT — Temporal recognizes input shapes diverge across activity vs workflow vs signal vs query lifecycle phases. The Tech Spec §2.9.1b Q2 trigger-purpose-input axis distinction matches this precedent.

### §6.3 Does activepieces-style typed pieces argue for trait-level Input?

Activepieces ([activepieces/activepieces](https://github.com/activepieces/activepieces)) is TypeScript-based; pieces author typed inputs via Zod schemas per piece. Schema is JSON-data at runtime, NOT TypeScript-generic at type level. Same as n8n correlation Q3 §2.9.1c — schema-as-data axis, not schema-as-trait-type axis. **Activepieces does NOT argue for trait-level Input.**

### §6.4 New axis surfacing?

**No new axis surfaces from n8n / Temporal research that would unblock §2.9 consolidation.** The four axes already in §2.9.1c (Method-Input/Output, Configuration, Trigger-purpose, Schema-as-data) cover the consumer landscape. **Verdict: §2.9 REJECT (refined three times) closure preserved.** No new evidence justifies sixth iteration.

**🟢 Closure documented.** Phase 8 should NOT re-open §2.9.

---

## §7 Top-N findings

The 10-15 most critical findings with full attribution. Severity-ordered.

### §7.1 🔴 critical

**§7.1.1 — `ItemLineage` typed surface is absent.** n8n research L48-54 + L235-249 + L353-358 + correlation L408. Nebula §2.5 ActionHandler enum and §7.4 ActionResult routing have no per-item provenance tracking. n8n's `pairedItem` class is one of the most-filed forum-error classes (5+ forum threads cited at L244-248). Nebula has structurally inherited the n8n pain class without addressing it. **Tech Spec FROZEN CP4 escalation candidate.** Either canon-state "Nebula does not commit to per-item lineage in v1" OR ItemLineage primitive lands as 5th primary or as ActionContext extension.

**§7.1.2 — Determinism contract is undeclared.** Temporal research L36-39 + L135-150 + correlation L283. Nebula ActionContext (spike L205-207) has no `ctx.now()` / `ctx.random()` / `ctx.uuid()`. Author can use `SystemTime::now()` and `rand::random()` freely. If Nebula ever moves toward replay (durable execution post-crash), determinism is a load-bearing tax. **Either canon-state "no replay" OR determinism contract lands.** Phase 8 framing should explicitly take a position.

**§7.1.3 — Sub-node typed traits (ChatModel, Memory, Tool) are absent.** n8n research L77-84 + L281-296 + correlation L412. AI Agent ecosystem is the fastest-churn class in n8n (`#24397`, `#21740`, `#28215`, `#26202`, `#27805`). Tech Spec has no surface for sub-node wiring. If Nebula targets AI workflows post-cascade, this is structural debt. **Out of cascade scope but deserves Phase 8 acknowledgement.**

**§7.1.4 — Per-action-type concurrency knob is deferred to engine cascade.** n8n research L314-321 + Temporal correlation L286 (`temporal#7666` 14 upvotes — Temporal still doesn't have it). Strategy §3.4 reserves cluster-mode `dedup_window` hook; per-action concurrency cap is **not at action layer**. n8n bulk-op amplification class (`#26569` WooCommerce 20× amplification) is unaddressed structurally. **Engine cascade absorption required for parity.**

**§7.1.5 — Streaming pipeline primitive is absent.** n8n research L181 + L382-393 + correlation L421. Memory leaks with large datasets (`#20124`, `#16862`, `#15269`) trace back to lack of bounded-channel backpressure. PollAction (§2.6) is event-by-event for triggers, not mid-graph stream operators. **5th primary candidate; canon §3.5 revision territory.**

**§7.1.6 — Saga / compensation surface is absent.** Temporal research has no Saga line; n8n has no compensation primitive. ResourceAction `cleanup` is per-resource-scope, not workflow-level rollback. ActionResult has no `Compensate` / `Rollback` variant. **Unaddressed; out of cascade scope but no cascade-home named.** If Nebula targets financial / bulk-op workflows, this is critical.

**§7.1.7 — Bulk-op idempotency at action surface is absent.** n8n WooCommerce 20× amplification (`#26569`). BatchAction §2.6 has `batch_size()` + `extract_items()` + `process_item()` + `merge_results()` — but NO idempotency-key plumbing per item. Author hand-rolls dedup via StatefulAction state. **🔴 Engine cascade scope per §1.2 N4 — but no cascade-home named for `BatchAction::idempotency_key()` per-item hook.**

**§7.1.8 — Workflow-version save-time pin migration is absent.** n8n research L56-64 + L208-218 + L325-334. Workflow pins type version at save time; n8n authors hand-roll `if (typeVersion < N)` branches. Nebula §13.1 pre-1.0 hard breaking changes; post-1.0 trait-level deprecation. **No per-action workflow-pin migration.** All running executions must finish on old version before deploy. Acceptable for Nebula's batch-deploy model but no executions-in-flight migration path documented. n8n correlation L411 named `migrate_v` as required at registration — Nebula's `migrate_state` (Q7 R1 §2.2.2) is for STATE only, not for parameters.

### §7.2 🟠 high

**§7.2.1 — Binary data primitive absent at trait level.** n8n research L86-94 + L86-94 + L344-352 + correlation L413. ActionOutput is `serde_json::Value`. Filesystem-fill (`#8028`) and exec-state-too-large (`#21746`) are direct consequences. n8n correlation L413 names content-addressed blob with reaper. **Out of cascade scope per acceptable deferral but deserves explicit cascade-home.**

**§7.2.2 — Provider-API drift / freshness is absent.** n8n research L359-379 + correlation L417. Nebula `#[action(version)]` (§4.6.2) is the action's own version, NOT a tracked provider-API version. Real-world (`#28660` LinkedIn deprecated, `#26071` Monday deprecated, `#28441` Todoist) integration drift is unaddressed at action surface. **Not blocking cascade but noteworthy.**

**§7.2.3 — Determinism re-emphasis — the cost.** Temporal NDE rate even at 5+ years SDK maturity (research L36-39) suggests determinism is genuinely hard — Nebula's choice to NOT commit to replay is principled per `docs/COMPETITIVE.md` line 41 ("typed Rust integration contracts + honest durability"). But the absence is currently silent. Phase 8 should make it explicit.

**§7.2.4 — Retry-policy declaration vs output-driven.** Temporal `RetryPolicy` at registration is more ergonomic than Nebula's `ActionResult::Retry`. **Acceptable per Strategy §4.3.2 wire-end-to-end pick.** But user-facing UX gap may surface at integration time — flag for Phase 8 / Phase 6 implementation.

### §7.3 🟡 medium / acknowledged

**§7.3.1 — Merge node fan-IN class.** n8n highest-churn node class (~14 issues at L86-94 / L395-399). Tech Spec ActionResult `MultiOutput` is fan-OUT. Fan-IN merge-by-position / merge-by-key is structurally absent at primary trait level. Fits awkwardly into Stateless/Stateful. **Engine-cascade or workflow-cascade scope; acceptable.**

**§7.3.2 — Continue-As-New / child workflow.** Temporal research L84 + L240. Engine-cascade scope per §1.2 N4. Acceptable deferral.

**§7.3.3 — Code node / scriptable action.** n8n research L99-134 + research correlation L407. Sandbox cascade per §1.2 N7. Acceptable deferral.

### §7.4 🟢 well-covered

**§7.4.1 — Cross-plugin shadow attack.** §6.2 hard removal of `CredentialContextExt::credential<S>()` per security 03c §1 VETO. Closes n8n research L91 (#27833 community-node credentials leaking). **Strongest piece of Tech Spec security floor.**

**§7.4.2 — ResourceAction DI lifecycle.** Q7 R2 paradigm restoration. Both Temporal and n8n lack equivalent typed surface. **Strongest piece of Tech Spec relative to peers.**

**§7.4.3 — Macro emission regression coverage.** §5 macro test harness + 6 probes ported from spike commit `c8aef6a0` + Probe 7 (CR8 `parameters = Type` fix). Closes "three independent agents repeating same emission bug" class. **CR2/CR8/CR9/CR11 all closed.**

**§7.4.4 — `*Handler` async-fn-in-trait per ADR-0024.** Q1 post-freeze amendment-in-place. `#[async_trait]` adoption aligns with workspace policy; mechanical migration when `async_fn_in_dyn_trait` stabilizes. Ecosystem-aligned (~15k crates) per user pushback.

**§7.4.5 — §2.9 four-axis Input/Output REJECT closure.** Temporal precedent confirms (workflow.input ≠ activity.input — different lifecycle phases). Phase 1 research surfaced no new axis. **No re-open trigger.**

---

## Summary statistics

- **n8n pains cataloged:** ~57 distinct classes (~150 underlying issues / forum threads)
- **Severity distribution:** 8 🔴 / 6 🟠 / 9 🟡 / 3 🟢
- **Temporal-comparison verdict:** Nebula and Temporal occupy structurally different positions (graph-edge state vs replay code) — alignment is honest. Three Temporal-prescribed primitives (determinism contract; idempotency-key; per-action concurrency) are missing-without-cascade-home and warrant Phase 8 framing.
- **Top 3 🔴:**
  1. **§7.1.1 ItemLineage typed surface absent** — n8n's worst forum-error class structurally inherited
  2. **§7.1.2 Determinism contract undeclared** — silent posture; Phase 8 should make it explicit
  3. **§7.1.3 Sub-node typed traits (ChatModel/Memory/Tool) absent** — fastest-churn n8n class structurally absent
- **Strongest pieces:** §6.2 hard removal of no-key credential heuristic (S-C2 / CR3); ResourceAction DI lifecycle (Q7 R2 restoration); §5 macro emission regression coverage; §2.9 four-axis REJECT closure
- **Re-open triggers identified for §2.9:** ZERO. Verdict closure preserved.

**Phase 2 architect synthesis hand-off scope:** the 8 🔴 + 6 🟠 findings above are the gap landscape. None invalidate Tech Spec FROZEN CP4 ratification path; all warrant Phase 8 framing as "what Nebula commits to vs defers" or canon-statements as escalation candidates. No Tech Spec amendment proposed by this Phase 1 read.
