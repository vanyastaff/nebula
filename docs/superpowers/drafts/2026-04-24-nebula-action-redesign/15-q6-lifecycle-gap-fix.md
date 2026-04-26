---
name: Q6 post-freeze gap fix — TriggerAction `start()` / `stop()` lifecycle methods missing from §2.2.3
status: SINGLE-PASS GAP FIX 2026-04-25 (post-freeze amendment-in-place; lifecycle-only, NOT bundled with Q4 reconsideration)
date: 2026-04-25
authors: [architect (drafting); user (gap surface)]
scope: 15th post-freeze item — production drift between `crates/action/src/trigger.rs:61-72` and Tech Spec §2.2.3 spike-locked shape; spike covered trait shape only, not lifecycle
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.2.3 lines 195-216 + §2.9 lines 484-682 + §15.9 lines 2461-2555
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/14-q4-trigger-input-asymmetry.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs lines 254-262
  - crates/action/src/trigger.rs lines 50-73 (production TriggerAction); lines 268-353 (production TriggerHandler)
---

# Q6 post-freeze — `start()` / `stop()` lifecycle gap fix

## §1 Honest framing — the slip

**Production code has lifecycle methods that the Tech Spec dropped without rationale.** Verification:

- `crates/action/src/trigger.rs:50-72` — production `TriggerAction` declares `start(&self, ctx)` + `stop(&self, ctx)` as required methods with `#[diagnostic::on_unimplemented]` enforcing both.
- `crates/action/src/trigger.rs:280-353` — production `TriggerHandler` (dyn-safe) carries the same `start` + `stop` plus two-shape semantics (setup-and-return vs run-until-cancelled), idempotency invariant, cancel-safety contract.
- `final_shape_v2.rs:254-262` — spike Probe 2 trait shape has `handle()` only, no `start()` / `stop()`.
- Tech Spec §2.2.3 (line 195-216) freezes the spike shape verbatim.
- Tech Spec §2.9 (line 530) post-freeze Q2 amendment writes "engine drives `start` → engine receives events on a channel → engine dispatches `handle(event)` per event" — but `start` is not defined anywhere in §2.

**Spike was shape-only.** Phase 4 spike NOTES record Probe 2 verifying `type Source: TriggerSource` is a required associated type via `error[E0046]: missing: Source`. Probe 2 did not exercise lifecycle. Iter-2 §2.2 compose test exercised body composition; Iter-2 §2.4 cancellation test exercised `tokio::select!` shape — neither invoked `start()` / `stop()`.

**Why this matters.** §2.2.3 today is implementable but is not a complete contract. An implementer porting `WebhookAction` from production cannot derive the start/stop signatures from §2.2.3. The drift is concrete: production webhook actions register their URL with GitHub/Slack/Stripe at `start()` time (per-instance secret + URL read from `&self`); without `start()` in the contract, that responsibility is unwritten.

## §2 The three concrete questions

### §2.1 Q1: Where do `start()` / `stop()` live?

User's option set:

- **(i) TriggerAction** — preserve production. `start(&self, ctx)` reads per-instance webhook URL/secret from `&self` (registered via `parameters = T` macro zone or zone-bound credential slots).
- **(ii) TriggerSource** — transport-level. `WebhookSource::start()` opens HTTP server; `KafkaSource::start()` connects to broker.
- **(iii) Split** — `TriggerSource` does transport bring-up; `TriggerAction` does action-specific registration (THIS URL with GitHub).
- **(iv) Other shape.**

#### Picked: Option (i) — `TriggerAction` carries `start()` + `stop()`

Concrete rationale:

1. **Per-instance state lives in `&self`, not in `TriggerSource`.** Production `WebhookAction` has fields like `pub webhook_url: String` and `pub secret: SecretString` — these are bound at registration time per the `&self` configuration carrier paradigm (§2.9.1a Resolution point 1, line 501). `TriggerSource` (post-redesign §2.2.3) is shape-only: `pub trait TriggerSource: Send + Sync + 'static { type Event: Send + 'static; }` — it is a type-level marker for the projected event payload, not a runtime carrier. Hoisting `start()` to `TriggerSource` would force `TriggerSource` to grow either runtime fields (paradigm break — `TriggerSource` becomes runtime-instantiated) or generic parameters threading per-instance state (signature explosion).
2. **Consumer-side migration is null.** Existing `WebhookAction` / `PollAction` / community trigger implementors already write `start(&self, ctx)` + `stop(&self, ctx)`. Option (i) preserves their impl bodies verbatim; the only change is the trait's place in the cascade-redesigned trait family (it stays on `TriggerAction`, gains `Source: TriggerSource` associated type from §2.2.3 spike-lock).
3. **Option (iii) split has appeal but no current consumer.** A Kafka consumer that opens a broker connection at transport layer + an action that registers its topic-handler at action layer reads cleanly as a split. But: today, no `TriggerSource` impl exists in production with transport-level lifecycle distinct from per-action registration. The webhook source today is monolithic with action-side URL registration. Adding split shape preemptively is speculative DX surface per `feedback_active_dev_mode.md` ("before saying 'we will need X', confirm X has a current consumer"). Future cascade can add `TriggerSource::start()` if a multi-action transport (e.g., one Kafka client servicing many trigger actions) materializes — that is canon §3.5 territory, not Q6 gap-fix scope.
4. **Option (ii) cannot work.** `WebhookSource` (transport-level) does not know per-instance secrets or URLs — those are in the action's `&self`. `WebhookSource::start()` would have to receive them as parameters, which collapses back to "action drives start()" with extra plumbing.

Final shape:

```rust
pub trait TriggerAction: Send + Sync + 'static {
    type Source: TriggerSource;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Start the trigger (register listener, schedule poll, etc.).
    ///
    /// Two valid shapes (mirrors production §2.2.3's TriggerHandler contract):
    /// - **Setup-and-return** — register external listener, return immediately
    /// - **Run-until-cancelled** — run trigger loop inline until ctx cancellation
    ///
    /// `start` must be paired with `stop`; calling `start` twice without an
    /// intervening `stop` returns `Self::Error` (Fatal-level; see §6.2).
    /// Cancel-safe at every `.await` point (per §6.4 cancellation invariant).
    fn start<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    /// Stop the trigger (unregister, cancel schedule).
    ///
    /// Clears any state set by `start` so a subsequent `start` is accepted.
    /// For run-until-cancelled shape, prefer cancelling `ctx.cancellation`
    /// to let `start()` exit cleanly.
    fn stop<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    /// Handle a single event projected from `Source`. Engine-driven —
    /// the engine sources events from `Source: TriggerSource` and
    /// dispatches each into `handle`.
    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

**Differences from production `crates/action/src/trigger.rs:61-72`:**

- Receiver context: `&'a ActionContext<'a>` per Tech Spec §2 lift-onto-ActionContext (production uses `&(impl TriggerContext + ?Sized)`). This is the cascade-wide pattern — credential resolution and event dispatch funnel through `ActionContext`. Source-of-truth: §2.2.1 / §2.2.2 / §2.2.4 already use `&'a ActionContext<'a>`.
- Lifetime form: single-`'a` RPITIT shape per §2 cascade pattern (production uses `impl Future + Send` with implicit lifetime). Cascade-wide modernization per Strategy §4.3.1.
- `handle()` added per §2.2.3 spike Probe 2 — was absent from production; production wires event delivery through `TriggerHandler::handle_event` (lines 373-389) with type-erased `TriggerEvent` envelope. Cascade replaces the type-erased envelope with `<Source as TriggerSource>::Event` projection per §2.2.3 spike-lock. **No regression** — production already supports event-driven dispatch through the type-erased envelope; cascade typifies it.

### §2.2 Q2: Migration path for existing `WebhookAction` / `PollAction` implementors

**Concrete impl-body deltas per existing implementor:**

| Implementor | Today (`crates/action/src/trigger.rs:517-527` mock pattern) | Post-cascade |
|---|---|---|
| `WebhookAction` (community plugin) | `async fn start(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> { /* register URL with GitHub */ }` | `async fn start<'a>(&'a self, ctx: &'a ActionContext<'a>) -> Result<(), Self::Error> { /* register URL with GitHub */ }` |
| `PollAction` (run-until-cancelled) | `async fn start(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> { /* poll loop with tokio::select! */ }` | Same body, lift signature per cascade pattern |

**Codemod transform shape — T7 (NEW).** Adds to §10.2 codemod transforms table:

| Transform | What it rewrites | Source pattern | Target pattern | Auto / Manual | Notes |
|---|---|---|---|---|---|
| **T7** | `TriggerAction` lifecycle method signatures | `async fn start(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError>` (and `stop` mirror) | `async fn start<'a>(&'a self, ctx: &'a ActionContext<'a>) -> Result<(), Self::Error>` | **AUTO** for trivial signature lift; bodies pass through unchanged | Token-level rewrite of receiver context type + lifetime form; `Self::Error` rewriting depends on whether implementor declared `type Error` (existing T1 zone-injection captures the macro-side; T7 covers manual impls) |

