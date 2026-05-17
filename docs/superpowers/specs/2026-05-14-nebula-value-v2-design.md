# nebula-value v2 — Technical Specification (ТЗ)

> Status: **Draft for review** · Date: 2026-05-14 · Origin:
> CONFERENCE-DAY9.md Round V · Supersedes the Feb-2026 rollback
> (`aa7792bf` migrate-to-serde_json::Value).
>
> This spec defines a **new, lean `nebula-value`** — designed from the
> lessons of the removed 39k-LOC crate, not a restoration of it.
> Format follows the project's design-spec convention.

---

## 1. Mission

Provide the **value substrate** for the entire Nebula workspace: the
single runtime data type that crosses every boundary (schema,
validator, expression, action, resource, credential properties,
durable state, transport). It must do what `serde_json::Value`
structurally cannot — exact decimals, zero-copy bytes, native
temporal, stable ordering, version-frozen durable serialization —
while staying small, serde-interoperable, and free of the
architectural cycle that killed the previous crate.

### Non-goals (explicitly OUT of this crate)

Lesson from the 39k-LOC bloat (matklad, verified against git):

- **Arithmetic / comparison / logical ops on `Value`** (`ops.rs`)
  → live in `nebula-expression`.
- **Value diffing** (`diff.rs`) → `nebula-expression` or a dedicated
  consumer.
- **Path access** (`path.rs`, JSONPath-like) → `nebula-expression`.
- **Schema** (`schema.rs`) → `nebula-schema` owns schema; value does
  not duplicate it.
- **Concurrency machinery** (`dashmap`/`arc-swap`/`parking_lot`) →
  not a data-layer concern; removed.
- **Validation** → `nebula-validator` depends on value, never the
  reverse.

The crate is the **data**, not the operations on the data.

---

## 2. Layer placement (Q1)

`nebula-value` is a new **Foundation-zero** layer, below Core.

```
Foundation-zero : nebula-value          (depends on NO nebula-* except nebula-error)
Core            : nebula-validator → nebula-schema → nebula-expression
                  (each depends on nebula-value; never the reverse)
Business / Exec / API : as today
```

**Dependency direction is strictly one-way.** `nebula-value` imports
no `nebula-validator` / `nebula-schema` / `nebula-expression`. The
cycle that disabled `nebula-validator` in the old crate's
`Cargo.toml` is **structurally impossible** here (dtolnay).

External deps (minimal): `serde` + `serde_json` (interop only),
`indexmap` (ordered Object), `im` (persistent Array), `bytes`,
`rust_decimal`, `chrono` (behind `temporal` feature), `nebula-error`.
Dropped vs old: `dashmap`, `arc-swap`, `parking_lot`, `async-trait`,
`tokio`, in-crate `schema`.

`deny.toml` `[wrappers]`: `nebula-value` gets an empty allowlist;
upper layers wire it, never the reverse.

---

## 3. The `Value` enum (types)

Based on the real removed code (`core/value.rs:21`), with the four
fixes the audience derived from inspecting it:

```rust
#[non_exhaustive]                       // Wes McKinney: Arrow variant later (DataFusion ScalarValue↔Arrow precedent)
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    Null,                               // #[default]
    Boolean(bool),
    Integer(Integer),                   // newtype over i64 — int≠float at type level (CEL/bson/CBOR universal)
    Float(Float),                       // wraps NotNan<f64> — total order (VRL precedent; durable hashing)
    Decimal(Decimal),                   // rust_decimal + explicit (precision: u8, scale: i8) — DataFusion/Polars/bson universal
    Text(Text),                         // Arc<str> — UTF-8; distinct from Bytes (CEL/bson safer model)
    Bytes(Bytes),                       // bytes::Bytes — zero-copy (Charter P5)
    Array(Arc<[Value]>),                // CEL precedent — immutable snapshot, Arc-clone O(1), no `im` dep
    Object(IndexMap<Key, Value>),       // insertion order (T13); durable hashing canonicalizes sorted (Niko)
    // NO Redacted variant — see §4 (removed 2026-05-15 after user challenge)
    #[cfg(feature = "temporal")] Date(Date),
    #[cfg(feature = "temporal")] Time(Time),
    #[cfg(feature = "temporal")] DateTime(DateTime),
    #[cfg(feature = "temporal")] Duration(Duration),
}
```

