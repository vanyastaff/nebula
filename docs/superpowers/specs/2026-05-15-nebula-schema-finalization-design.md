# nebula-schema finalization — unidirectional schema/validator seam + API integration

**Status:** Drafted 2026-05-15. Synthesis of a four-seat adversarial panel
(Rust 1.95 type-system / radical SOLID critic / layering+canon enforcer /
security abuse-case adversary). Design approved as-is by the user; scope and
boundaries decided by the panel per the user's mandate ("experts decide,
defend SOLID + layer boundaries, breaking changes OK if spec-correct").

## Context

`nebula-schema` was framed as "unfinished". Thorough survey contradicts that:
the proof-token pipeline `ValidSchema → ValidValues → ResolvedValues`, the
13-variant `#[non_exhaustive]` `Field` enum (ADR-0003), the derive family,
lint passes, and JSON-Schema export (`schemars` feature) are implemented and
tested (~11k LOC + 3.6k LOC tests). The remaining work is **not** "build the
crate" — it is closing real seams and standing canon violations the panel
surfaced. Five briefing assumptions were wrong and are corrected here because
the plan must be built on the corrected facts:

1. **"~15 library panics" is false.** Almost every flagged `panic!` is inside
   `#[cfg(test)]` / test helpers (verified: `value.rs:843`, `value.rs:645`,
   `schema.rs:886`, `schema.rs:912`, all `loader.rs:*`). The real depth guard
   (`value.rs:90/137/541/604`) already returns a typed
   `recursion_limit_error()`. The **only** genuine library-code violation is
   `crates/schema/macros/src/derive_schema.rs:95`: `#[derive(Schema)]` emits a
   `panic!` into downstream user crates (transitively breaks the AGENTS.md
   no-panic rule for every derive user), with `:126` `.expect("…valid
   FieldKey")` as a softer second offender.

2. **`nebula-schema` already depends on `nebula-validator`, and the predicate
   engine already lives in validator.** `crates/schema/Cargo.toml` →
   `nebula-validator` path dep; `crates/schema/src/mode.rs:3` →
   `use nebula_validator::Rule`; `Predicate::evaluate(&PredicateContext)` +
   `PredicateContext` already live in `nebula-validator`. Q3 is ~80%
   codification of where the engine already is, **plus** a
   visibility/required *reporting* surface that exists nowhere yet.

3. **The two-responsibility smell is in `validated.rs`, not
   `validator_bridge.rs`.** `validator_bridge.rs` is a pure path/error
   coordinate translator — a *symptom*. The crate boundary violation is
   `crates/schema/src/validated.rs:333` (`ValidSchema::validate` owns a
   `ValidationReport` populated with validator codes), `:1699`/`:1712`
   (`run_rules`/`run_root_rules` construct `PredicateContext` and call the
   validator), and the error re-map (`push_validator_rule_errors`,
   `translate_validator_code`).

4. **`RequiredMode::When(Rule)` is a standing §4.5 [L1] false capability.** It
   is a public type with **zero engine consumers** (`value.rs` never reads
   it). Shipping/keeping it without an evaluator is a canon §4.5 +
   §14 ("phantom types") violation that exists *today*.

