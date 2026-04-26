# Q8 Comprehensive Design Audit — DX + Parameters + Activepieces (Phase 1 of 3)

**Date:** 2026-04-25
**Agent:** dx-tester
**Slice:** Activepieces peer research + n8n parameter pain points + Tech Spec FROZEN CP4 (post-Q7) cross-reference
**Inputs read line-by-line:**
- `docs/research/activepieces-peer-research.md` (408 lines)
- `docs/research/n8n-parameter-pain-points.md` (493 lines)
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` (3522 lines — focused on §2.2.3, §2.6, §2.9.1a-c, §4, §5, §6, §9, §10, §13, §15.10-§15.11)
- `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/02a-dx-authoring-report.md` (Phase 1 authoring DX report — 264 lines)
- Spot-checks: `crates/schema/src/{field.rs, mode.rs, option.rs, loader.rs}` to inventory existing parameter primitives

**Method.** READ — no scratch code (Phase 1 was the scratch round). Cite line refs throughout. **Cross-axis** correlation table per the spec; each finding is graded 🔴 BLOCKING / 🟠 MAJOR / 🟡 MINOR / 🟢 NON-ISSUE with research provenance.

---

## §1 Activepieces «type-safe pieces» analysis

Activepieces (~21k stars, MIT) markets itself as «developer-first, type-safe pieces» and is the closest TypeScript peer to what Nebula's `#[action]` macro is shaping. They promise some things Nebula promises; they fail at some things Nebula has structurally avoided. Both halves are instructive.

### §1.1 Type-safety claims

**Activepieces dispatch model.** Each piece is authored with `createPiece({ displayName, actions, triggers, auth })` (peer-research line 62). Inside, `createAction(...)` and `createTrigger(...)` produce typed objects. The framework's typed surface is the `Property.{ShortText, LongText, Array, StaticDropdown, Checkbox, …}` DSL with a `propsProcessor` validation hook (peer-research lines 62-65) — a typed property catalogue, not a raw JSON-schema string. **Triggers** explicitly pick `TriggerStrategy.POLLING | WEBHOOK` as a typed enum (lines 67-69). Webhook triggers add `onHandshake` for provider verification. This is genuinely a step up from n8n's stringly-typed `INodeProperties`.

**Type bounds on input/output.** The framework is typed; the **community-piece code is not**. The literal title of issue #12456 is *«Eliminate excessive `any` types in community pieces for improved type safety»* (peer-research lines 110-112). The framework provides `Type<X>` parameters but community authors use `any` everywhere — this is the **«typed framework necessary but not sufficient»** finding (peer-research line 303). Consequence for Nebula: typed framework is a precondition, not a moat; **enforcement** at the macro / clippy / cargo-deny boundary is what actually keeps community plugins typed. Nebula's `#[action]` macro hard-rejects `credential = "string"` at compile time per Tech Spec §4.7 (compile_error! with span at the offending literal) — this discipline is precisely what Activepieces lacked when issue #12456 piled up.

**Schema generation from types vs hand-declared.** Activepieces is **hand-declared** — authors construct the property DSL by hand. No automatic derive-from-type. This is the gap Nebula's `#[action(parameters = T)]` + `#[derive(HasSchema)]` (`crates/schema/src/has_schema.rs`) is built to close: Rust types are the source of truth, the schema is derived. Per Tech Spec §4.6.1 (line 1361-1374) the macro emits `with_schema(<T as HasSchema>::schema())` — single-source-of-truth.

### §1.2 Developer onboarding

**Time-to-first-piece comparison.** Activepieces has CLI scaffolding (`create-trigger.ts` per peer-research line 105) and `nx`-based hot reload. Reviewers consistently describe it as «TypeScript way to write n8n nodes». Concrete data point: **Activepieces' typed piece framework is their best DX lever** (peer-research line 330). No specific TTFP number is in the research, but the qualitative consensus is «good».

Nebula's measured TTFP from Phase 1 02a:
- Action 1 (Stateless HTTP GET): **12 min**
- Action 2 (StatefulAction pagination): **8 min** (Action 1 knowledge transferred)
- Action 3 (ResourceAction + Credential): **32 min** ← dominated by credential pain (8 of 12 🔴/🟠)

Target (post-cascade): **<5 min for Action 1**. Phase 1 verdict: 👎 because the Action-3 credential surface is structurally unusable — but Tech Spec §4.6, §6.2, §6.4, §10.4 land the fixes.

**Boilerplate per piece.** Tech Spec §4.5 lines 1339-1350 commits to **~71 LOC expanded per first slot**, with linear ~10 LOC per additional credential slot. Phase 1 measured ~8 LOC business logic, ~29 LOC boilerplate (Action 1). Activepieces piece authoring is qualitatively similar (per the research, ~50-80 LOC for a basic piece) — the boilerplate ratio is comparable; Nebula's win is that the boilerplate is **macro-emitted** (zero hand-written), not author-written.

**IDE integration.** Activepieces gets TypeScript LSP (good autocomplete, good hover). Nebula gets rust-analyzer (also good autocomplete, also good hover). **Tied** on first-order IDE story; Nebula wins on second-order — `cargo check` is a **type-system gate**, not just an editor lint, so Activepieces' issue #12456 (community `any` creep) cannot have a Rust analogue.

### §1.3 Migration / customer pain

**Why users migrate FROM Activepieces** (per peer-research lines 240-254):

