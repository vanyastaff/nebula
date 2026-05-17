# Design Conference Notes — May 2026

> Companion document to [VISION.md](./VISION.md). Captures the four-day design
> conference (simulated roundtable with industry veterans) that produced the
> charter. Use this as an **index** to traceability — what each decision came
> from, who weighed in, what was rejected and why.

**Format note:** Reactions attributed to named individuals are *literary
approximations* of their publicly known stances and writing. None of the
quotes are real direct quotations; they synthesize public talks, blog posts,
RFC discussions, and well-known engineering style.

---

## Sessions overview

| Day | Topic | Headline outcome |
|---|---|---|
| 1 | Action surface DX (Concept A vs B vs C) | **Concept A-modified** — registry-owned metadata, `StaticMetadata` removed |
| 2 morning | Concrete examples | Five canonical authoring patterns documented |
| 2 afternoon | `result.rs` / `output.rs` / `port.rs` | Per-shape `*Outcome` enums; `OutputEnvelope` always; ports stay in metadata |
| 3 morning | Strategic vision | Six architectural principles; three target profiles; 18-month roadmap |
| 3 mid | WASM decision | **Deferred to 2028+** — preserve full crates.io ecosystem access |
| 3 evening | `nebula-sdk` facade | Single Nebula dependency; feature flags; `register_into` plugin convention |
| 4 | Predecessors speak | Nine veteran-led suggestions added to backlog (B-01 to B-09) |
| 5 morning | Foundation Five (`schema`/`credential`/`action`/`resource`/`plugin`) | Five foundation principles (F1-F5); per-crate action items; cross-cutting consistency rules |
| 5 evening | Schema as form + Cross-foundation dependency graph | F6 (schema is the form), F7 (typed dependency graph); Action/Resource/Credential dependencies via slot fields with cycle detection |
| 5 late | Symmetric API (Tokio maintainers) | F8 (symmetric API), F9 (tokio-grade naming); `Acquirable` trait, `Handle<T>` alias, single `#[require(...)]` attribute |
| 5 breakfast | Modifier wrappers | `Resolvable` trait family for `Option<H>` / `Lazy<H>` / composition; conscious non-support of `Vec<Handle<X>>` etc. |
| 6 morning | `nebula-schema` deep design review | F10-F14 proposed (schema as data, JSON Schema 2020-12 superset, extension discipline, newtype flagship, all-errors-at-once) |
| 6 mid-afternoon | Three corrections (validator/expression siblings, drop custom widgets, rethink extensions) | F12 refined (no custom widgets), F15 (three siblings: schema+validator+expression), F16 (closed-set rule) |
| 6 late-afternoon | Honest reckoning — audit existing `crates/schema/src/` (24 modules, 11K LOC) | 95% of proposals already implemented; F13 clarified (stdlib module, not separate crate); refined backlog NS-1 to NS-9 |
| 6 evening | UI form composition — schema vs slot bindings | F17 (two channels: schema + slot bindings), F18 (three-layer slot binding); two-panel editor rendering; ADR-0064 |
| 7 | Resource topology & multi-agent extensibility | F19 (author API decoupled from visual presentation), F20 (default hidden + opt-in visible); Hidden + Inspector default, Pattern B (canvas nodes) opt-in, multi-agent auto-promotion (3+ consumers); ADR-0065 |
| 8 | Reckoning — charter consistency audit | ADR-0052 superseded by ADR-0066 (Concept A-modified ratified, factory layer survives as engine-internal); F1-F6 formally defined; 0053-0065 triaged into Accepted / Stay-Proposed batches |

---

## Day 1 — Action surface (the trigger for everything)

**Problem:** plugin author must write 4 impl-blocks for the simplest stateless
action: `DeclaresDependencies`, `Action` with delegating `metadata`,
`StaticMetadata` companion with `OnceLock` ritual, then the actual
`StatelessAction`.

**Three concepts proposed:**
- A — Owned metadata via registry, drop `StaticMetadata`
- B — Derive-only (forbid manual implementations)
- C — Tower-style `Service<ActionRequest>` unification

**Vote:** A-modified (10/12); B and C rejected for legitimate manual-impl use cases and for breaking single-method elegance.

**Decisive arguments:**
- Carl Lerche (Tokio) — runtime cost analysis: owned metadata cloned **once
  at register**, not per dispatch
- Niko Matsakis — type-system: companion trait is leak; instance method
  preserves dyn-compat without it
- Alice Ryhl — junior-developer DX test: 30 minutes vs 5 minutes
- David Tolnay — cited tower/serde precedent

**Rejected for v1.x:**
- withoutboats's "drop `Action` trait entirely" — legitimate but compile
  errors get worse without marker trait

---

## Day 2 morning — Five canonical examples

Concept A-modified validated against five sample actions:

1. **Function-style** `#[action] async fn hello(name: String) -> Result<String, ActionError>` — 4 lines
2. **Struct-style HTTP** with `impl StatelessAction` — 12 lines
3. **Paginated derive** with `#[derive(Action)]` + `impl PaginatedAction` — 20 lines
4. **Slot-binding** Telegram with `#[resource]` / `#[credential]` field
   attributes — 25 lines
5. **Webhook** with HMAC signature config — 15 lines

**New decisions:**
- Type alias `CredentialFor<C>` to hide
  `CredentialGuard<<C as Credential>::Scheme>` — Esteban Küber