5. **The real API violation is not the `serde_json::Value` transport.**
   `serde_json::Value` for credential `data` is canon-sanctioned (ADR-0011
   §Decision-2, PRODUCT_CANON §12.4 [L3]). The standing violation is that
   `crates/api/src/services/credential.rs:177` ("Validate `req.data` against
   `Credential::schema()`") is a never-implemented TODO, masked only by a 503
   stub — a §4.5 [L1] + §10 [L2] violation the moment the write path goes
   live. `ValidSchema::json_schema()` exists (`crates/schema/src/json_schema.rs`)
   but is never called by the API; catalog endpoints
   (`crates/api/src/services/credential.rs` `list/get_credential_type`) return
   503 with the `schema` field unpopulated.

Canon constraints honored: PRODUCT_CANON §3.5 (structural contract), §4.5
(false capability), §12.4/§12.5; INTEGRATION_MODEL §29/§33 (proof-token
custody — tokens minted only by `nebula-schema`); ADR-0002 (token seam),
ADR-0003 (Field enum, untouched), ADR-0011 (typed config vs `Value`),
ADR-0034 (`SecretValue` in `nebula-schema`, redact, promote at resolve),
ADR-0047 (OpenAPI 3.1, layer boundary), deny.toml `[wrappers]`.

## Decision

### Adjudicated responsibility line (resolves the panel's one real conflict)

The SOLID critic's binary success criterion ("zero `nebula-validator`
dependency in `nebula-schema`") is **rejected** as over-rotation: ADR-0003
makes `Field` a carrier of `Rule` *as data*; removing the type would make a
validated field undeclarable. The defensible line, supported by the
type-system expert and the canon enforcer:

| Concern | `nebula-schema` (Core) | `nebula-validator` (Core) |
|---|---|---|
| Data | `Field` typestate; `VisibilityMode`/`RequiredMode`/`Rule` as **inert data**; `SecretValue` (ADR-0034); JSON-Schema export | `Rule`/`Predicate`/`PredicateContext`; **evaluation engine**; unified `ValidationReport` |
| Behavior | proof-token mint (`validate`/`resolve` sole token constructors); **structural** conformance only | **all** rule/visibility/required evaluation + reporting + error mapping |
| Deleted | `run_rules`/`run_root_rules` execution, `validator_bridge.rs`, `translate_validator_code` | `RuleContext`, `Rule::evaluate` |

The `nebula-schema → nebula-validator` dependency stays (it carries the `Rule`
*type* as field data) but becomes **behaviorally unidirectional**: schema no
longer executes evaluation nor owns the validator→schema error mapping.
`nebula-validator` never depends on `nebula-schema`. The
`ValidSchema → ValidValues → ResolvedValues` typestate is **unchanged** —
condition evaluation was always an internal predicate of `validate()`, never
part of the proof token (INTEGRATION_MODEL §29/§33).

### Q3 — visibility/required engine into `nebula-validator`

New `nebula_validator::policy` module:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presence { Active, Skipped }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Requiredness { Required, Optional }

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum VisibilityPolicy { Always, Never, When(Rule) }

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RequiredPolicy { Never, Always, When(Rule) }

impl VisibilityPolicy {
    /// THE ONLY public way to turn a policy into a decision — no
    /// `evaluate(&self) -> bool` exists, so a caller cannot get a raw
    /// bool and "remember" to branch on it.
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Presence;
}
impl RequiredPolicy {
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Requiredness;
}

impl Rule {
    /// Replaces `Rule::evaluate(&dyn RuleContext)`, which silently
    /// returned `false` for nested JSON-Pointer paths
    /// (crates/validator/src/rule/mod.rs:175-182) — a fail-open bug in
    /// `required` gating for credentials.
    #[must_use]
    pub fn matches(&self, ctx: &PredicateContext) -> bool;
}

/// Validator-owned reporting (the "required reporting" Q3 moves here).
pub fn resolve_field_policies<'a>(/* fields + ctx */)
    -> FieldPolicyResolution<'a>; // { plans: Vec<FieldPlan>, required_failures: ValidationErrors }
```

Binding rules:

- **`RuleContext` and `Rule::evaluate` are deleted, not deprecated.** Their
  own `TODO(post-refactor)` (`crates/validator/src/rule/mod.rs:184`) names
  exactly this migration. A deprecation shim would leave the nested-path
  fail-open path live for any future caller (no-shim discipline).
- schema-side hidden-field skip becomes structurally unrepresentable
  otherwise: `match plan.presence { Presence::Skipped => continue,
  Presence::Active => run_value_rules(...) }` — `#[non_exhaustive]` +
  exhaustive in-crate `match` means a future variant fails compilation in
  `nebula-schema` until it decides its skip-ness. Replaces the discipline
  `if !visible && raw.is_none() { return; }` at `validated.rs:1046-1059`.
- **`RequiredMode::When` is wired end-to-end** through
  `resolve_field_policies` (closes the V1 phantom). Required-condition
  evaluation is net-new behavior; it ships in the same cascade — not data
  without engine.
- `nebula-schema`'s `VisibilityMode`/`RequiredMode` store the
  `nebula_validator::policy` types directly (not a re-export shim).
- Error type for `required_failures` reuses
  `nebula_validator::foundation::ValidationError` (80 B, RFC 6901 path) — no
  new error enum.
- **Secret scrub (security hole #1, CRITICAL):** before any
  `PredicateContext` is handed to the validator engine, `nebula-schema` must
  exclude fields whose declared `Field` is `Field::Secret(_)` from the
  context map — keyed on the **schema type**, not the runtime `FieldValue`
  enum tag. Pre-resolve a secret is `FieldValue::Literal(String)` plaintext,
  so the existing tag-based redaction in `crates/schema/src/context.rs:18-29`
  does not fire. Construction site is owned by `nebula-schema`
  (`context.rs`); the validator must receive an already-scrubbed context. A
  new lint (`crates/schema/src/lint.rs`, `secret.predicate_on_value`,
  Severity::Error, same shape as the existing `secret.default_forbidden`)
  rejects any value-comparing predicate (`Eq/Ne/Contains/Matches/In/Gt/...`)
  whose `FieldPath` targets a `Field::Secret`. Secret presence/absence
  (`Set`/`Empty`) remains legal.
- **`PredicateContext` Debug redaction (hole #5, MED):** hand-implement a
  redacting `Debug` for `PredicateContext`
  (`crates/validator/src/rule/context.rs:12`, currently `derive(Debug)`) so a
  future span/log in the moved engine cannot leak plaintext.

ADR required: see "ADRs" below (**ADR-0052**, extends ADR-0002/0003/0034,
honors ADR-0034 §3 redaction at the predicate-JSON boundary).

### HasSchema convergence — business-layer wiring (SOLID: DIP + ISP)

One concept, three trait shapes today: `Action` has `type Input: HasSchema`
**plus** redundant `fn input_schema()/output_schema()`
(`crates/action/src/action.rs:76,79` — ISP fat interface); `Credential` has a
provided `fn properties_schema()` (`crates/credential/src/contract/credential.rs:158`);
`Resource` has the clean `Config: HasSchema` + `<R::Config as
HasSchema>::schema()` (`crates/resource/src/manager/mod.rs:656`). Converge on
the `Resource` pattern:

- Delete `Action::input_schema`/`output_schema`; the associated-type bound is
  the single source of truth.
- Delete `Credential::properties_schema`; consumers call `<C::Properties as
  HasSchema>::schema()`.
- Ergonomics without re-stating the abstraction: a free
  `nebula_schema::schema_of::<T>()` helper.
- Zero new crates, zero `deny.toml` change — `HasSchema` stays in
  `nebula-schema` (Core), already importable by all three Business crates.

### Q2 — API boundary: engine-mediated authority (c) + JSON-Schema read-model (a), composed

These are not mutually exclusive; the panel was treating them as a fork.

- **(c) Write-path authority.** API keeps `serde_json::Value` at the HTTP
  edge (canon-sanctioned). The credential write path validates `data` against
  the resolved `ValidSchema` **before persist** — authority sits with the
  rule-semantics owner (validator, invoked via the engine/credential layer,
  which already legally depends on `nebula-schema`). Closes V2. Implements
  the existing `crates/api/src/services/credential.rs:177` TODO. No new
  crate, no `deny.toml` change, no new ADR.
- **(a) Read-model.** `ValidSchema::json_schema()` is produced by the
  credential/engine layer (Business/Exec — already legally depends on
  `nebula-schema`; `schemars` feature enabled there) and crosses to
  `nebula-api` serialized as `serde_json::Value` inside the catalog response
  DTO. `nebula-api` still never imports `nebula-schema`; the `Value` is a
  response body, which ADR-0011 sanctions at the edge. Catalog endpoints
  populate `CredentialTypeInfo.schema` from it. Closes V3. One-paragraph
  **amendment** to ADR-0047 (not a new ADR).
- **Public projection (hole #6, MED).** The public OpenAPI DTO must strip
  `x-nebula-root-rules` and per-field rule operands
  (`crates/schema/src/json_schema.rs:108-116`) — full cross-field predicate
  logic must not leak to unauthenticated clients. The public projection is a
  `nebula-api`-owned mapper, not a raw `json_schema()` passthrough.
- **Rejected: shared wire crate (Q2-b).** A new crate depended on by both
  `nebula-api` (API) and the credential/engine path is a new cross-layer node
  (Stable-Abstractions "zone of pain"), needs a new ADR + a `deny.toml`
  `[wrappers]` stanza, and does not even solve the problem (credential
  schemas are dynamic per credential key — a shared struct is still `Value`).

### JSON-Schema export hardening (security hole #3, HIGH)

`crates/schema/src/json_schema.rs:191,212` (`apply_common_keywords`) copies
`f.default` verbatim into the exported schema for **every** field including
`Field::Secret`. The only guard is the separate advisory `lint.rs:153-165`
pass, never invoked by `json_schema()`. Move the secret-default refusal
**into the exporter** (`if matches!(field, Field::Secret(_))` → never emit
`default`) and add a regression test asserting a secret `default` is absent
even when the builder was fed one.

### ReDoS / DoS hardening (security hole #4, HIGH)

`compile_regex` (`crates/validator/src/rule/helpers.rs:7`) is
`regex::Regex::new` with no `size_limit`/`dfa_size_limit`, recompiled per
`evaluate()` (`crates/validator/src/rule/predicate.rs:102-110`), no cache.
Attacker-authored `Predicate::Matches` patterns cause memory/CPU
amplification (linear *matching* is guaranteed by the `regex` crate, but
*compilation* of huge bounded repetitions is not bounded). Q3 worsens this
(visibility/required predicates run on every schema validate, incl. the
high-frequency credential form). Fix: bounded `RegexBuilder` (size + dfa
limits) returning the existing `ValidationError`; a compiled-regex cache
keyed by pattern; plus a schema-lint cap on pattern string length
(`crates/schema/src/lint.rs`) rejecting over-long patterns before storage.

### The one real panic

`crates/schema/macros/src/derive_schema.rs:95`: the lints it defers to
runtime (visibility cycles, dangling refs) are static properties of the
field graph fully known at macro-expansion time. Run them in the proc-macro
and emit `compile_error!`; make the runtime `HasSchema::schema()` body
infallible by constructing `ValidSchemaInner` directly via a crate-internal
`#[doc(hidden)] valid_schema_from_parts` (generalize the existing
`ValidSchema::empty()` "constructed directly, bypassing lint passes" escape
hatch). `:126` `.expect` → const-assert in the macro + a
`FieldKey::new_unchecked` const-fn constructor so the generated code carries
no panic path.

### ADRs

- **ADR-0052 (new — next free number; verified highest existing = 0051):**
  *"Field-level visibility/required condition evaluation moves into
  `nebula-validator` (co-located with `Predicate::evaluate` /
  `PredicateContext`); `nebula-schema` retains `VisibilityMode`/`RequiredMode`
  as `Rule`-carrying data only and delegates to a validator-owned
  `resolve_field_policies` API; required-condition evaluation is wired
  end-to-end (no §4.5 phantom); `PredicateContext` construction at this
  boundary applies ADR-0034 §3 `SecretLiteral` redaction AND excludes
  `Field::Secret` values by schema type; `ValidSchema::validate` /
  `ValidValues::resolve` remain the sole proof-token mints in
  `nebula-schema`."* `related: [0002, 0003, 0034, 0011]`. **Extends**, does
  not supersede, ADR-0002/0003/0034.
- **ADR-0047:** one-paragraph in-place amendment — catalog
  `CredentialTypeInfo.schema` is populated by `ValidSchema::json_schema()`;
  credential `data` request body stays `serde_json::Value`; the write path
  validates `data` against the resolved `ValidSchema` before persist.
- **ADR numbering landscape (read before allocating 0052).** Three ADR
  number spaces are in play and have diverged: (1) the canon archive
  `C:/Users/vanya/RustroverProjects/docs/adr/` holds `0001–0041` (the
  authoritative canon ADRs cited by this spec — `PRODUCT_CANON.md` 4/27,
  `INTEGRATION_MODEL.md` 4/24, all real and current; that directory ALSO
  contains stale working logs under `superpowers/`, `tracking/`, `drafts/`
  which are NOT authority) and now also a fresh external
  `0042-tool-provider-typed-resource-tools.md` (5/15) — i.e. the external
  archive advanced its own 0042; (2) this worktree's `docs/adr/` holds the
  M6/M11 cascade `0042–0051` per its README convention, with a pre-existing
  in-worktree `0042` filename collision (`0042-layered-retry.md` vs
  `0042-node-binding-mechanism.md`; README indexes only the latter). **ADR-0052
  is allocated deliberately in the worktree cascade space** (next free after
  the worktree's 0051), because the schema-seam decision belongs to the M-cascade
  ADR set, not the external canon archive. Housekeeping (separate trivial PR,
  not fixed here): renumber one of the in-worktree `0042` files; the external
  vs worktree 0042 divergence is a pre-existing archive split, out of scope.

### Phasing

One cascade, four PRs, ordered by dependency:

1. **P1 — Q3 core.** `nebula_validator::policy` module + `Rule::matches`;
   delete `RuleContext` + `Rule::evaluate`; schema delegates via
   `resolve_field_policies`; wire `RequiredMode::When` end-to-end; secret
   scrub (#1) + `PredicateContext` redacting `Debug` (#5). Author ADR-0052 +
   seam test in the same PR (canon §0.1/§17: L2 seam change ⇒ ADR + seam
   test).
2. **P2 — schema cleanup.** Delete `run_rules`/`run_root_rules` /
   `validator_bridge.rs` / `translate_validator_code`. `ValidSchema::validate`
   remains the sole entry point and proof-token mint (signature/postcondition
   unchanged, INTEGRATION_MODEL §29/§33): it computes schema-owned structural
   conformance (presence/type/cardinality) and **delegates** all
   rule/visibility/required evaluation and report assembly to the single
   validator-owned API; the combined `ValidationReport` comes back already in
   validator vocabulary, so schema performs no error translation (that is why
   `translate_validator_code`/`validator_bridge.rs` are deleted, not
   replaced). Fix `derive_schema.rs:95`/`:126`; JSON-Schema secret-default
   refusal (#3); bounded+cached regex (#4).
3. **P3 — HasSchema convergence.** Normalize `Action`/`Credential` to the
   `Resource` pattern; add `schema_of`.
4. **P4 — API.** Write-path schema validation (V2); catalog
   `json_schema()` population (V3); public DTO projection stripping
   `x-nebula-root-rules` (#6); ADR-0047 amendment.

### Plan lockdown requirements (panel final-objection round — 4/4 no blocking objection)

All four panelists returned NO BLOCKING OBJECTION / ACCEPTABLE. The one
adjudicated conflict (the SOLID critic's "zero `nebula-validator` dependency"
criterion, overruled as over-rotation) was accepted by that panelist on
re-review — consensus is earned by reasoned overrule + explicit acceptance,
not unanimity-from-start. Each panelist returned one binding constraint the
implementation plan MUST encode as a merge-blocker, not implementation
discretion:

1. **P2 delegation contract (cohesion).** Pin the exact signature of the
   single validator entry point; schema-owned structural conformance is
   ordered before and short-circuits rule/predicate evaluation so predicates
   never run against structurally-invalid values (else the `Rule::matches`
   fail-open class returns as an under-specified merge boundary); a test
   asserts this is the ONLY schema→validator behavioral crossing.
2. **Presence wiring is data-flow-enforced, not call-order-enforced
   (type-safety, P1).** `FieldPlan` is the sole input type to the field-rule
   loop; `run_value_rules` reachable only via `match plan.presence {
   Presence::Skipped => continue, Presence::Active => … }`; the runner cannot
   observe a raw `Field`/`VisibilityPolicy` except through `plan`; the
   "iterate `fields` + separately consult `HashMap<FieldPath, Presence>`"
   relocation of the old `if !visible && raw.is_none() { return; }` discipline
   is forbidden structurally. trybuild proves enum exhaustiveness, not
   bypass-impossibility — the constraint is explicit, not test-implied.
3. **ADR-0052 + proof-token-custody seam test land inside P1's PR
   (canon §0.1/§17).** Hard P1 merge-blocker — not P2, not "ADR follows";
   P1 is the L2 seam change, so the ADR + seam test ship in the same PR.
4. **#2 is a hard-blocking separate task before the credential write path is
   exposed to untrusted workflow authors in production (security).** Verified:
   P1–P4 neither touches nor worsens it (`Rule::matches` only tightens a
   `required` fail-open; P4 only adds a pre-persist gate; the `&str`-keyed
   resolution signature is unchanged; no new id-controlled caller). "nebula-schema
   finalized" MUST NOT be read as "#2 closed."

## Non-goals

- **Security #2 — `slot_bindings` confused deputy — OUT OF SCOPE, tracked
  separately.** The credential/resource resolution path
  (`crates/action/src/context.rs:788-809`,
  `crates/engine/src/credential/resolver.rs:68-79`) has **no**
  owner/tenant/workspace authorization; a crafted workflow JSON can resolve
  any credential id via `slot_bindings`. This is an engine authorization
  hole, broader than "finish nebula-schema"; folding a workspace-wide auth
  refactor into this spec would violate scope discipline. After all four
  phases land, credential resolution is still confused-deputy-exposed — this
  is stated explicitly so it is not mistaken for closed.
- **"Integration-only" as a terminal state** — canon-illegal (leaves V1/V2/V3
  standing). Not an option; P1–P4 is the minimum legal terminal scope.
- Merging `nebula-schema` into `nebula-validator` — bloats the Core import
  graph for schema-only consumers (`nebula-expression` et al.), re-fuses two
  independent change axes (ADR-0003 typestate vs rule semantics).
- Changing the `Field` variant set (ADR-0003 untouched — no new/removed
  variants).

## Tests

- **Compile-fail (trybuild):** no public path yields a raw `bool` from a
  policy; a `match` on `Presence` missing the `Skipped` arm fails to compile;
  `RuleContext`/`Rule::evaluate` no longer resolve.
- **Regression (schema):** secret-shaped field as `FieldValue::Literal`
  (pre-resolve) is absent from the `PredicateContext` handed to the
  validator; value-comparing predicate on `Field::Secret` is rejected at
  lint with `secret.predicate_on_value`; JSON-Schema export of a
  secret-default field has `default == null`.
- **Regression (validator):** `Rule::matches` resolves nested JSON-Pointer
  paths (the old `Rule::evaluate` fail-open case now passes); bounded regex
  rejects an over-limit pattern with a typed error; `PredicateContext` Debug
  output contains no field values.
- **Behavioral:** `RequiredMode::When` enforces required-ness end-to-end
  (absent → typed `required` failure; condition false → optional).
- **Seam test (ADR-0052):** proof-token custody unchanged — `ValidValues` /
  `ResolvedValues` constructible only via `nebula-schema` pipeline methods.
- **API:** credential create with `data` violating the resolved schema is
  rejected before persist with a typed report; OpenAPI public projection
  round-trips without `x-nebula-root-rules`.

## Verification gate

```
cargo check  --workspace --all-features
cargo nextest run -p nebula-schema -p nebula-validator -p nebula-api -p nebula-action -p nebula-credential -p nebula-resource
cargo test   -p nebula-schema -p nebula-validator --doc
cargo clippy --workspace --all-targets -q -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo deny check
```

(Workspace pre-PR gate is `task dev:check`.) Each phase: conventional commit
(`feat(schema):` / `feat(validator):` / `refactor(action):` / `feat(api):`),
`gh pr create` referencing ADR-0052 (P1) / ADR-0047 amendment (P4). ADR-0052
lands with P1; the `0042` filename collision is a separate housekeeping PR.