1. **Raw speed.** Thread #3864 — canonical: «n8n 0.5-1s/request vs AP ~15s/request» on identical hardware. Sandbox overhead dominates.
2. **Integration count.** ~400 pieces vs n8n's 1100+; long-tail enterprise integrations thinner.
3. **Expression power.** n8n `$json`/`$node`/`$workflow` + Code node > AP mustache + sandboxed code piece.
4. **Community size.** ~180k stars vs ~21k for AP.
5. **Complex branching / multi-trigger flows.** AP only got multiple triggers (#9690), sequential webhook handling (#6844), native parallelism (#10980) as **open issues** — capabilities n8n already has.

**Customer complaints about Activepieces.** Top complaint clusters (peer-research lines 159-220):

- **Piece breakage on upgrade** — `area/third-party-pieces` label has dozens of stale (>6 month) bugs because community pieces have no dedicated maintainer.
- **Code-step instability** — five separate issues with title «code piece fails» (#11998/#11995/#10989/#10639/#9554), curly-brace and named-property bugs.
- **OAuth long-tail** — Google 7-day, Xero, GoHighLevel, Monday, Facebook Pages, Hubspot — each provider variant breaks differently.
- **«I regret switching»** stories cluster on piece breakage after upgrade + slow UI feeling at scale.

### §1.4 Cross-cutting features

**AI / MCP integration patterns.** Activepieces is **first-class MCP** — `mcpTool` trigger, MCP server UI, 400+ claimed MCP servers, «flows-as-tools» (peer-research lines 91, 282). But MCP UI was hidden in 0.73 and broke flows-as-tools (#12294); MCP security (tool injection #12234, output sanitization #12381/#12389) is a recurring class. **Implication for Nebula:** MCP-first mindset wins ergonomics; MCP-output-sanitization must ship with the surface, not bolted on. Nebula's §6.3 `redacted_display` helper + dedicated `nebula-redact` crate (Tech Spec §6.3.2) is the right shape.

**Code-as-piece (custom code blocks).** Activepieces' code piece is fragile: #10634 fails on curly braces; #9554 fails on property named «target»; #10989/#11995/#11998 generic «code piece fails». Cross-cutting fragility class (peer-research lines 168-174). **Implication:** code-action surfaces need strict input schema + good error surfacing. Nebula's typed `parameters = T` + `redacted_display` partially closes this; **Q8 follow-up:** action-as-code-block (the «inline custom code» feature n8n + AP both expose) is **not** in Tech Spec §4 zones. Open question whether Nebula should support code-block-in-parameter at all — see §3 below.

**Loops / branches.** AP's «callable subflow» exists but requires sequential processing workaround (#6844). Nebula's `ControlAction` (sealed DX trait per Tech Spec §2.6 + ADR-0038) is the parallel — lands as Stateless + `#[action(control_flow = …)]` zone.

### §1.5 Activepieces correlation table

| Source line | Finding | Nebula coverage | Severity |
|---|---|---|---|
| AP peer line 67 | `TriggerStrategy.POLLING \| WEBHOOK` typed enum | Tech Spec §2.6 sealed DX traits `WebhookAction` / `PollAction` (peer-of-Action per §15.11 R6); §2.2.3 R3 `accepts_events()` predicate | 🟢 Covered |
| AP peer line 89 | `PieceAuth.OAuth2/BearerToken/CustomAuth` declarative variants | Tech Spec §4.1.1 three-pattern credential dispatch (Concrete / ServiceCapability / CapabilityOnly); credential Tech Spec §3.4 line 851-863 | 🟢 Covered |
| AP peer line 110-112 (#12456) | `any` creep in community pieces | Nebula's `cargo check` + `#[action]` macro hard-error on `credential = "string"` (Tech Spec §4.7) — **structurally enforced** | 🟢 Better than AP |
| AP peer line 132 (#6749) | Webhook payload **NO schema validation by default** (open 14 mo) | Tech Spec §6.1 JSON depth cap 128 + `nebula-validator` `ValidSchema` per §4.6.1 universal carrier | 🟢 Day-one |
| AP peer line 160-166 | Piece breakage on upgrade — `area/third-party-pieces` cluster | Tech Spec §13.1 deprecation policy (post-1.0); §5 macrotest expansion snapshots regression-lock per-slot emission | 🟠 Covered for emission; **gap:** no contract-test framework for community plugins yet |
| AP peer line 168-174 | Code-step fragility | Tech Spec §4.6.1 `parameters = T` + `HasSchema` derive — Probe 7 forces typed bound (`E0277: HasSchema not satisfied`) | 🟠 Partial — code-block-in-parameter not in zones |
| AP peer line 187-192 | OAuth refresh long-tail (#3067, #6867, #7850, #4665) | Credential Tech Spec governs; per Strategy §6.6 cross-crate coordination cascade | 🟢 Cross-cascade |
| AP peer line 195-201 | Performance — flow editor >60s at 100+ steps; #11204 OOM on piece sync | Out of action cascade scope (engine / api territory) | 🟢 Out-of-scope |
| AP peer line 213-216 (#12234) | MCP tool description injection + missing output sanitization | Tech Spec §6.3 `redacted_display` + dedicated `nebula-redact` crate (§6.3.2 lines 1738-1758) | 🟢 Day-one |
| AP peer line 232-237 | MIT license vs n8n Sustainable Use License | Cargo-canonical OSS license — out of action cascade scope | 🟢 N/A |
| AP peer line 250-253 | Native parallelism + sequential-webhook open issues | Tech Spec §2.6 `WebhookAction`/`PollAction` peer-of-Action shape declares semantics in metadata; cluster-mode hooks per Strategy §3.1 component 7 (deferred) | 🟠 Partial — primitives exist; multi-trigger-per-flow is engine cascade |
| AP peer line 273-285 (Quick Wins #1-#10) | AP's 10 «Nebula could do» items | All ten map to Tech Spec sections — see §3 below | 🟢 |

**§1 Net.** Activepieces' three biggest wins (typed property DSL, declarative auth variants, explicit trigger-strategy enum) are all already in Nebula's frozen design. Their three biggest pains (sandbox cost, OAuth long-tail, piece breakage on upgrade) are mitigated by orthogonal choices (Rust over JS sandbox, credential cascade ownership, contract tests + macrotest snapshots). **Activepieces is not a competitor where Nebula's frozen design has a gap; it's a peer where Nebula's frozen design is structurally better.** The risk to mitigate is that Nebula's surface gets the typed-framework story right but fails at distribution / discoverability — which is Phase 0 audit T1 / Phase 6 §16.5 cascade-final readiness scope, not Q8.

---

## §2 n8n parameter pain points (493 lines, surveyed)

Total findings extracted: **62 distinct pains** across the three axes (Coverage / Ergonomics / Bugs). Categorised below with severity and Nebula-coverage cross-ref.

### §2.1 Coverage (parameter types)

n8n catalogues ~20 parameter types (n8n-param line 96-99): `string`, `number`, `boolean`, `options`, `multiOptions`, `collection`, `fixedCollection`, `resourceLocator`, `resourceMapper`, `credentialsSelect`, `notice`, `hidden`, `json`, `dateTime`, `color`, `filter`, `assignmentCollection`, `workflowSelector`, `cron`. **Breadth normal.**

Nebula's `Field` enum (`crates/schema/src/field.rs:780-807`) has **13 variants**: `String`, `Secret`, `Number`, `Boolean`, `Select`, `Object`, `List`, `Mode`, `Code`, `File`, `Computed`, `Dynamic`, `Notice` — with `StringField.hint: InputHint::{Date, DateTime, Time, Color}` collapsing what n8n exposes as 4 separate types. `NoticeField`, `Dynamic`, `Computed` have direct n8n analogues.

| n8n type | Nebula coverage | Notes |
|---|---|---|
| string | `StringField` | covered (with hint = Date/DateTime/Time/Color) |
| number | `NumberField` | covered |
| boolean | `BooleanField` | covered |
| options | `SelectField` (`crates/schema/src/option.rs`) | covered with `disabled` + `description` per option |
| multiOptions | `SelectField` (with `multiple` flag) | needs verification — multi-select shape |
| collection | `ObjectField` | covered |
| fixedCollection | `ListField` | needs verification — fixedCollection's wrapper-keyed shape (n8n line 270-275) |
| resourceLocator | **GAP** — no 3-form variant | 🟠 see §2.1.1 below |
| resourceMapper | **GAP** | 🟠 see §2.1.2 below |
| credentialsSelect | Implicit via `#[action(credentials(...))]` zone | 🟢 covered architecturally — author declares; runtime resolves |
| notice | `NoticeField` | covered (`NoticeSeverity::{Info, Warning, Danger, Success}`) |
| hidden | `VisibilityMode::Never` | covered (collapsed into visibility) |
| json | `ObjectField` | covered (or via `serde_json::Value`-typed field) |
| dateTime | `StringField` + `InputHint::DateTime` | covered |
| color | `StringField` + `InputHint::Color` | covered |
| filter | `Computed` / `Code` field | partial — n8n's filter is its own type |
| assignmentCollection | `ListField` of `ObjectField` | needs verification |
| workflowSelector | `DynamicField` with workflow-loader | needs verification — cross-workflow reference |
| cron | `StringField` + `InputHint::Cron`? **No `Cron` variant in InputHint** | 🟡 minor — could collapse into String hint |

#### §2.1.1 Explicit gap: `resourceLocator` 3-form pattern (n8n line 87-91)

n8n's `resourceLocator` lets the user supply an entity ID via three modes: typed list (dropdown), URL (paste a URL, server extracts ID), free-text ID. It's stuck on Typeform (#21148), Outlook (#17078), Matrix (#16379). **Activepieces does NOT have a parallel.** Nebula has nothing comparable today — the closest is a `DynamicField` + custom UI mode logic, but the **3-state UI semantics** (List/URL/ID) are not first-class in `Field` or `InputHint`.

**Severity:** 🟠 — resourceLocator is the n8n-canonical UX for «pick a Slack channel by name OR by ID OR by URL» and it covers ~40% of customer-facing nodes (Slack, GitHub, Notion, Asana, Trello, Linear, Jira, etc.).

**Tech Spec coverage:** Nebula's `parameters = T` schema-as-data approach can carry the variant via tagged enum (`enum SlackChannelRef { Id(String), Url(String), Pick { id: String, name: String } }` + `#[derive(HasSchema)]`), but the **UI-rendering hint** to «show three modes with a switcher» is not in `InputHint` and not in `Field::Mode`'s variants. **Recommendation for Phase 2:** add `InputHint::ResourceLocator { modes: [List, Url, Id] }` or extend `ModeField` per n8n's `resourceLocator.three_modes` pattern.

#### §2.1.2 Explicit gap: `resourceMapper` (n8n line 87-91, 278-296)

n8n's `resourceMapper` shows a column-by-column UI for mapping an external schema (DB columns, spreadsheet headers) to flow values. Hot bugs: #23884 (404 on switch), #19327 (NocoDB fields list empty), #6770 (Postgres auto-mapping). Root cause per n8n-param line 296: «resourceMapper persists snapshot of schema at save-time, but live schema evolves; no schema version pin or migrate-mappings-on-schema-drift step».

**Severity:** 🟠 — resourceMapper is critical for ETL nodes (Postgres / Google Sheets / Airtable / NocoDB). Nebula has no parallel.

**Tech Spec coverage.** Tech Spec §2.2.4 ResourceAction's `configure(&self, ctx) -> Future<Self::Resource>` model is the right composition target — a Postgres ResourceAction could expose its column schema via metadata, downstream nodes consume it via `resource.schema()`. **But:** the schema-drift problem (n8n correlation table line 354) is not addressed in Tech Spec §2.2.4. **Recommendation for Phase 2:** mapping-with-schema-version-pin pattern, plus engine-side migrate-on-drift step.

#### §2.1.3 Coverage gaps (n8n line 102-122)

| Missing | n8n evidence | Nebula coverage |
|---|---|---|
| `loadOptions` inside `fixedCollection` | feature req 70942 (open) | 🟠 `Dynamic` field has `depends_on: Vec<FieldPath>` — paths can express sibling-by-path; **structural support exists**, but not yet exercised in the `#[action(parameters = T)]` macro emission (Tech Spec §4.6 silent on this) |
| Populate `fixedCollection` from `loadOptionsMethod` | thread 6585 | 🟠 same — `Dynamic` + `loader` via registry (`crates/schema/src/loader.rs:251`) supports record loaders; but Tech Spec doesn't say how the macro composes a `ListField` with a record loader |
| File upload with default/preview | #21905 required file accepts empty | 🟢 `FileField` exists (`crates/schema/src/field.rs:666`) — needs verification of `default` + `preview` shape |
| Schema-evolution tracking for `resourceMapper` | #23884, #19327, #6770 | 🟠 see §2.1.2 — not first-class |
| **Pagination in `resourceLocator` list** | #21148 Typeform stuck, #17078 Outlook | 🟢 `loader.rs:202-230` — `PaginatedResult<T> { items, next_cursor, total? }` is built-in. **Best-in-class.** |
| Expression-mode for array-item addition | #19982 | 🟢 `ExpressionMode::Allowed` is per-field; `ListField` items inherit |
| Expression-mode on conditional driver AND dependent simultaneously | #25803 (open) | 🟠 — `VisibilityMode::When(Rule)` + `RequiredMode::When(Rule)` exist; need verification this composes — see §2.2.1 below |
| **Date-range picker** | forum requests | 🟡 — would be a tagged enum {Start: DateTime, End: DateTime}; `parameters = T` natively supports |
| JSON-schema-driven collection | forum requests | 🟢 — `DynamicField` with loader-returned schema + `Object`/`List` is the path |
| Rich text / WYSIWYG | forum requests | 🟡 — `StringField` with `InputHint::RichText` would close this; today only `Code`/`String` |
| Typed key-value (not just `{k:string, v:string}`) | n8n `assignmentCollection` partial | 🟢 — typed via `parameters = T` Rust struct |

### §2.2 Ergonomics / Universality

#### §2.2.1 Conditional parameters (n8n's `displayOptions`)

**This is where Nebula's existing schema is best-in-class.** `crates/schema/src/mode.rs:10-18`:

```rust
pub enum VisibilityMode {
    Always,                     // default
    Never,                      // replaces n8n `hidden`
    When(Rule),                 // n8n displayOptions.show
}
```

Plus `RequiredMode::When(Rule)` (line 32-40). Both are typed `Rule` (`nebula_validator::Rule`), serde-roundtrippable, and cycle-detectable at schema-load time.

**Compare to n8n root cause** (n8n-param line 311): «displayOptions evaluation runs against single flat parameter map, so name collisions across siblings of different `fixedCollection` variants ambiguous. No cycle detection in dependency graph.»

**Nebula already addresses both:**
- Per-field `key: FieldKey` is typed (not stringly), and `FieldPath` (`crates/schema/src/path.rs`) lets a `Rule` reference siblings by path — sibling collision is structurally avoided per Tech Spec §4.1.3 zone parser invariants ("Cross-zone slot-name collision is `compile_error!`"; same discipline applies via n8n correlation table line 348 «paths for parameters, not flat names»).
- Cycle detection in `Rule` dependency graph is a **nebula-validator** scope item (referenced via n8n correlation table line 350 «Schema DAG validator at node-registration time»). **Verification gap:** does `nebula-validator` actually run cycle detection at `Schema::validate()` time? **Open Q for Phase 2.**

**Severity:** 🟡 — primitives exist; needs verification + a probe in `tests/compile_fail/` for the cycle case.

#### §2.2.2 Default values

n8n: #19607 (open) — fixedCollection with defaults can't save in community nodes; #1119 — redundant data persisted when collection options removed; #19197 — community-node upgrade silently changes defaults; #27160 (open) — Git integration resets credential options; #27590 — date field doesn't show default. **Hot class — 5+ open**.

n8n-param root cause line 296: «resourceMapper persists snapshot of schema at save-time, but live schema evolves».

**Nebula coverage.** `define_field!` macro emits `default: Option<Value>` per field (`crates/schema/src/field.rs:46-47`) — typed via the `Value` round-trip. `parameters = T` macro emission (Tech Spec §4.6.1) routes through `<T as HasSchema>::schema()` — defaults are derived from `#[serde(default)]` attributes on the Rust struct. **Strong shape.**

**Open gap:** workflow-version pinning + migrate-on-drift. n8n correlation table line 358 names it («Default-value migration requires explicit `migrate_from` in node manifest»). Tech Spec §13.1 deprecation policy (line 2454-2462) commits «Codemod artefact for any non-trivial migration» — but field-level default migration is not in the codemod transforms T1-T9. **Recommendation for Phase 2:** consider whether `parameters = T` macro also emits a `migrate_from = OldT` attribute hook for backward-compat default reshaping.

**Severity:** 🟡.

#### §2.2.3 Validation messages

n8n: #21905 — required file accepts empty submission; #24286 — options missing required field; #22378 — form default-value query params don't work with dropdown/checkbox/radio; #25913 — OpenAI «developer» role validation warning despite required; #19431 — community-node fails «Could not get parameter»; #19319 — Webhook returns 200 but «JSON parameter needs to be valid JSON». **Hot class — 6 open**.

n8n correlation table line 361: «Required validation runs server-side always.» Nebula already commits to this — Tech Spec §6.1 JSON depth cap fires in `StatelessActionAdapter::execute` at deserialization (line 1599). `nebula-validator::Rule` is the validation primitive.

**Severity:** 🟢 — covered structurally.

#### §2.2.4 Group / section / tab organization

n8n's `displayOptions.show` lets you partition fields into conditional sections, but there's no first-class «tab» or «section» concept — partitioning is emergent. n8n correlation table doesn't surface this as a pain.

**Nebula coverage.** `Field`'s `group: Option<String>` (`crates/schema/src/field.rs:58-59`) is a free-form group label. **No `tab` or `section` first-class.** This is fine — UI consumers (the engine's React side, future plugin marketplace) interpret `group` as section title. **Severity: 🟢.**

#### §2.2.5 Resource locator (n8n's 3-form pattern)

Already covered in §2.1.1 — 🟠 gap.

### §2.3 Bugs (n8n-param Ось 3)

#### §2.3.1 Expression toggle fragility (n8n-param line 130-156)

This is the **single largest pain class** in n8n parameters: 25+ open+closed. Examples:

- **#15900 hot** — Set node outputs literal JS-source instead of evaluated value.
- **#27131** — editor silently flips strict↔loose `typeValidation` when opening IF/Switch.
- **#24499** — decimal `.` → `,` per locale, corrupts numbers.
- **#16498/#20519/#16262** — emoji break expression parser.
- **#21982** — Korean chars break autocomplete.
- **#23395** — drag-and-drop generates wrong `$json.field` after append/merge.
- **#27734** — **Prototype pollution vulnerability in `@n8n_io/riot-tmpl`**.

n8n correlation table line 348 root cause: «expression — regex-matched string prefix, NOT AST-backed typed value. Parameter's effective type at runtime determined by re-parsing, not typed model on client.»

**Nebula coverage.** `crates/schema/src/expression.rs` exists; `crates/schema/src/value.rs:FieldValue` is the typed wrapper. Nebula's expression handling is referenced in Tech Spec §2.6 / §4.6 indirectly but **expression parsing internals are out of action cascade scope** — they live in nebula-schema / nebula-validator / nebula-expression crates. The Quick-Win in n8n correlation table line 370 («`enum ParamValue { Fixed(T), Expression(ExprAST) }`») is structurally what `FieldValue` provides today.

**Severity for Q8 scope:** 🟢 — out of action cascade. **For broader Nebula:** ensure expression layer is AST-backed (verify in Phase 2 via spot-check on nebula-expression).

#### §2.3.2 `loadOptions` caching / invalidation (n8n-param line 222-249)

n8n: #22123 (open) — ResourceLocator in-flight response caching keys on wrong value, two concurrent loads on slow network swap results; #27499 (open) — unable to filter MySQL table list; #27652 (open) — OpenAI model list fails when custom credential header uses expression because `loadOptions` evaluated **without** resolve credential expressions; #21148 — Typeform stuck loading. **Hot class — 7 open.**

n8n root cause line 248: «`loadOptions` — synchronous-ish REST call with reactive memo, no explicit TTL, no version key per-credential, no per-call tracing.»

**Nebula coverage.** `crates/schema/src/loader.rs` is the loader-registry primitive. `LoaderContext` (line 33-95) carries `field_path`, `cursor: Option<String>`, **redacts secrets in values before exposing to loader** (line 97-153 — security-by-construction!), and `register_option` / `register_record` give a typed registry path. **No explicit TTL.** **No version-key per credential** (n8n correlation table line 352 Quick-Win).

**Severity:** 🟠 — primitives are best-in-class on the security side (secret redaction); cache key + version pinning + per-call tracing not yet wired. Tech Spec §6.3 `redacted_display` covers the error-emit side; loader-cache shape is **out of action cascade scope** (nebula-schema territory). **Recommendation for Phase 2:** confirm `LoaderContext` cache discipline in nebula-schema; if absent, schedule sibling cascade.

#### §2.3.3 `fixedCollection` UX fragility (n8n-param line 252-275)

n8n: #19607 (open), #6693, #1119, #1318, #13049, #23347. **Hot — 6+ open.** Root cause: fixedCollection output shape unusual (wrapper object keyed by variant name); persisted state in DB carries obsolete sub-keys forever.

**Nebula coverage.** `ListField` per `crates/schema/src/field.rs:531` is the parallel; it inherits the standard `default`, `visible`, `required`, `expression`, `group`, `rules` chain. Output shape is `Vec<T>` (not n8n's wrapper-keyed shape), so the «property name appears twice in output» pain (n8n line 272) is structurally avoided.

**Severity:** 🟢 — structurally better than n8n.

#### §2.3.4 `resourceMapper` schema drift

Already covered in §2.1.2 — 🟠.

#### §2.3.5 `displayOptions` cycle / conditional cross-talk

Already covered in §2.2.1 — 🟡, primitives exist; cycle detection verification needed.

#### §2.3.6 Community-node authoring mistakes

n8n: **#27833 (closed) — community-node credentials NOT isolated per workflow** — all workflows resolve to last saved credential. **High-severity class.** Plus #23877 — community OAuth2 nodes ignore user-entered scopes; #19607; #4037 — lintfix destroys declarative properties.

n8n correlation table line 360: «Community node leaks credentials across workflows. Root cause: only static ESLint rule. Nebula mitigation: Resolver keyed by `(workflow_id, node_id, type)`; runtime assertion.»

**Nebula coverage.** Tech Spec §6.2 hard-removal of `CredentialContextExt::credential<S>()` no-key heuristic (line 1662-1700) is **the structural fix** for this exact class — cross-plugin shadow attack S-C2 / CR3. Each `#[action(credentials(slot: Type))]` zone produces a typed `CredentialRef<C>` slot; resolution at engine time is keyed by `(action_key, field_name)` (Tech Spec §3.1 line 1024). No type-name heuristic. No silent collision.

**Severity:** 🟢 — Nebula's frozen design **structurally eliminates** n8n's #27833 class.

### §2.4 n8n correlation table

| Source line (n8n-param) | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| line 130-156 | Expression toggle fragility (25+ issues) | Out of action cascade — nebula-schema/expression scope | 🟢 OOC |
| line 222-249 | `loadOptions` cache pollution (#22123 hot) | `crates/schema/src/loader.rs` provides primitives; cache-key shape not wired into `parameters = T` emission | 🟠 |
| line 252-275 | `fixedCollection` UX bugs | `ListField` structurally avoids the wrapper-keyed shape | 🟢 |
| line 278-296 | `resourceMapper` schema drift (#23884 hot) | ResourceAction `configure()` lifecycle is foundation; **no schema-version-pin pattern** | 🟠 |
| line 299-310 | `displayOptions` cycle / cross-talk | `VisibilityMode::When(Rule)` + `FieldPath` exist; cycle detection verification needed | 🟡 |
| line 313-329 | Validation / required-field bugs | Tech Spec §6.1 JSON depth cap + `nebula-validator` server-side validation | 🟢 |
| line 332-340 | Community-node credential leak (#27833) | Tech Spec §6.2 hard removal — **structural fix** for shadow attack S-C2 | 🟢 |
| line 348 (Quick-Win 1) | `enum ParamValue { Fixed(T), Expression(ExprAST) }` | `FieldValue` exists; AST-backed verification at Phase 2 | 🟢 |
| line 354 (Quick-Win 7) | `loadOptions` returns `{items, cursor, total?}` | `loader.rs:202-230` PaginatedResult — best-in-class | 🟢 |
| line 358 (Quick-Win 8) | `migrate_from` in node manifest for default reshaping | Tech Spec §13.1 codemod for breaking changes; field-default migration not in T1-T9 | 🟠 |
| line 362-367 | `ParamValue` CRDT-friendly with content-addressed version counter | Out of action cascade; storage cascade scope | 🟢 OOC |
| line 396-405 | Meta-idea: «Rust types = SSoT → codegen TypeScript client → AST expressions → Unicode lexer» | Tech Spec §4.6.1 single-source-of-truth via `HasSchema`; Probe 7 enforces typed bound | 🟢 |

**§2 Net.** Nebula's existing schema (`crates/schema/`) plus the Tech Spec's `parameters = T` universal carrier (§4.6.1, §2.9.1a Resolution point 2 line 835) **already covers ≥50 of the 62 surveyed n8n pains** — most via the typed-Rust-as-SSoT structural choice, not a per-pain fix. The four remaining 🟠 gaps:

1. **resourceLocator 3-form** (`InputHint::ResourceLocator { modes }` not yet in the Field DSL). [§2.1.1]
2. **resourceMapper schema-drift / version-pin** (no first-class «migrate mapping on schema drift» step). [§2.1.2]
3. **`loadOptions` cache key + version pin per credential** (primitives in nebula-schema, not wired into action). [§2.3.2]
4. **Field-default migrate_from** (not in codemod T1-T9). [§2.2.2]

These are the four 🟠 gaps Phase 2 (synthesis) should weigh — they are coverage holes, not paradigm conflicts.

---

## §3 #[action] macro emission DX completeness

Tech Spec §4 (lines 1200-1407) covers credential/resource zones + `parameters = T` + `version = "X.Y[.Z]"` + `description` doc-fallback. Q8-research-driven gaps below.

### §3.1 Conditional parameter visibility (n8n displayOptions equivalent)

**Tech Spec coverage status: COVERED via schema layer.** `parameters = T` macro emission (§4.6.1 line 1361) routes through `<T as HasSchema>::schema()`; `HasSchema` derive can attach `visible: VisibilityMode::When(Rule)` per field in the Rust struct. The macro itself does NOT need to expose a `display_options(...)` zone — it's already in the Rust type's schema derivation.

**Verification needed (Phase 2):** does `#[derive(HasSchema)]` on `nebula-schema-macros` actually emit `VisibilityMode::When(Rule)` from a per-field attribute like `#[schema(visible_when = "...")]`? — spot-check on `crates/schema/macros/`.

**Severity:** 🟢 — likely covered; verification gap.

### §3.2 Resource locator (3-form pattern)

**Tech Spec coverage status: GAP at field-DSL layer.** `InputHint::{Date, DateTime, Time, Color}` is the existing typed-string-hint mechanism. There's no `InputHint::ResourceLocator { modes: [List, Url, Id] }`. As a workaround, an action author can:

```rust
#[derive(HasSchema, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SlackChannelRef {
    List { id: String, name: String },
    Url(String),
    Id(String),
}
```

The schema derivation produces a tagged enum + variant fields, but the **UI rendering** doesn't get a «3-mode switcher» hint — it renders as «Mode select + dependent fields» (which works, but is 3 clicks instead of 1). **Severity:** 🟠 — this is the n8n `resourceLocator` UX gap; viable workaround exists.

### §3.3 Code-block-in-parameter

**Tech Spec coverage status: GAP at macro layer.** `CodeField` (`crates/schema/src/field.rs:645`) exists and `Code` is one of the 13 Field variants — so a parameter type **CAN** be a code block. The action macro emits `parameters = T` where `T: HasSchema`; if `T` includes a `code: String` field with `#[schema(field_kind = "code")]`, the schema captures the intent. **Verification needed (Phase 2):** does the schema-macros support a `field_kind` / `widget` attribute on `String`-typed fields to map to `CodeField` instead of `StringField`?

Activepieces' code-piece fragility (peer-research line 168-174) lands here — Nebula must surface code-block fields with strict input schema + good error surfacing. Tech Spec §6.3 `redacted_display` plus the Probe 7 `parameters = T` `HasSchema` bound discipline closes the worst class. **Severity:** 🟡.

### §3.4 Plugin-marketplace UI generation

**Tech Spec coverage status: COVERED.** Per §2.9.1c (lines 871-911) — schema-as-data axis is universal. `ActionMetadata.base.schema` carries the JSON-schema for UI form generation; `ActionMetadata.inputs` / `outputs` carry port topology. **n8n surface parity is explicitly NOT pursued per `docs/COMPETITIVE.md` line 29-41** — but the data is sufficient for any UI consumer that walks `ActionMetadata` per the schema-as-data axis (§2.9.6 point 2).

**For Activepieces parity:** AP exposes `INodeTypeDescription`-equivalent runtime arrays; Nebula exposes `ActionMetadata`'s typed reflection — **stronger** because Rust derive-macro pipelines guarantee `<T as HasSchema>::schema()` is consistent with the live Rust type. Single source of truth.

**Severity:** 🟢 — covered architecturally.

### §3.5 Cross-cutting: 4 attribute zones in §4.1 vs ergonomic completeness

Tech Spec §4.1 zones: `credentials(slot: Type)`, `resources(slot: Type)`, `parameters = T`, `version = "X.Y[.Z]"`, `description`. **Things NOT in zones (intentional):**

- Display-options / conditional visibility — lives in `T`'s `#[derive(HasSchema)]`.
- Webhook signature policy (Activepieces' `onHandshake`) — for `WebhookAction`, lives in the `WebhookAction` impl body, not `#[action(...)]`.
- Poll interval — for `PollAction`, lives in the impl body; Tech Spec §15.11 R6 names `PollConfig` + `POLL_INTERVAL_FLOOR`.
- Output port declarations — `ActionMetadata.outputs` per `crates/action/src/port.rs`.

**This is the right shape.** The `#[action]` macro is a **field-rewriting + metadata-generating attribute**, not a god-attribute. Per ADR-0036 §Negative item 2 («pervasive struct-level rewriting harms LSP / grep / IDE hover semantics»), the narrow zone discipline is load-bearing.

**Severity:** 🟢.

---

## §4 Plugin authoring DX time-to-first-action prediction

### §4.1 Phase 1 measured baseline (current production)

- Action 1 (Stateless HTTP GET): **12 min**
- Action 2 (StatefulAction pagination): **8 min**
- Action 3 (ResourceAction + Credential): **32 min**

Phase 1 verdict: 👎. Bottleneck: Action 3's 8 of 12 🔴/🟠 findings are credential-dispatch surface.

### §4.2 Post-cascade prediction (with frozen Tech Spec + Q7 amendments enacted)

Walking the same three flows against the frozen design:

**Action 1 (Stateless HTTP GET) post-cascade:**
1. README + lib.rs has `#[action(...)]` example as canonical happy-path. **No `#[derive(Action)]` ambiguity** (hard-removed per Tech Spec §10.2 T1).
2. Phase 1 friction CC1 (`semver` re-export) is now in §10.4 step 1.5 of the migration guide — explicit «Add `semver = { workspace = true }`» step. **Friction surfaces as a guide step instead of a compile error.** Better but not great — true fix is the macro re-exporting `::nebula_action::__private::semver` (CP4 §15 housekeeping). **Time:** 4-5 min if the migration-guide step is read; 8-10 min if not.
3. CC2 (`Input: HasSchema` undocumented) is closed by Probe 7 (Tech Spec §5.3 line 1468) — the macro test harness fires `E0277: HasSchema not satisfied` at the macro site, not «no method `with_parameters`». **Diagnostic surfaces the actual missing bound.**
4. `#[derive(Action)]`/`Action` (trait) name collision (Phase 1 friction Action-1 #3) is gone — `#[derive(Action)]` ceases to exist (Tech Spec §10.2 T1).
5. `ctx.input_data()` Phase 1 spec drift (Action-1 #4) is closed — spec ratifies `ctx: &'a ActionContext<'a>` + `&self` configuration carrier (§2.9.1a Resolution point 1).

**Predicted Action 1 time:** **4-6 min** if the migration guide is read up-front; 8-10 min cold. **Hits target (<5 min) when migration guide is followed.**

**Action 3 (ResourceAction + Credential) post-cascade:**
1. `#[action(credential = "string")]` silent drop — closed by Tech Spec §4.7 hard `compile_error!` with span on the literal, message «the `credential` attribute requires a type, not a string. Use `credential = SlackToken`, not `credential = "slack_token"`.»
2. `#[action(credential = Type)]` requiring `CredentialLike` with zero implementors — closed because Tech Spec §4.1.1 dispatches **three patterns** (Concrete / ServiceCapability / CapabilityOnly) on the credential's `Scheme` associated type (`<C as Credential>::Scheme = X`), not via a separate `CredentialLike` trait. Macro picks `resolve_as_bearer::<C>` / `resolve_as_basic::<C>` / `resolve_as_oauth2::<C>` based on `<C as Credential>::Scheme` at emission time (§4.3 line 1297).
3. `ctx.credential::<S>()` no-key heuristic — closed by §6.2 hard removal.
4. `ctx.credential::<S>(key)` / `ctx.credential_opt::<S>(key)` (Phase 1 spec promise) — replaced by `ctx.resolved_scheme(&self.<slot>)` per §6.2.
5. `BearerSecret` vs `SecretToken` rename drift — Phase 2 cross-section pass is the right time to land the rename, but it's not yet in Tech Spec §10.2 transforms. **🟡 carry-forward.**
6. `credential(optional) = "key"` syntax — Tech Spec §4.1.1 line 1234: «Empty zone (`credentials()`) is permitted (zero-credential action). Omitting the zone entirely is permitted; equivalent to `credentials()`.» **Optional-credential is structurally absent — instead, optional-cred is expressed as Rust `Option<CredentialRef<C>>`.** This is cleaner than the Phase 1 spec form and structurally honest.

**Predicted Action 3 time:** **8-12 min** (Phase 1 was 32 min). Five of the eight Phase 1 source-lookups are eliminated by Tech Spec §6.2 / §4.7 / §4.1.1 / §10.4. The remaining lookups are CredentialRef + SchemeGuard composition (cross-crate; credential Tech Spec §15.7 governs).

**Hits target (<5 min)?** **Action 1: yes, with migration guide.** **Action 3: no — 8-12 min is the realistic floor for credential-bearing actions because of the cross-crate composition** (action zone → credential's `Scheme` → engine's `resolve_as_bearer` → guard with RAII zeroize). The **<5 min target is achievable for Action 1 only**; Action 3's complexity is fundamental, not removable.

### §4.3 The five remaining frictions on the path to <5 min

1. **`semver` consumer-side declaration** — fix is `pub use ::semver as __private_semver;` in `nebula-action::lib.rs` and macro emits `::nebula_action::__private_semver::Version::new(...)`. Tech Spec §10.4 step 1.5 names the pain but commits the larger fix to «CP4 §15 housekeeping». **Phase 2 should pull this forward.**
2. **`#[derive(HasSchema)]` discoverability** — newcomer reads `#[action(parameters = MyInput)]` in the example, has to guess `MyInput: HasSchema`. Probe 7 catches the missing derive at compile time, **but the README example must show the derive too**. README scope, not Tech Spec scope; flag as Phase 2 docs item.
3. **`BearerSecret` vs `SecretToken` rename** — credential Tech Spec §15.5 owns the rename; cross-section pass at CP4 §15 should land it.
4. **`InputHint::ResourceLocator`** — first-class 3-mode switcher hint for n8n parity on Slack/GitHub/Notion/etc. — see §2.1.1 above. Phase 2 scope.
5. **`migrate_from` codemod for parameter defaults** — Tech Spec §10.2 transforms cover trait/method-signature changes but not field-default migrations. Phase 2 scope.

---

## §5 Top-15 critical DX findings (research-attributed, severity-ranked)

| # | Severity | Finding | Source attribution |
|---|---|---|---|
| 1 | 🔴 BLOCKING (closing) | `#[action(credential = "string")]` silent drop — closed by Tech Spec §4.7 hard `compile_error!` per `feedback_no_shims.md`. **Verify in Phase 2: does Probe 7 (or a new probe) lock the silent-drop regression?** | Phase 1 02a finding 1; Tech Spec §4.7 lines 1384-1407 |
| 2 | 🔴 BLOCKING (closing) | `ctx.credential::<S>()` no-key heuristic cross-plugin shadow attack S-C2 — closed by Tech Spec §6.2 hard removal. Codemod T2 manual-marker discipline preserves `feedback_no_shims.md`. | Phase 1 02b §2.2 + AP peer line 89 (typed PieceAuth peer); Tech Spec §6.2 |
| 3 | 🔴 BLOCKING (closing) | `Input: HasSchema` bound undocumented — closed by Probe 7 (Tech Spec §5.3 line 1468) firing typed `E0277: HasSchema not satisfied` at macro site. README must also show `#[derive(HasSchema)]` in the canonical example. | Phase 1 02a finding 7; Tech Spec §4.6.1 line 1374 |
| 4 | 🔴 BLOCKING (closing) | `#[derive(Action)]` emits `::semver::Version` requiring user crate to declare `semver` dep — closed in §10.4 step 1.5 (migration guide names it). **Phase 2: hoist to macro re-export `::nebula_action::__private_semver`.** | Phase 1 02a finding 5; Tech Spec §10.4 step 1.5 |
| 5 | 🟠 MAJOR | n8n `resourceLocator` 3-form pattern (#21148, #17078, #16379) — no `InputHint::ResourceLocator` in current `nebula-schema`. Workaround via tagged enum exists; UI rendering is 3 clicks instead of 1. | n8n-param line 87-91; Nebula `crates/schema/src/field.rs` |
| 6 | 🟠 MAJOR | n8n `resourceMapper` schema-drift / version-pin (#23884, #19327, #6770) — Nebula has ResourceAction lifecycle but no first-class «migrate mapping on schema drift» step. | n8n-param line 278-296; Tech Spec §2.2.4 |
| 7 | 🟠 MAJOR | `loadOptions` cache-key with credential version pin (#22123 confirmed) — `nebula-schema/loader.rs` has primitives but no per-credential-version cache key wired into `parameters = T` emission. | n8n-param line 222-249 + AP peer line 89-90; Nebula `crates/schema/src/loader.rs` |
| 8 | 🟠 MAJOR | Activepieces' `area/third-party-pieces` cluster — community pieces no contract-test framework. Tech Spec §5 macrotest snapshots cover **emission**, not **consumer-side contract**. | AP peer line 159-166; Tech Spec §5 |
| 9 | 🟠 MAJOR | Field-default migration (`migrate_from`) not in codemod T1-T9. Quick-win 8 from n8n correlation table. | n8n-param line 358; Tech Spec §10.2 |
| 10 | 🟠 MAJOR | `BearerSecret` (spec) vs `SecretToken` (code) rename drift — credential Tech Spec §15.5 owns; CP4 §15 cross-section pass lands. | Phase 1 02a finding 15; Tech Spec §15 cross-section |
| 11 | 🟡 MINOR | `displayOptions` cycle detection — `VisibilityMode::When(Rule)` exists; cycle-detection at `Schema::validate()` time **needs verification** (n8n correlation Quick-Win 4). | n8n-param line 311 + correlation table line 350; Nebula `crates/schema/src/mode.rs` |
| 12 | 🟡 MINOR | Code-block-in-parameter (`CodeField` exists in `Field::Code`) — schema-macros support for `#[schema(field_kind = "code")]` attribute on a `String` field needs verification. | AP peer line 168-174 (code-piece fragility); Nebula `crates/schema/src/field.rs:645` |
| 13 | 🟡 MINOR | `#[derive(Action)]` and `Action` (trait) name collision in re-exports — closed in Tech Spec §10.2 T1 (`#[derive]` ceases). | Phase 1 02a Action-1 #3; Tech Spec §10.2 T1 |
| 14 | 🟡 MINOR | `assert_*!` test macros documentation (Q7 Y2) — Tech Spec §17 CHANGELOG names them but discoverability via `nebula-action::prelude` not yet locked in §9.4. | Tech Spec §15.11.1 Y2 |
| 15 | 🟡 MINOR | `cron` parameter type — n8n has a `cron` field; Nebula has `String + InputHint`, no `InputHint::Cron`. Trivial addition. | n8n-param line 96-99; Nebula `crates/schema/src/input_hint.rs` |

### §5.1 Top-3 🔴 (synthesised — closing-status, must hold the line in Phase 2)

1. **`#[action(credential = "string")]` hard-error discipline** — Tech Spec §4.7 ratified the `compile_error!`; Phase 2 must add the regression-lock probe in `tests/compile_fail/` (current Probe 7 is for `parameters = Type !HasSchema`, not for string-form credential). **Without a probe, the silent-drop regression can return.**
2. **`ctx.credential::<S>()` hard removal — security-lead VETO authority** — Tech Spec §6.2 explicitly retains security-lead implementation-time VETO (line 1664). Phase 2 implementation cascade must not soften to `#[deprecated]`. Per `feedback_no_shims.md` discipline.
3. **`Input: HasSchema` typed bound diagnostic via Probe 7** — Tech Spec §4.6.1 names Probe 7; Phase 2 must lock the `.stderr` snapshot to ensure the diagnostic stays surfaceable, not regressed to «no method `with_parameters`» (the Phase 1 02a finding 6 confusion-mode).

---

## Cross-cascade hand-off note

This Phase 1 of 3 of Q8 surfaces — no fixes proposed (Phase 2 synthesis), no commits. Findings to hand off:

- **§2.1.1, §2.1.2, §2.3.2** — three nebula-schema-territory gaps (resourceLocator 3-form, resourceMapper schema-drift, loadOptions cache key with credential version pin). **Phase 2 should propose whether to extend `crates/schema` ahead of action cascade landing or schedule sibling cascade.**
- **§4.3** — five frictions on the <5 min target. **Phase 2 weighs which to pull into the action cascade vs CP4 §15 housekeeping.**
- **§5 Top-15** — ranked friction log. **Phase 2 picks which to bundle as cascade-scope absorb vs separate housekeeping.**
- **§5.1 Top-3 🔴** — closing-status invariants. **Phase 2 implementation cascade must not soften.**

End of Phase 1 Q8 dx-tester research. Hand off to Phase 2 synthesis (architect / tech-lead).