- `#[diagnostic::do_not_recommend]` on blanket `Action` impls — Niko + Esteban
- `register_module` API for plugins shipping multiple actions — Maxim Fateev
  (deferred — backlog)

---

## Day 2 afternoon — Result/Output/Port redesign

### `result.rs` — Per-shape outcomes

**Old:** single `ActionResult<T>` enum with 9 variants — `Success`, `Skip`,
`Branch`, `MultiOutput`, `Continue`, `Break`, `Wait`, `Stop`, `Fail`.

**New:** four shape-specific enums; engine-level concerns lifted out:

```rust
pub enum StatelessOutcome<T>      { Success(T), Skip { reason } }
pub enum StatefulOutcome<T>       { Continue { … }, Break { … } }
pub enum ControlOutcome           { Branch { port, output }, Multi, Drop, Pass }
pub enum TriggerEventOutcome      { Emit(Value), Drop, Defer { until } }
```

`Wait` and `Stop` move to engine layer (orchestration intent, not action return).

**Vote:** 11/12. Tolnay voted "fine, but I liked the convenience".

### `output.rs` — Always-envelope

**`OutputEnvelope<T>`** as universal carrier. Defaults available; observability
fields (cost, tokens, timing, origin) opt-in via builder chain.

**`bytes::Bytes` specialization** for byte streams (Carl Lerche, Fabrice
Bellard) — refcount-shared, zero-copy.

**`AsyncIterator` migration** — gated until Rust 1.95+ stable check (Niko
Matsakis caveat).

**`Deferred + Stream` mutually exclusive** via `PhantomData` enforcement
(Maxim Fateev — composition undefined).

### `port.rs` — Ports as metadata, not trait API

Ports live in `ActionMetadata`, not as trait methods. `ConnectionFilter`
serves three layers (UI editor, validator, runtime) — single source of truth
(Carl Lerche).

`SupportPort` → `OutputPort { kind: Diagnostic }` rename (boats + Esteban
naming).

---

## Day 3 morning — Strategic vision

**Six architectural principles ratified** (see VISION.md §3).

**Three target profiles** identified — single core, different SDK shapes:
- Simple API integration (4-line Hello World)
- AI agent orchestration (streaming, dynamic DAG)
- Factory DevOps (durable, sandboxed, observable)

**Competitive landscape audit** — pain points enumerated:
- n8n — runtime crashes at scale (TypeScript dynamic typing)
- Airflow — scheduler bottleneck, no compile-time DAG validation
- Temporal — alien programming model, runtime-enforced determinism
- Argo — YAML hell
- Restate — TypeScript-first, less integration ecosystem
- Zapier — closed, no extensibility

---

## Day 3 mid — WASM decision

**Decision: defer WASM to 2028+.**

**Rationale (product-side):**
- WASI preview3 not GA
- Critical crates lack WASM target: `tokio` (browser-only), `reqwest`
  (limited), `sqlx` (none), `aws-sdk-*` (partial), Kafka clients (none)
- Plugin author choosing Nebula loses 60% of crates.io ecosystem
- Headline message must be: **"any crate from crates.io works in your action"**

**Architecturally compatible:** when WASI preview3 GA + ecosystem mature,
adding WASM as Tier-4 sandbox does not break Tiers 1-3.

**Pat Hickey (Wasmtime) accepted:** "Come back in 2027-2028."

---

## Day 3 evening — `nebula-sdk` facade

**Plugin author and integrator depend on a single Nebula crate:** `nebula-sdk`.

Internal crates (`nebula-action`, `nebula-engine`, `nebula-credential`,
`nebula-resource`, `nebula-workflow`, `nebula-storage`, `nebula-execution`,
`nebula-schema`) are **implementation details** re-exported through facade.

Pattern: `tokio` umbrella, `serde` re-exporting `serde_derive`, `axum`
re-exporting `axum-core` + `axum-macros`.

Plugin distribution: ordinary cargo crate exposing `register_into(&mut
ActionRegistry)`. No special plugin packaging, no private registry, no
dynamic loading.

See VISION.md §4 for full facade content + feature flags.

---

## Day 4 — Predecessors speak

**Special session.** Industry veterans of competing systems shared
hard-won lessons. Each contributed actionable suggestion now logged in
charter backlog.

| Veteran | Built | Key lesson | Backlog ID |
|---|---|---|---|
| Jan Oberhauser | n8n | Type safety at compile time is enterprise-readiness | (already in P1) |
| Jeremiah Lowin | Prefect | OSS-first commitment from day 1 | Process commitment |
| Nick Schrock | Dagster | `AssetMaterialization` events for lineage | **B-06** |
| Maxim Fateev | Temporal | Workflow versioning via `MigratesFrom` trait | **B-08** |
| Matei Zaharia | Apache Spark | Lazy DAG + planner stage = future optimization slot | **B-01** |
| Wes McKinney | pandas / Arrow | Apache Arrow as data plane between nodes | **B-02** |
| Harrison Chase | LangChain | Typed agent tools as first-class | **B-03** |
| Jerry Liu | LlamaIndex | Token budget tracking in OutputMeta | **B-04** |
| Joao Moura | CrewAI | Multi-agent via workflow composition (`run_workflow!`) | **B-09** |
| Mike Perham | Sidekiq | Built-in OSS dashboard | **B-05** |
| Linden Tibbets | IFTTT | Recipe mode for non-developer surface | **B-07** |