**Impl-body coverage.** T7 lifts signatures only — bodies pass through unchanged because:

- Per-instance state reads (`self.webhook_url`, `self.secret`) work identically before and after — `&self` field access is unchanged.
- External-service calls (GitHub register, Slack subscribe, Kafka consumer init) are body content, not signature.
- Error type lifting (`ActionError` → `Self::Error`) is a §2.2.3-cascade-wide pattern — implementors who already declare `type Error` see no body change; implementors who don't (rare; production trait lifted `ActionError` directly) get a typed-error refactor that is `feedback_hard_breaking_changes` acceptable per Strategy §4.3.3.

**Reverse-dep impact estimate.** From §10.3 per-consumer table:

- `nebula-engine` — TriggerHandler dispatch sites unchanged at the engine boundary (engine sees `Arc<dyn TriggerHandler>`, see §2.4 post-Q1 `#[async_trait]` form per §15.9.1); ~0 T7 sites.
- `nebula-api` — webhook transport calls handler boundary, not action trait directly; ~0 T7 sites.
- `nebula-plugin` — test fixtures may include trigger action mocks; ~1-2 T7 sites estimated.
- `apps/cli` — CLI dev fixtures may include trigger actions; ~1-2 T7 sites estimated.
- Community plugins — every webhook/poll trigger action incurs 1 T7 (per impl). Migration-guide step added in §10.4.

**Aggregate T7 footprint:** ~3-5 internal sites + ~1 per community trigger plugin. Well within cascade migration discipline.

### §2.3 Q3: Spike artefact handling

**Decision: note lifecycle out of spike scope; PASS verdict for shape-only validation stands.** Concrete handling:

- Spike `final_shape_v2.rs:254-262` is the **trait-shape signature-locking source per §0.1 inputs frozen-at footer**. The spike validated `type Source` and `handle()` projection composition. It did NOT validate `start()` / `stop()` because Probe 2 was scoped to `error[E0046]: not all trait items implemented, missing: Source` — a missing-associated-type compile-fail probe, not a lifecycle composition probe.
- **No re-spike.** The lifecycle methods lift cleanly from production `crates/action/src/trigger.rs:61-72` — production code is already evidence the shape compiles and runs (Mock test at line 517-527 PASS). Re-spiking would duplicate evidence already in production.
- **§0.1 inputs frozen-at footer amendment.** Spike PASS verdict scope is narrowed in place: "spike PASS at commit `c8aef6a0` (worktree-isolated; trait-shape compose + cancellation; lifecycle methods derived from production `crates/action/src/trigger.rs:61-72`, NOT spike-validated)."

**Why this is honest.** The spike's PASS verdict was always shape-and-composition scope; lifecycle was never a probe. Q6 surfaces that scope boundary explicitly rather than retroactively expanding the spike. Per `feedback_active_dev_mode.md` ("more-ideal over more-expedient"), the more-ideal shape is to acknowledge the scope boundary; the more-expedient shape would be to claim spike covered lifecycle when it did not.

## §3 Bundle decision — SPLIT, not bundle

User's bonus connection raised: «Then we can add also Input to start method to setup start configuration» — `start(&self, ctx, input: Self::Input)` parameterizing `type Input` on `TriggerAction`.

**Decision: SPLIT.** Q6 lifecycle-only fix; no Q4 re-litigation. Concrete rationale:

### §3.1 Q4 blockers under `start(input: Self::Input)` framing