Evidence-driven choices (Round V research, primary sources):
`Float=NotNan<f64>` and `Ord` derive — VRL precedent, required for
durable content-hashing and map keys. `Array=Arc<[Value]>` — CEL
precedent, workflow data is immutable snapshots so persistent `im`
is unnecessary weight. `Decimal` carries `(precision, scale)` in the
value — DataFusion/Polars/bson are unanimous; f64-money rejected by
the entire surveyed industry. `Text`+`Bytes` distinct — CEL/bson
safer model for untrusted input vs VRL strings-as-bytes.

Newtype scalars (`Integer`, `Float`, `Text`, `Decimal`) preserved —
they carried invariants and total-ordering/hashing the bare
primitives lack. Kept lean: each is a thin wrapper + conversions, no
ops.

### Fixes vs the removed crate (derived from code inspection)

| Fix | Why | Source |
|---|---|---|
| `Object` = `IndexMap` not `im::HashMap` | old crate silently lost insertion order — this was the T13 bug found in Round 1 | Niko Matsakis, verified in code |
| `#[non_exhaustive]` | add `Arrow(RecordBatch)` (B-02) later without a breaking change; old enum was exhaustive | Wes McKinney |
| zero `nebula-*` value deps | old `Cargo.toml` had `nebula-validator` disabled for a cycle | dtolnay, verified in Cargo.toml |
| no `Value::Secret` (and no `Value::Redacted` — removed 2026-05-15) | extractable secret in generic Value breaks Interlude II; redaction is serializer+schema concern | Tony Arcieri, matklad, Niko |
| ops/diff/path/schema removed | this is what bloated 39k → ~3k; they belong in expression/schema | matklad, verified in tree |

---

## 4. Secret (Q2)

**No extractable `Value::Secret`.** Decision and rationale:

- A `Value::Secret(String)` variant would let any holder of `&Value`
  pattern-match and extract plaintext — this **breaks the Interlude II
  invariant** (secret unreachable to Action code; reachable only in
  `Resource::create`).
- Secret material is the exclusive domain of `nebula-credential`
  (`SecretString`, AES-256-GCM, Argon2, zeroize, AAD — Charter
  §12.5). `nebula-value` is the lowest layer and must not depend on
  crypto (`zeroize`/`secrecy`) — that would re-introduce a layer
  inversion.
- A secret form field does not materialize as a plaintext `Value` in
  Action flow: it travels the credential properties pipeline
  (Interlude II) into encrypted `State`. Generic `Value` flow never
  touches it.
- **No `Value::Redacted` variant either** (removed 2026-05-15 after
  user challenge; audience consensus). Rationale: it was proposed
  *before* the Interlude II decision. Once Interlude II guarantees
  the secret never enters the generic `Value` flow (it travels the
  credential pipeline into encrypted `State`), `Value` structurally
  never contains anything to redact — so a `Redacted` marker solves
  a non-existent problem (matklad YAGNI; Tony Arcieri withdrew his
  own proposal). A `Value::Text("[REDACTED]")` sentinel is rejected
  as a stringly-typed anti-pattern (collision with a real `"***"`
  value; not programmatically distinguishable — Niko).
- **Redaction is a serializer + schema concern, not a value
  property** (withoutboats). When output (log/trace/error) has
  schema context, the serializer sees `Field::Secret` and prints
  `••••` **without touching the `Value`**. A `Value` logged *without*
  schema context contains no secret by the Interlude II invariant —
  nothing to redact. `redaction = (Value, Schema) → output`, a pure
  function at the presentation boundary, not an enum variant.

This keeps `nebula-value` crypto-free, the enum minimal, and
preserves the Interlude II security model end-to-end.