**Genuine support pledges** for cross-promotion when v1.0 ships:
Jan Oberhauser (blog post), Harrison Chase (LangChain adapter), Jerry Liu
(LlamaIndex integration example), Joao Moura (CrewAI scenario port),
Mike Perham (mentorship on commercial roadmap), Matei Zaharia (Stanford
class benchmarking).

---

## Day 5 — Foundation Five (морning + evening + late + breakfast)

### Day 5 morning — `nebula-schema` / `nebula-credential` / `nebula-action` / `nebula-resource` / `nebula-plugin`

**Frame:** ffmpeg-style commitment. Five foundation crates designed for
ten-year longevity. Apache-2.0/MIT dual license, zero patent traps,
plugin-friendly, library-first (each crate usable standalone).

**Five Foundation Principles ratified (F1-F5 in VISION.md §3):**
- F1 — Foundation crates are libraries, not frameworks
- F2 — Zero patent traps, forever
- F3 — Trait-only by default, engine-owned only where necessary
- F4 — Observable by construction
- F5 — Idiomatic Rust 2024+, never compromises

**New voices added:** Tony Arcieri (RustCrypto), Cart (Bevy plugin
system), Aleksey Kladov / matklad (rust-analyzer architecture), Sean
McArthur (hyper / reqwest), Ari Seyhun (schemars), Stjepan Glavina
(smol).

**Per-crate action items registered:** S-1 to S-5 (schema), C-1 to C-7
(credential), A-1/A-2 (action), R-1 to R-5 (resource), P-1 to P-5
(plugin). Cross-cutting X-1 to X-5. See VISION.md §9 backlog.

### Day 5 evening — Schema is the form + Dependency graph

**Two missed aspects surfaced by product side:**

1. **`nebula-schema` doubles as universal form definition** for Action
   inputs, Resource configurations, Credential properties. Same schema
   serves type safety + runtime validation + UI form generation +
   documentation generation. Field attributes (`#[field(...)]`) are
   standard vocabulary.

2. **Cross-foundation dependency graph** — Action depends on {Resource,
   Credential}, Resource depends on {Resource (composition!), Credential
   (ADR-0044)}, Credential depends on Credential (derived chains for
   OAuth refresh, AWS STS).