| Blocker | Status under `start(input)` |
|---|---|
| **B1 — silent semantic divergence** (same name, opposite meaning) | **STILL HOLDS.** `start(input: Self::Input)` is called once per registration; `StatelessAction::execute(input: Self::Input)` is called per dispatch. Same name carries opposite frequency semantics (lifecycle-once vs per-dispatch-many). A reader seeing `TelegramTrigger::Input = TelegramTriggerInput` and `StatelessSlackSend::Input = SlackSendInput` would assume identical lifecycle role; they would be wrong. The name-collision trap from Q4 §14 (line 103) survives the start(input) refinement. |
| **B2 — signature-doubling** (parallel to `&self` + `parameters = T`) | DISSOLVES. If `start(input: Self::Input)` IS the configuration entry, then configuration is method-bound, not also `&self`-bound. (But: `&self` still carries spawned-task handle, atomics, internal state — so partial `&self` retention. Not a clean dissolution.) |
| **B3 — decorative (no method-signature carry)** | DISSOLVES. `start(input)` puts Input in the method signature. |
| **B4 — ADR-0038 binds verbatim spike shapes** (line 209-262) | **STILL HOLDS.** Spike `final_shape_v2.rs:254-262` has no `type Input` on TriggerAction. Adding would invalidate freeze per §0.2 invariant 4 (spike-shape divergence trigger). Re-validation would require new spike work. |
| **B5 — §2.9.1a paradigm contradiction** ("Configuration carrier is `&self` ... no new associated type") | Partially HOLDS. Even with `start(input)`, per-instance runtime state (atomics, task handles, registration tokens) still lives in `&self`. The paradigm "configuration in `&self` fields, populated at registration" is partially preserved (stable runtime state) and partially disrupted (input-typed configuration moved to method). Mixed paradigm is harder to teach than single paradigm. |

**B1 + B4 alone are sufficient blockers.** B1 is the load-bearing semantic trap (unchanged from Q4 verdict on iteration 4 — REJECT). B4 invalidates the freeze without re-spike work.

### §3.2 Why split, not bundle

1. **Q4 was a single-round REJECT closed 2026-04-25** ([14-q4-trigger-input-asymmetry.md](14-q4-trigger-input-asymmetry.md) Verdict + amendment trail). Re-opening it bundled with Q6 conflates two distinct decisions: lifecycle gap (mechanical drift between production and spec) vs paradigm choice (`type Input` semantic on TriggerAction). Bundling muddies the freeze-warrant trail — a future contributor reading "FROZEN CP4 (amended-in-place 2026-04-25 — Q1 + Q6)" cannot tell whether Q6 was "lifecycle preserve" or "lifecycle preserve + Input ratification."
2. **B1 survives under start(input).** The bonus user note "Then we can add also Input to start method" reframes Q4 but does not dissolve the load-bearing blocker. Q4 verdict (REJECT) stands; bundle would be reversal without B1 dissolution.
3. **B4 invalidates freeze without re-spike.** Per §0.2 invariant 4, a different shape required at signature-locking source (`final_shape_v2.rs:209-262`) invalidates freeze. Adding `type Input` to TriggerAction is exactly that invalidation. Lifecycle-only fix preserves the spike-locked shape (no `type Input` added; `start()` / `stop()` are derived from production, not from spike-shape change).
4. **Lifecycle gap is a mechanical drift fix** (production has start/stop; spec dropped them; production wins per cross-source-authoritative discipline + ADR-0035 amend-in-place precedent — see §15.9.2 "canonical-form correction" criterion). Bundling Q4 ratification turns a mechanical fix into a paradigm decision.
5. **No status qualifier change for the split-or-bundle question.** Q6 is structural amendment (§2.2.3 signature change adding methods), so it earns a status qualifier per §15.9.5 / §15.9.6 precedent. Q4 reconsideration would be a separate qualifier IF it had a verdict change — which it does not under split.

### §3.3 If user contests SPLIT decision

Single-round budget is exhausted on the bundle question. If user contests, escalation paths:

- **B1 dissolution evidence.** User produces a name-distinction mechanism that disambiguates per-dispatch-Input from lifecycle-Input semantically (e.g., explicit rename `type DispatchInput` vs `type ConfigInput`). Q5 already explored this with `type Config` rename — REJECT on B5 paradigm contradiction (§14 line 230 record).
- **B4 re-spike commitment.** User commits to a new spike validating `type Input` on TriggerAction with `start(input)` shape compiling end-to-end. Acceptable but adds cascade-day cost and re-opens spike validation gate.
- **Tech-lead ratification.** Cascade prompt explicitly authorized "Architect's call" — split is the call. Tech-lead may revisit.

## §4 Amendment shape (ENACTED in this gap-fix)

### §4.1 Tech Spec §2.2.3 amendment-in-place

Replace lines 195-216 with the four-method shape (TriggerSource declaration unchanged; TriggerAction gains `start` + `stop` adjacent to `handle`). Cite §15.10 as enactment record (new sub-subsection added below).

### §4.2 Tech Spec §15 — close lifecycle-gap as new §15.10

