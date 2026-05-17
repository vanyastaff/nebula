# ADR-0061: `nebula-schema` core trait ratification

**Status:** Proposed (2026-05-14)
**Tags:** schema, validator, expression, ratification

## Context

Day 6 honest reckoning at the design conference revealed that
`nebula-schema` is **already production-grade**: 24 modules, 11K LOC,
13 closed-set Field variants, ~20 InputHint variants, three-tier
proof tokens via type-state pattern, Mode field for discriminated
unions, Loader trait, Secret family with Argon2 KDF, JSON Schema
export, lint pass, validator/expression bridge — all shipped.

This ADR **ratifies the existing design** rather than proposing a
redesign. It documents the contract surface, formalizes the
three-sibling boundary with `nebula-validator` and `nebula-expression`,
and records what is closed-set vs extensible.

## Decision

### Core trait shape (ratified, no changes)

```rust
pub trait HasSchema {
    fn schema() -> ValidSchema;        // owned (not &'static)
}

// Object-safe companion (added when first use case emerges, not before):
// pub trait HasSchemaObject { fn schema_object(&self) -> ValidSchema; }
```

`ValidSchema` is opaque struct (constructed via `Schema::builder()`).
`HasSchema::schema()` returns owned `ValidSchema`. Caller may cache via
`OnceLock` if performance matters.

### Three-tier proof tokens (ratified)

Type-state pipeline for value processing:

```
Schema           // builder-time draft
  ↓ .build() — runs Schema::lint pass
ValidSchema      // structurally valid; can call .validate()
  ↓ .validate(values)
ValidValues      // values match schema; can call .resolve()
  ↓ .resolve(ctx)
ResolvedValues   // expressions evaluated; ready to deserialize
```

Each stage produces a new type obtainable only via the previous stage.
Compile-time enforcement of pipeline ordering. Niko Matsakis flagged
this as "academic-grade design that actually shipped."

### Three siblings, one purpose (per F15)

- `nebula-schema` — **describes shape**. Field types, validation
  specs, expression placeholders.
- `nebula-validator` — **checks values**. `Validator` trait,
  registry, built-in rules.
- `nebula-expression` — **resolves placeholders**. AST, evaluation
  engine.

Linear dependency: `schema` → `expression` (uses `ExpressionAst`
type) and `schema` ← `validator` (validator uses schema specs).
No cycles. Documented in `deny.toml` layer wrappers.

### Closed-set extension surfaces (ratified per F12 / F16)

| Surface | Closed/Open | Where |
|---|---|---|
| `Field` enum (13 variants) | Closed | `nebula-schema::field` |
| `InputHint` enum (~20 variants) | Closed | `nebula-schema::input_hint` |
| `Widget` enum (~17 variants) | Closed | `nebula-schema::widget` |
| `Format` vocabulary (~15 entries) | Closed | enumerated in ADR-0058 |
| `Validator` trait | **Open** | `nebula-validator` — extension via custom impl |

New entries to closed sets via ADR amendment + minor bump.

### Specialized Field variants (ratified)

| Variant | Purpose |
|---|---|
| `String` + `InputHint` | Text-based fields with semantic hints (Email, Url, Cron, Date, …) |
| `Secret` | Credentials with `SecretString` backing + audit hooks |
| `Number`, `Boolean`, `Select`, `Object`, `List` | Standard primitives |
| `Mode` | **Discriminated union** — auth scheme variants, body kind variants |
| `Code`, `File` | Domain-specific |
| `Computed` | Expression-only, no user input |
| `Dynamic` | Schema resolved at runtime via `Loader` trait |
| `Notice` | Display-only field for inline help / warnings |

`Mode` field cleanly expresses what JSON Schema needs `oneOf` magic for
— Henry Andrews offered to bring this design to JSON Schema community.

### Conditional fields via Predicate / Rule (ratified)

```rust
Field::secret(field_key!("api_key")).active_when(Rule::predicate(
    Predicate::eq("auth_type", json!("api_key")).unwrap(),
))
```

Typed predicate AST (not stringly-typed expressions). Compile-time
validation of conditional logic via `nebula-validator`.

### Lint pass (ratified)

`Schema::build()` runs structural lint checks before producing
`ValidSchema`. Diagnostic improvements tracked via NS-4.

## Consequences

### Positive

- No redesign: existing 11K LOC stays. Risk of regression zero.
- Charter principles F10-F16 all derive from existing design — no
  forced retrofit.
- Production-readiness today; remaining work is documentation +
  `stdlib` module (per ADR-0062).

### Negative

- File `validated.rs` is 1943 LOC — maintenance hazard. Tier-2 work
  (NS-5) splits it. Not blocking ratification.
- Closed-set discipline requires future ADR amendments for additions
  — slower than open registry.

### Neutral

- `HasSchemaObject` companion trait deferred until first legitimate
  use case (YAGNI per Niko).

## Documentation deliverables (NS-2 / NS-3)

- README section: "Three-tier proof tokens" pattern as flagship
  feature.
- README section: `Mode` / `Computed` / `Dynamic` as Nebula
  extensions over JSON Schema.
- `nebula-schema` rustdoc landing page synthesizes pipeline diagram
  + sibling crate boundaries.

## References

- Conference Day 6 late afternoon (CONFERENCE-NOTES.md) — honest
  reckoning.
- Existing crates: `nebula-schema`, `nebula-validator`,
  `nebula-expression`.
- AGENTS.md layered architecture.

## Out of scope

- `stdlib` newtype module — see ADR-0062.
- JSON Schema lossless interop — see ADR-0063.
- File splits / refactors — Tier-2, not blocking ratification.
- Performance benchmarks — separate `cargo bench` work.