**F6 ("Schema is the form")** and **F7 ("Dependencies as a typed graph,
validated at registration")** ratified.

**Action items:** S-6 to S-10 (extended `#[field]` vocabulary, form
crate split, custom widgets, CLI form-preview, secret-string diagnostic),
C-8/C-9 (derived credential chains + scope enforcement), R-6 to R-8
(resource composition, on_failure policy, type/string-keyed dependency
forms), X-6 to X-9 (cycle detection, numbered errors, topological init,
shared derive infrastructure).

**Forthcoming ADRs scheduled:** 0058 (`#[field(...)]` vocabulary), 0059
(cross-foundation dependency graph + cycle detection).

### Day 5 late — Symmetric API (Tokio maintainers special session)

**Pizza-and-Tokio session.** Carl Lerche, Alice Ryhl, Eliza Weisman
joined to address authoring asymmetry: `ResourceGuard<R>` vs
`CredentialFor<C>` vs `<C as Credential>::Scheme` projection — different
words for structurally identical concepts.

**Outcome:** F8 ("Symmetric API surface") and F9 ("Tokio-grade naming
discipline") ratified.

**Decisions:**
- Single `Acquirable` trait with blanket impls on `Resource` and
  `Credential`; `Handle<T>` type alias resolves via
  `<T as Acquirable>::Handle`.
- Single `#[require("key")]` attribute; kind inferred from type.
- `ctx.acquire("key")` unified method.
- Drop `CredentialFor` alias and `<C as Credential>::Scheme` from
  author-facing API.
- Separate `#[resource]` / `#[credential]` attributes kept as
  explicit-form for emphasis cases (dtolnay's compromise).
- `AcquireFailure` trait shared between `ResourceError` /
  `CredentialError` for cross-kind retry semantics.
- Standardized observability: `target = "nebula::acquire"` with `kind`
  field.
- Sealed pattern (or `negative_impls` when stable) prevents one type
  implementing both `Resource` and `Credential`.

**Hard breaking change** for plugin authors. Acceptable pre-1.0.
Migration table in VISION.md §5.

**Action items:** Y-1 to Y-7. **Forthcoming ADR:** 0060 (Symmetric
Foundation API).

### Day 5 breakfast follow-up — Modifier wrappers

**Question from product side:** how do `Optional` / `Lazy` modifiers
work in the new symmetric world?

**Decision:** four base modifier combinations supported via `Resolvable`
trait composition (inherited from ADR-0043 and applied symmetrically to
both Resource and Credential):

| Type | Semantics |
|---|---|
| `Handle<X>` | required + eager |
| `Option<Handle<X>>` | optional + eager |
| `Lazy<Handle<X>>` | required + lazy |
| `Option<Lazy<Handle<X>>>` | optional + lazy |

**Conscious non-support in v1.x:** `Vec<Handle<X>>` (use multiple field
declarations or fan-out resource), `Refresh<H>` (becomes `Refreshable`
trait impl on Credential, not a wrapper), `Pooled<H, N>` (resource
topology in `nebula-resource`, not action wrapper), `Cached<H>` /
`Failover<[H; N]>` (niche, defer). All produce typed compile errors via
`#[diagnostic::on_unimplemented]` on `Resolvable` with helpful
suggestions.

**Action items:** Y-8 to Y-11. Folded into ADR-0060 expanded scope.

---

## Day 6 — `nebula-schema` deep design (four sub-sessions)

### Day 6 morning — Initial deep design review

**Frame:** `nebula-schema` is foundation-of-foundations (used in Action
input, Resource config, Credential properties, Workflow input, Output
meta). Eight rounds covering core trait design, field attributes,
validation rules, newtype patterns, expression placeholders, JSON
Schema interop, extensibility, performance.

**New voices:** Henry Andrews *(JSON Schema spec author)*, Ari Seyhun
*(schemars maintainer)* — anchored discussion to JSON Schema 2020-12.

**F10-F14 proposed:** schema-as-data, JSON-Schema-superset, extension
discipline, newtype flagship, all-errors-at-once.

### Day 6 mid-afternoon — Three product-side corrections

User-side correction prompted re-examination:
1. **`nebula-validator` and `nebula-expression` are sibling Core-layer
   crates**, not optional contrib. They form a tightly coupled subsystem
   with `nebula-schema`.
2. **Custom widgets dropped** — security (XSS injection vector), UX
   consistency, CSP-hardening. Cart (who initially proposed them)
   accepted Bevy precedent: closed-palette inspector widgets are the
   right pattern.
3. **Extension families reconsidered** — discipline: "extension only if
   contract evolves AND not strictly required for typical user".

**F12 refined** (validators only extensible; widgets/renderers/formats
closed-set), **F15** (three-siblings principle), **F16** (closed-set
extension surfaces overarching rule).

Render crates (HTML/React) moved to **`nebula-editor` separate
product** scope.

### Day 6 late-afternoon — Honest reckoning

Moderator audited actual `crates/schema/src/` and discovered the crate
is **already production-grade** (24 modules, 11,261 LOC, 13 Field
variants in closed enum, 20-entry InputHint, three-tier proof tokens
via type-state, `Mode` field for discriminated unions, Loader trait,
Secret family with Argon2 KDF, JSON Schema export via `schemars`,
lint pass, validator/expression bridge — all already shipped).

**95% of morning proposals turned out to be already implemented.**

Reactions:
- Henry Andrews — proposed bringing `Mode` field design to JSON Schema
  community (cleaner than `oneOf` magic).
- Niko Matsakis — three-tier proof tokens labeled "academic-grade
  design that actually shipped"; conference-talk material.
- Cart — withdrew "custom widgets" suggestion completely after seeing
  closed `Widget` enum; recognized Bevy precedent.
- Tony Arcieri — withdrew `Drop`/AAD audit items (likely already
  correct in `secret.rs`).

**F13 clarified**: `stdlib` module (Email/Url/Cron newtype zoo) ships
inside `nebula-schema` as default feature, not separate crate.

**Backlog drastic reduction**: SC-7/13/14/15/17/19 removed (already
implemented); refined NS-1 to NS-9 (real gaps + documentation polish).

**Charter outcome:** "Crate is production-grade today. Tier-1: add
stdlib module, document flagship features, polish lint diagnostics.
Tier-2: file split for `validated.rs` (1943 LOC). Ship."

### Day 6 evening — UI form composition

**Frame:** n8n collapses credential/resource selection into form schema
(each node has `credential` property field). Nebula approach: dependency
declared in `#[require("auth")]`, UI auto-generates picker. Two distinct
sources, two UI panels.

**Jan Oberhauser** *(n8n founder)* returned to share scars: the merged
approach gave n8n boilerplate, runtime crashes from credential type
mismatches, no discoverability. "If I rebuilt n8n, I'd go your way."

**Three-layer architecture ratified (F18):**
1. Author declares need (`#[require("key")] field: Handle<T>`) at
   compile time.
2. Workflow author binds instance (`slot_bindings: { key:
   "instance_id" }`) at config time.
3. Deployment registers instances (`resources.register("instance_id",
   ...)`) at startup.

Three audiences, three layers, single source of truth per layer. UI
editor renders Layer 2 as picker drawing options from Layer 3, valid
options filtered by Layer 1 type.

**Two-panel UI rendering ratified (F17):**
- "Action Input" panel — generated from `#[derive(Schema)]` on
  `Self::Input`.
- "Bindings" panel — generated from `#[require(...)]` declarations on
  Action struct.

Schema does NOT contain credential/resource picker fields; bindings
panel derived from action declarations. Two orthogonal concerns, never
conflated.

**Mode field + slot binding interaction documented:** Mode field
selects credential scheme (API key vs OAuth2); slot binding picker
filters to instances of selected scheme. Two-tier credential rendering
naturally emerges from existing primitives.

**TypedDAG forward compatibility (NS-9):** `#[require]` declarations
are forward-compatible with future TypedDAG generic bounds. Today
runtime UI picker; tomorrow compile-time generic bound; same author
code. Smooth upgrade story.

**Action items:** UI-1 / UI-2 / UI-3 (UI rendering convention, helpful
disabled state, two-tier credential interaction); NS-8 (`MetadataSlot`
in `ActionMetadata`); NS-9 (TypedDAG forward-compat docs).

**Forthcoming ADR:** **0064** (UI form composition — Cart + Jan
Oberhauser collaboration).

---

## Day 7 — Resource topology & multi-agent extensibility

**Frame:** User feedback on Day 6 evening mockups: tabs confusing,
resource picker awkward ("выбирать rate limiter странно").
User-preferred direction: Pattern B (explicit canvas nodes) for
extensibility + multi-agent. Alternative on table: hidden resources
configured separately.

**New voices:** Mark Payne *(Apache NiFi committer, 12 years experience)*,
Frances Perry *(Apache Beam, Google)*, Rich Hickey *(Clojure, "Simple
Made Easy")*, Alan Kay *(via video — message passing perspective)*.
Returning: Mitchell Hashimoto, Joao Moura, Harrison Chase, Maxim
Fateev, Stephan Ewen, Cart, dtolnay, matklad, Niko Matsakis, Carl
Lerche.

**Six rounds covering:** real-world precedents (NiFi, Beam, Terraform);
multi-agent implications (CrewAI, LangGraph); hidden-resources
alternative (Restate, Temporal); engine/runtime constraints; type
system / decoupling philosophy; vote.

**Voting (13 guests):**
- Hybrid (default hidden, opt-in canvas) — **5** *(matklad, dtolnay, Niko, Carl, Mitchell)*
- Hidden + Inspector — 4 *(Maxim, Stephan, Cart, Alan/Hickey aligned)*
- Pattern B always (canvas nodes default) — 3 *(Mark, Joao, Harrison)*
- Layered canvas — 1 *(Hickey strongest)*

**Outcome — Hybrid wins by plurality:**

1. **Author code unchanged** — `#[require("db")] pool: Handle<PostgresPool>`
   stable regardless of visual choice (dtolnay's pivot, F19).
2. **Default visual mode = Hidden + Inspector** — clean canvas + side
   panel with full bindings audit. Protects 80% of workflow authors
   from infrastructure cognitive load. Maxim's "workflow author shouldn't
   pick infrastructure" claim respected (F20).
3. **Opt-in visual mode = Promote-to-canvas (Pattern B)** — power
   users / integration architects render bindings as canvas nodes with
   dotted supply edges. Per-workflow persisted, collaborator sees same
   view. Mark's NiFi precedent.
4. **Multi-agent auto-promotion** — when 3+ agents/actions share one
   resource/credential, automatically promoted to canvas node.
   Tunable threshold. Joao's CrewAI insight.
5. **Layered canvas (Hickey)** — backlog, post-MVP. Two layers (logic
   / infrastructure) with toggle.

**Two new principles ratified:**
- **F19** — Author API decoupled from visual presentation
- **F20** — Default hidden, opt-in visible

**Action items:** UI-4 (Inspector panel), UI-5 (promote mechanism),
UI-6 (multi-agent auto-promotion heuristic), UI-7 (layered canvas
backlog).

**Forthcoming ADR:** **0065** — Visual rendering modes for slot
bindings (Mark Payne + Cart + Mitchell Hashimoto cross-collaboration).

**Notable quotes:**
- **Rich Hickey**: «Pluggable rendering is itself simplicity. You
  decoupled visual presentation from data model. That's the unbraiding
  I argued for.»
- **Alan Kay**: «Late binding во visual layer — правильно. Объекты
  sent messages, view rendered chooses how. This is how GUIs should
  always have worked.»
- **Mark Payne** *(losing vote, gracious)*: «I lost the vote, but
  acknowledged. NiFi's "controllers in side panel" решение тоже было
  unpopular initially, then accepted. Hidden default with inspector is
  acceptable compromise.»
- **dtolnay**: «Author writes one declaration. Editor chooses how to
  draw it. Engine doesn't care. This is correct decoupling.»

---

## Day 8 — Reckoning: charter consistency audit (2026-05-14, evening)

**Frame:** internal review surfaced three structural inconsistencies between
CONFERENCE-NOTES Days 1-7, VISION.md (charter), the ADR set 0052-0065, and
actual repo state of `crates/action/` and `crates/schema/`:

1. **ADR-0052 ↔ VISION.md §5 conflict.** Day 1 voted Concept A-modified (10/12).
   Charter §5 mandates `StaticMetadata` elimination, `Action::metadata()`
   removed, `Action` reduced to marker. ADR-0052 (Accepted 2026-05-13) kept
   instance-method `Action::metadata(&self)`, `ActionFactory`, `ErasedAction`,
   `GenericXxxFactory`. ADR-0052 is not marked Superseded; no follow-up ADR
   exists. Working tree `crates/action/src/lib.rs:108` still exports
   `StaticMetadata` and ships `pub mod erased/factory/from_workflow_node`
   modules — matching ADR-0052, not the charter.
2. **F-numbering drift.** Notes Day 5 morning ratifies F1-F5 (Foundation Five
   principles); Day 5 evening introduces F6 (Schema is the form) and F7
   (Dependency graph); Day 5 late introduces F8 (Symmetric API) and F9
   (Tokio-grade naming). Charter §3 starts the F-series at **F7**: F1-F6
   are absent, F9 (Tokio-grade naming) is missing entirely, and charter's
   F7-F9 correspond to notes' F6-F8. Either the charter is incomplete or
   the notes are wrong — both can't stand.
3. **Ratification debt.** ADRs 0053, 0054, 0055, 0056, 0057, 0058, 0059,
   0060, 0061, 0062, 0063, 0064, 0065 are all **Proposed (2026-05-14)** —
   none Accepted. Charter cites them as load-bearing forthcoming work, but
   no batch ratification ceremony happened. "Everyone agreed at the
   conference" is true; "every ADR file says Accepted" is not.

**Panel:** dtolnay, Niko Matsakis, Carl Lerche, Alice Ryhl, withoutboats,
Esteban Küber, matklad (action / DX). Henry Andrews, Ari Seyhun, Cart
(schema / UI). Bryan Cantrill, Mike Perham (process discipline). Maxim
Fateev (governance pattern, returning).

---

### Reckoning 1 — ADR-0052 vs charter Concept A-modified

**Carl Lerche** *(framing)*: "The Day 1 vote stands. The author-facing
surface dropping `metadata()` is correct — registry-owned metadata is what
the vote produced. But charter §5 doesn't actually say anything about
killing the internal `ActionFactory` / `ErasedAction` layer. Those are
engine-side dispatch, not author surface. ADR-0052 conflated the two."

**Niko Matsakis**: "Two separable changes were entangled in 0052. (A)
*author trait shape* — does `Action` carry `metadata()`? Day 1 says no.
(B) *engine dispatch layer* — does the runtime hold `Arc<dyn ActionFactory>`
behind erased variants? Day 1 didn't vote on this; 0052's reverse-dep
citation correctly notes engine + plugin + sandbox depend on it. Keep B,
drop A — that's the actual consensus."

**dtolnay**: "Once you split it that way, the residual disagreement is
small. `Action` is a marker (no `metadata`); `ActionFactory::metadata(&self)`
remains; `register(my_action)` infers metadata via `#[action]` /
`#[derive(Action)]` macro emission. The factory layer is engine-internal —
plugin authors never type the word."

**matklad**: "We need to be honest about the source-break. Today's surface
ships `Action::metadata(&self)` for callers that walk a registry and want
the metadata without going through the factory. There are exactly six
such call sites in `crates/engine/src/runtime/`. They can read
`factory.metadata()` instead. The break is mechanical."

**withoutboats**: "Symmetric with Day 5 late — we already accepted a hard
breaking change for `Acquirable`/`Handle<T>`. Author-side `Action`-as-marker
is the same shape of break. Pre-1.0 acceptable per
`feedback_hard_breaking_changes.md`. Don't soften it."

**Esteban Küber**: "I want `#[diagnostic::on_unimplemented]` on `Action`
explaining that authors don't implement it — they pick a sub-trait. The
marker trait without a diagnostic is a footgun."

**Maxim Fateev**: "Process question: how do we prevent this drift from
recurring? Day 1 conference produced a vote; ADR-0052 (drafted by a
different sub-thread same week) didn't honor it. The lesson is that ADRs
written *during* a charter cycle must cite the charter session that
authorized them."

**Bryan Cantrill** *(governance)*: "Two rules going forward: (1) charter
session vote = mandatory citation in ADR §Context; (2) any ADR landing
within 30 days of a charter session that contradicts a charter section
needs explicit Supersedes/Amends line — the audit-trail is non-negotiable."

**Vote (12/12 panel present):**

| Option | Tally |
|---|---|
| **A** — Accept Concept A-modified per charter §5; `Action` becomes pure marker; engine-internal `ActionFactory`/`ErasedAction` retained; write ADR-0066 superseding 0052's "Action shape" portion; mark 0052 as **Superseded in part by 0066** with diff-spec. | **11** *(everyone except Cart, who abstained as out-of-scope)* |
| B — Revert charter §5 to match 0052 (keep instance `metadata`) | 0 |
| C — Status quo, document the conflict, defer | 0 |

**Decisions:**

1. **ADR-0066 — "Concept A-modified ratified: Action as marker"** scheduled.
   Owner: core team. Target: Q3 2026 (same milestone window as 0055 facade
   work). Diff-spec:
   - Drop `Action::metadata` trait method (was instance per 0052; now gone).
   - Keep `DeclaresDependencies + Send + Sync + 'static` supertraits.
   - Keep `ActionFactory` / `ErasedAction` / `GenericXxxFactory` /
     `FromWorkflowNode` — these are engine-internal, not author surface.
   - `ActionFactory::metadata(&self)` is the registry-side access point.
   - Remove `pub use action::StaticMetadata` from `crates/action/src/lib.rs`;
     remove the trait itself if no internal caller remains.
   - `#[diagnostic::on_unimplemented]` on `Action` pointing authors to
     `StatelessAction` / `StatefulAction` / etc.
2. **ADR-0052 amended** with header note: `Status: Accepted 2026-05-13;
   Superseded in part by ADR-0066 (2026-05-14) — see §"Author trait
   shape"`. The "hybrid" engine-internal dispatch layer survives.
3. **Charter §5 unchanged.** "Removed by Concept A-modified" list is
   correct as-is; ADR-0066 ratifies it formally.
4. **Process commitment** *(per Bryan Cantrill)*: every future ADR must
   cite the originating CONFERENCE-NOTES session in `§Context`; any ADR
   that lands within 30 days of a charter session and conflicts with a
   charter section must explicitly carry `Supersedes:` / `Amends:` ADR
   metadata. Mechanical CI check optional, not blocking.

**Action items:** RC-1 (write ADR-0066), RC-2 (amend ADR-0052 header),
RC-3 (`lib.rs` cleanup PR removing `StaticMetadata` export — gated on
ADR-0066 Accepted), RC-4 (add `Supersedes`/`Amends` ADR template field).

---

### Reckoning 2 — F-numbering audit

**Frame:** charter §3 lists F7-F20. Notes Day 5 morning claims F1-F5 were
ratified "in VISION.md §3" — but they don't exist there. Notes' F6-F9 are
either renumbered to F7-F9 in charter (F6→F7, F7→F8, F8→F9) or lost
entirely (notes' F9 — Tokio-grade naming).

**matklad**: "Trace says the charter was edited from F1-onwards in a
draft, then truncated. The Foundation Five principles ratified Day 5
morning *exist* — they're in the conference record — but the editor pass
dropped them when extending to F20. This is editorial error, not
substantive disagreement."

**Carl Lerche**: "F9 from Day 5 late ('Tokio-grade naming discipline')
got *folded* into charter F9 ('Symmetric API surface') — the symmetric
API wording subsumes naming discipline. Acceptable, but it should be
called out so the historical record isn't lost."

**dtolnay** *(textual proposal)*: "Add F1-F6 to charter §3 verbatim from
Day 5 morning conference record. Keep current F7-F20 numbering — too
much downstream reference cost to renumber. Notes' F6/F7/F8 get a
footnote saying 'corresponds to charter F7/F8/F9 — numbering harmonized
on 2026-05-14'."

**Niko Matsakis**: "The Foundation Five (F1-F5) plus 'Plugin SDK is the
seam' (F6 unused in notes but implied) need to land in charter §3 as
written so authors reading §3 see the full principle hierarchy. Don't
relegate them to historical footnote."

**Henry Andrews** *(schema concern)*: "F11-F14 numbering is unaffected
and that's where my JSON Schema work lives. As long as the schema-side
of the F-series stays stable, I'm content."

**Vote (12/12):**

Unanimous — add F1-F6 to charter §3, keep current F7-F20 numbering,
footnote the historical drift.

**Decisions:**

1. **Charter §3 prepended with F1-F6** matching Day 5 morning record:
   - **F1** — Foundation crates are libraries, not frameworks
   - **F2** — Zero patent traps, forever (Apache-2.0/MIT dual license)
   - **F3** — Trait-only by default, engine-owned only where necessary
   - **F4** — Observable by construction (typed errors + spans +
     invariants ship together)
   - **F5** — Idiomatic Rust 2024+, never compromises
   - **F6** — Plugin SDK is the seam (single facade, internal crates
     are implementation detail)
2. **Charter §3 F7 onwards unchanged** — F7-F20 stay as authored.
3. **Notes Day 5/6/7 F-references** annotated with bracketed charter
   equivalent: e.g., "F6 in notes [= charter F7]" on first mention.
4. **F9-Tokio-grade-naming-discipline absorbed into charter F9** —
   noted as merged.

**Action items:** RC-5 (charter §3 patch adding F1-F6), RC-6 (notes
inline annotations for F6-F9 mappings).

---

### Reckoning 3 — Proposed → Accepted batch triage

**Frame:** 13 ADRs sit Proposed (2026-05-14). Charter cites them as
load-bearing. Either ratify what's ready, or keep them Proposed honestly
and stop citing them as decided.

**Mike Perham** *(governance)*: "Ratification ceremonies matter. 'Accepted'
on the ADR file is what downstream readers — and future contributors —
trust. 'Proposed' citation in a charter is hand-waving. Pick what's
real."

**Bryan Cantrill**: "Two filters: (1) is the design actually settled —
no open questions, just implementation? (2) is the implementation cost
in the next milestone window? If yes-yes — ratify. If no on either — stay
Proposed honestly."

**Per-ADR triage** *(consensus pass)*:

| ADR | Title | Design settled? | Implementation milestone | Verdict |
|---|---|---|---|---|
| **0053** | Two-struct DX consolidation | **No** — three options listed, no preferred | Q1 2027 | **Stay Proposed** |
| **0054** | Typed capability system | **No** — owner is Sam Scott design collaboration, design not started | Q4 2026 | **Stay Proposed** |
| **0055** | `nebula-sdk` facade | **Yes** — design clear from charter §4 | Q4 2026 | **Accept** |
| **0056** | Type-safe DAG validation | **No** — experimental track | Q1 2027 | **Stay Proposed** |
| **0057** | AI agent SDK | **No** — depends on 0056 + 0054 | Q2 2027 | **Stay Proposed** |
| **0058** | Schema field UI vocabulary | **Yes** — closed-set vocabulary specified in ADR body | Q3-Q4 2026 | **Accept** |
| **0059** | Cross-foundation dependency graph | **Yes** — tarjan SCC at registration, charter F8 | Q4 2026 | **Accept** |
| **0060** | Symmetric Foundation API | **Yes** — `Acquirable`/`Resolvable`, `Handle<T>` per charter F9 + breakfast follow-up | Q3-Q4 2026 | **Accept** |
| **0061** | `nebula-schema` core trait ratification | **Yes** — ratifies existing production code | Q3 2026 | **Accept** |
| **0062** | `nebula-schema::stdlib` newtype zoo | **Yes** — initial ship list locked in ADR | Q4 2026 | **Accept** |
| **0063** | JSON Schema 2020-12 lossless interop | **Partial** — extension namespace specified, import direction sketched only | Q1 2027 | **Stay Proposed** |
| **0064** | UI form composition | **Yes** — two-panel rendering, F17/F18 layered architecture | Q4 2026 with editor MVP | **Accept** |
| **0065** | Visual rendering modes | **Yes** — hybrid (hidden+inspector default, canvas opt-in) ratified Day 7 | Q1 2027 with editor MVP | **Accept** |

**Vote on the triage matrix (12/12):** unanimous accept.

**Decisions:**

1. **Eight ADRs ratified** (0055, 0058, 0059, 0060, 0061, 0062, 0064,
   0065): status → **Accepted (2026-05-14)**. Each gets:
   - `Status: Accepted (2026-05-14, ratified Day 8 reckoning)` header
     line.
   - Cross-reference to CONFERENCE-NOTES Day 8.
2. **Five ADRs stay Proposed** (0053, 0054, 0056, 0057, 0063) with
   honest "open question" lines added to each:
   - 0053 — pick option 1/2/3 before Q1 2027.
   - 0054 — Sam Scott engagement scheduled; capability trait family
     not yet sketched.
   - 0056 — experimental track; ADR will harden after `nebula-workflow-typed`
     spike.
   - 0057 — blocked on 0054 + 0056.
   - 0063 — import direction needs design (export side already shipped
     via `schemars` feature).
3. **New ADR-0066** added per Reckoning 1 — drafted, will be **Proposed**
   until core team review, target Accept by end of Q3 2026.
4. **Charter §11 forthcoming-ADRs table** updated:
   - 0066 added.
   - Eight Accepted ADRs migrated to "Existing ADRs preserved or amended"
     table.
   - Five Stay-Proposed ADRs keep their forthcoming entry with explicit
     "open questions" column.

**Action items:** RC-7 (header rewrite for eight Accepted ADRs — single
PR), RC-8 (open-questions block added to five Stay-Proposed), RC-9
(charter §11 table refactor), RC-10 (RC-1's ADR-0066 draft).

---

### Day 8 summary

**Three reckonings, three closed gaps:**

1. ADR-0052 ↔ charter §5 reconciled — ADR-0066 will ratify Concept
   A-modified author surface, ADR-0052 amended to note partial supersession,
   engine-internal dispatch layer survives intact.
2. F1-F6 added to charter §3; F-numbering drift corrected with footnotes
   instead of renumbering.
3. Eight ADRs ratified (0055, 0058-0062, 0064, 0065); five stay Proposed
   honestly (0053, 0054, 0056, 0057, 0063); new ADR-0066 in queue.

**Charter consistency restored.** Per Bryan Cantrill's governance
commitments, future ADRs cite their originating conference session and
explicitly carry `Supersedes:` / `Amends:` metadata when relevant.

**Notable quotes:**

- **Maxim Fateev** *(on process)*: «The Day 1 vote was real. Its
  partial loss inside ADR-0052 wasn't malice — it was a coordination
  gap. Coordination gaps compound. Cite the vote in the ADR.»
- **Bryan Cantrill**: «Workflow engines die from over-promise /
  under-deliver. Workflow engine charters die from undeclared
  inconsistency. Same disease, different surface.»
- **dtolnay**: «Marker `Action` is the right shape. The factory is the
  engine's problem. We were debating the wrong split.»
- **Carl Lerche**: «Author surface and engine internals are different
  concerns. ADR-0052 conflated them. ADR-0066 separates them. That's
  the actual delta.»

---

## Cross-references
</invoke>

- **Charter:** [`docs/VISION.md`](./VISION.md) — ratified architecture and
  product vision. Charter §3 to be patched with F1-F6 per Day 8 Reckoning 2.
- **Implementation plan:** [`.ai-factory/plans/nebula-action.md`](../.ai-factory/plans/nebula-action.md)
  — current iteration tracking Concept A-modified landing.
- **ADR-0052:** [`docs/adr/0052-action-surface-hybrid.md`](./adr/0052-action-surface-hybrid.md)
  — Accepted 2026-05-13; **Superseded in part by ADR-0066** (per Day 8
  Reckoning 1) — engine-internal hybrid dispatch layer preserved, author
  trait shape (`Action::metadata` instance method) removed in favor of
  Concept A-modified marker trait.
- **ADR-0066** *(forthcoming, scheduled Day 8 Reckoning 1)*: Concept
  A-modified ratified — `Action` as pure marker, `StaticMetadata` deleted,
  `ActionFactory::metadata(&self)` as registry-side access point.
- **Forthcoming ADRs (charter §11):** 0053 (two-struct DX), 0054 (typed
  capabilities), 0055 (`nebula-sdk` facade spec), 0056 (type-safe DAG),
  0057 (AI agent SDK), 0058 (`#[field(...)]` schema vocabulary), 0059
  (cross-foundation dependency graph), 0060 (Symmetric Foundation API:
  `Acquirable`/`Resolvable` traits, `Handle<T>`, `#[require(...)]`),
  0061 (`nebula-schema` core trait ratification — proof tokens,
  three-sibling boundary), 0062 (`nebula-schema::stdlib` newtype zoo),
  0063 (JSON Schema 2020-12 lossless interop), 0064 (UI form
  composition — schema vs slot bindings), **0065 (visual rendering
  modes — hidden + inspector default, canvas nodes opt-in,
  multi-agent auto-promotion)**.

---

## Maintenance

- These notes are **historical** — they capture the May 2026 conferences and
  do not get updated as work progresses. Use VISION.md and ADRs for current
  state.
- Next charter revision: every 6 months OR upon major roadmap milestone slip.
- If a future conference happens, append a new "Day N" section here with
  date.