§15.10 records:
- Lifecycle gap surfaced (production has start/stop; spec dropped them; spike was shape-only).
- Pick: Option (i) — `TriggerAction` carries lifecycle methods.
- Bundle decision: SPLIT (Q4 reconsideration deferred; B1 + B4 still bind under start(input)).
- §0.1 inputs frozen-at footer narrowed (spike PASS scope clarification).

### §4.3 Tech Spec §10 — codemod runbook addition

§10.2 transforms table gains T7 row (TriggerAction lifecycle signature lift, AUTO). §10.3 per-consumer step counts gains a "T7" column with estimates (~1-2 sites internal, ~1 per community trigger plugin). §10.4 plugin author migration guide steps 1-7 gain a sub-step under step 5: "T7 markers (if any): pass-through; bodies unchanged."

### §4.4 ADR-0038 amendment-in-place — NOT REQUIRED under split

Per ADR-0038 §Decision item 4 ("Pattern composition with ADR-0035") and §Neutral block last bullet ("Public API surface of the 4 dispatch traits ... unchanged at the trait level"), the trait-shape decision in ADR-0038 governs:
- Macro emission contract (item 1: rewriting scope; item 2: emission contract; item 3: dual enforcement layer; item 4: ADR-0035 phantom composition).
- ADR-0038 does **NOT** lock the per-method signatures of the 4 dispatch traits (that is ADR-0038 §Neutral block's "Public API surface ... unchanged at the trait level" — meaning the 4-trait family enumeration is preserved; not that every method signature is verbatim ADR-0038 lockdown).
- Tech Spec §2.2 is the per-method signature lock; spike `final_shape_v2.rs:209-262` is the signature-locking source per §0.1 inputs frozen-at footer.

**Lifecycle gap fix is Tech Spec amendment-in-place per §15.9 precedent** — it does not amend ADR-0038. ADR-0038 §Decision items 1-4 are unchanged. The Tech Spec § 2.2.3 signature change is a **§0.2 invariant 4 trigger** (spike-shape divergence; the spike did not have lifecycle methods, lifecycle methods are now added to §2.2.3) — but the divergence justification is documented (spike was shape-only; production has lifecycle; lifecycle is derived from production not from spike-shape change). This is the same discipline §15.9.1 used for `*Handler` `#[async_trait]` adoption (post-amendment shape derived from ADR-0024 and production code, not from new spike work).

**No ADR amendment.** Skip ADR-0038 file edit.

### §4.5 Status header qualifier

Append `(amended-in-place 2026-04-25 — Q6 lifecycle gap)` to status header per ADR-0035 amend-in-place precedent + §15.9 / §15.9.5 / §15.9.6 precedent for structural amendments. Q6 is structural (signature change); earns a status qualifier per §15.9.5 / §15.9.6 precedent ("rationale-tightening amendments without signature ripple do not warrant a separate status header qualifier" — Q6 has signature ripple, so it does).

Final status header form: `FROZEN CP4 2026-04-25 (amended-in-place 2026-04-25 — Q1 post-freeze; amended-in-place 2026-04-25 — Q6 lifecycle gap)`.

## §5 Summary

**Outcome: SPLIT lifecycle-only gap fix.** No Q4 reconsideration; B1 + B4 still bind under start(input) bundle framing.

**Design choice: Option (i)** — `TriggerAction` carries `start(&self, ctx)` + `stop(&self, ctx)` adjacent to existing `handle(.., event)`. Preserves production shape; per-instance state stays in `&self`; no `TriggerSource` runtime-instantiation paradigm break.

**Migration impact one-liner:** ~3-5 internal sites + ~1 per community trigger plugin; new codemod transform T7 (AUTO signature lift; bodies pass through unchanged); reverse-dep impact null on `nebula-engine` boundary (engine sees `Arc<dyn TriggerHandler>`, internal handler shape change is sealed behind `#[async_trait]` per §15.9.1).

**Bundle-vs-split rationale:** SPLIT. Q4 verdict (REJECT) stands; B1 (silent semantic divergence) survives the start(input) refinement; B4 (ADR-0038 binds spike shapes; Q4 bundle would invalidate freeze without re-spike) holds; bundling conflates mechanical drift fix with paradigm choice and muddies freeze-warrant trail. Architect's call per cascade prompt; tech-lead may revisit if user contests.

**No ADR amendment required.** ADR-0038 §Decision items 1-4 unchanged; lifecycle is Tech Spec §2.2.3 amendment-in-place per §15.9 precedent.

**Status qualifier added** per §15.9.5 / §15.9.6 precedent for structural amendments.