---

## 5. `nebula-validator` integration (Q3)

One-way: `nebula-value` ← `nebula-validator`.

- `nebula-value` defines `Value`, knows nothing about validation.
- `nebula-validator` depends on `nebula-value`:
  `trait Validate { fn validate(&self, v: &Value) -> Result<…>; }`,
  `Validated<Value>` proof-token lives in the validator crate.
- The cycle that disabled the old crate's validator dep is
  impossible — value never imports validator.
- Schema's proof-token pipeline (`ValidValues`/`ResolvedValues`)
  carries `Value`; the validator checks it; the schema describes it.
  Strictly linear.

---

## 6. Serde interop & durable serialization (Q3 cont.)

- Every `Value` variant and newtype impls `Serialize` +
  `Deserialize`, plus explicit `From<serde_json::Value>` /
  `TryInto<serde_json::Value>` for boundary conversion (reqwest
  `.json()`, sqlx, any crate).
- Serialization is an **explicit, versioned, frozen wire spec** —
  NOT blanket `#[derive(Serialize)]` whose shape drifts across serde
  versions (Maxim Fateev, durable-execution requirement). Wire
  format is documented and versioned alongside the Schema
  Immutability per KEY+Version principle.
- **Durable persistence is decoupled from the in-execution `Value`
  (Restate evidence).** Persisted state = `(opaque bytes, codec_id)`,
  replayed verbatim — the rich typed `Value` lives only in the
  execution layer. The `Value` enum may evolve freely across engine
  versions; durable storage stays stable as bytes + codec version,
  not as a serialized snapshot of the current enum shape. This
  isolates value-type evolution from on-disk format stability
  (Polars' string-type-rewrite corollary: the value layout *will*
  change — keep it behind a stable serialized boundary).
- **Deterministic durable hashing:** `Object` is `IndexMap`
  (insertion order, T13) in memory; the durable/content-hash codec
  canonicalizes via sorted keys so content hashes are stable
  regardless of insertion order (Niko Matsakis).
- Lossy edges documented: `serde_json::Value` has no Decimal/Bytes/
  temporal — `Value → serde_json::Value` of those degrades
  explicitly (Decimal → string `{"$dec":"…"}`, Bytes → base64
  `{"$b64":"…"}`, DateTime → RFC3339 string) with a documented,
  reversible convention. `serde_json::Value → Value` is best-effort.

---

## 7. Schema type mapping (Q4) — Rust types, not ours

Authors write **idiomatic Rust**; the derive maps to `Value`. Our
types appear only where Rust is impoverished. Extends the existing
`crates/schema/macros/src/type_infer.rs`.

| Author writes (Rust) | Maps to `Value` | Notes |
|---|---|---|
| `String` / `&str` | `Text` | |
| `i8..i64` / `u8..u64` | `Integer` | `i128/u128/isize/usize` rejected (as today) |
| `f32` / `f64` | `Float` | |
| `bool` | `Boolean` | |
| `Vec<T>` | `Array` | recursive |
| `Option<T>` | inner, `required=false` | |
| `HashMap<String,V>` / `BTreeMap` | `Object` (→ `Field::Map`, Round 1) | string keys |
| `nebula_value::Decimal` | `Decimal` | author writes our type **explicitly** — Rust has none; correctness, not barrier (Brian Goetz) |
| `nebula_value::{DateTime,Date,Time,Duration}` | temporal | explicit, where Rust std is weak |
| `nebula_value::Bytes` / `bytes::Bytes` | `Bytes` | binary/streaming |

Junior writes `struct Form { name: String, age: i64, tags:
Vec<String> }` — zero new types. `Decimal`/`DateTime` surface only
when genuinely needed (money, time). Progressive disclosure (Alice
Ryhl). `#[derive(Serialize, Deserialize)]` on the author's struct
works unchanged (withoutboats).

---

## 8. Developer experience (Q5)

Invariants:

1. **`Value` is hidden by the derive** — 4-line Hello World never
   names `Value`.
2. **serde just works** — author structs derive Serialize/Deserialize
   normally; our types are serde-compatible + `From/Into
   serde_json::Value`.
3. **Our types are progressive** — surface only when Rust cannot
   express the need.
4. **Zero-cost clone** — `Text=Arc<str>`, `Array=im::Vector`
   (structural sharing), `Bytes=bytes::Bytes` (refcount). Node→node
   passing is not a deep copy.
5. **Typed conversion errors** with suggestions —
   `#[diagnostic::on_unimplemented]` on conversion traits; "expected
   text, got integer; use `.to_string()` or change the field type".
6. **`#[non_exhaustive]`** — future `Arrow` variant (B-02) without a
   breaking change.

---

## 9. Crate size budget

Target: **~2-4k LOC** (vs removed 39k). Value enum + newtype scalars
+ collections wrappers + serde + conversions + temporal (feature).
CI budget assertion (analogous to A-1/A-2 for nebula-action). If a
PR pushes value-crate LOC past budget, it is a signal that
ops/diff/path leaked back in — reject and relocate.

---

## 10. Migration from current `serde_json::Value`

Workspace currently uses `serde_json::Value` everywhere (post
`aa7792bf`). Migration is the reverse of that commit, but **lean**:

1. Introduce `nebula-value` (this spec) as Foundation-zero.
2. Type alias bridge during migration: crates accept
   `impl Into<Value>` at boundaries; `From<serde_json::Value>` keeps
   external-crate ergonomics.
3. Migrate Core layer first (validator/schema/expression), then
   Business (credential properties, resource config, action I/O).
4. `serde_json::Value` remains the **external interchange** type at
   crate.io boundaries (reqwest/sqlx); `Value` is the **internal**
   substrate. Explicit conversion at the edge, documented.
5. Hard breaking change, pre-1.0 acceptable (per
   `feedback_hard_breaking_changes`).

---

## 11. Ecosystem evidence (verified, primary sources, 2026-05)

Research of comparable Rust projects (CONFERENCE-DAY9 Round V).
**Every project handling richer-than-JSON data ships a custom value
enum**; only pure JSON parsers mirror `serde_json::Value`, and even
they add precision/zero-copy escapes.

| Project | Custom | Decimal | Bytes | Temporal | Ordered map | Reason |
|---|---|---|---|---|---|---|
| Polars `AnyValue` | Y | i128(p,s) | Y | Y | Arrow Struct | Arrow columnar zero-copy |
| DataFusion `ScalarValue` | Y | 32/64/128/256 | Y +View | full | Arrow Map | lossless scalar↔Arrow array |
| bson `Bson` | Y | Decimal128 | Y +subtypes | Y | indexmap | JSON can't express DT/binary; int≠double |
| VRL (Vector) | Y | f64 | strings=Bytes | Timestamp | BTreeMap | progressive typing over events |
| CEL | Y | — | Arc bytes | Dur/TS | Map | spec types; int≠uint |
| simd-json/sonic-rs | mirror serde_json | — | — | — | — | pure JSON; +RawNumber/Borrowed escapes |
| ciborium/rmpv | Y | — | Y | tag/ext | Vec-pairs | wire richer than JSON |
| Restate | N — opaque bytes+codec | n/a | bytes | n/a | n/a | durability = bytes + codec id |

Sources: docs.rs (datafusion-common 53.1.0, polars, bson,
cel-interpreter, simd-json, ciborium, rmpv), GitHub
(vectordotdev/vrl, cloudwego/sonic-rs), pola.rs blog, restate.dev
blog. Full citations in CONFERENCE-DAY9.md Round V.

**Key evidence-backed conclusions:**
- `serde_json::Value` disqualified by requirements, not preference.
- `Decimal` with explicit **(precision, scale) carried in the
  value** is universal (DataFusion/Polars/bson) — not a sidecar.
- int≠float at the type level is universal (CEL/bson/CBOR/MsgPack);
  conflation breaks idempotent round-trip + durable replay.
- ordered map + total-order float universal (VRL `NotNan`+BTreeMap,
  bson indexmap); `HashMap` rejected by all.
- distinct `Text`(UTF-8) + `Bytes` is the safer model for a typed
  engine with untrusted input (CEL/bson) vs strings-as-bytes (VRL).
- separate in-execution typed value from persisted form (Restate:
  opaque bytes + codec id) — version the value freely, keep
  durability stable.

## 12. Open questions — RESOLVED on evidence (2026-05-15)

- **OQ-1 Decimal** → **`rust_decimal` (Decimal128-class) with
  explicit `(precision, scale)` carried in the `Decimal` value**, as
  DataFusion/Polars/bson unanimously do. NOT `bigdecimal` — no
  surveyed project uses unbounded for this.
- **OQ-2 Array** → **`Arc<[Value]>`** (CEL precedent), NOT
  `im::Vector`. Workflow node→node data is immutable snapshots;
  Arc-clone is O(1) and sufficient; drops the heavy `im` dependency.
  `im::Vector` only pays off for persistent structural updates,
  which this engine does not do.
- **OQ-3 Redacted** → **REMOVED entirely** (2026-05-15, user
  challenge + audience consensus). No variant, no sentinel. Redaction
  is a serializer+schema concern; Interlude II already guarantees
  the secret never enters `Value`, so there is nothing to redact in
  the value model. Tony Arcieri withdrew the original proposal;
  matklad YAGNI; Niko rejected the Text-sentinel anti-pattern. See
  §4.
- **OQ-4 ops** → `Value: PartialEq + Eq + Hash + PartialOrd + Ord`;
  **`Float = NotNan<f64>`** (VRL precedent — total order required for
  durable content-hashing and map keys). Arithmetic/coercion stays
  in `nebula-expression`.
- **OQ-5 Arrow** → reserve via `#[non_exhaustive]` only (DataFusion
  `ScalarValue`↔Arrow proves the path); add the variant in B-02.

**Q3 refinement (Restate lesson):** durable persistence =
`(opaque bytes, codec_id)`, decoupled from in-execution `Value`
evolution. The value enum may evolve freely across versions; durable
storage stays stable as bytes + codec version. `Object` is
`IndexMap` in memory (T13 declared order); durable hashing
canonicalizes via sorted keys so content hashes are deterministic
regardless of insertion order (Niko Matsakis).

**§3 enum updated:** `Float(Float)` where `Float` wraps
`NotNan<f64>`; `Array(Arc<[Value]>)`; `Object(IndexMap<Key,
Value>)`; `Decimal` carries `(rust_decimal::Decimal, precision: u8,
scale: i8)`.

---

## 12. Decision log (Round V, CONFERENCE-DAY9.md)

| Q | Decision | Driver |
|---|---|---|
| Q1 layer | Foundation-zero, one-way deps, cycle structurally impossible | dtolnay, matklad, Carl Lerche |
| Q2 Secret | NO `Value::Secret` AND no `Value::Redacted` (removed 2026-05-15); secret stays in nebula-credential; redaction = serializer+schema concern | Tony Arcieri, withoutboats, matklad, Niko |
| Q3 validator | one-way `value ← validator`; explicit versioned serde spec | dtolnay, Maxim Fateev |
| Q4 schema types | idiomatic Rust; our types only where Rust is weak; extend type_infer | Niko, Brian Goetz, Alice Ryhl, withoutboats |
| Q5 DX | Value hidden by derive; serde just works; progressive; zero-copy; typed errors; non_exhaustive | Alice Ryhl, Esteban, Carl Lerche |
| size | ~2-4k LOC budget, CI-enforced | matklad |
| fixes | IndexMap (T13), non_exhaustive, no ops/diff/path/schema/concurrency | code inspection consensus |

---

*End of nebula-value v2 ТЗ draft. Pending user approval of decisions
Q1–Q5 and open questions OQ-1…OQ-5.*
