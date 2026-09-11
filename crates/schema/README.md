---

name: nebula-schema

role: Typed configuration schema and phase-indexed data with schema-bound proofs
status: frontier
last-reviewed: 2026-09-10
canon-invariants: [L1-3.5, L1-4.5]
related: [nebula-validator, nebula-expression, nebula-action, nebula-resource, nebula-credential]
---

# nebula-schema

## Purpose

Core infrastructure for typed configuration shared by Actions, Credentials, and
Resources. The crate owns schema definitions, a canonical value tree, and the
checked transitions from authoring input to resolved runtime data. It replaces
the deleted `nebula-parameter` crate.

`nebula-sdk` is the sole curated, supported Rust product surface. This README
describes `nebula-schema` as an internal technical boundary, not a separately
supported downstream API.

## Role

The proof pipeline has two distinct kinds of state: the expression capability
of a tree and the schema checks certified by its wrapper.

```text
SchemaBuilder::build() -> Result<ValidSchema, ValidationReport>
ValidSchema::validate(AuthoredValue) -> Result<ValidValues, ValidationReport>
ValidValues::resolve(context).await -> Result<ResolvedValues, ValidationReport>
                  or resolve_data() -> Result<ResolvedValues, ValidationReport>
```

`validate` consumes input, folds aliases, applies transforms once, promotes
declared string secrets, and retains admitted `CompiledProgram`s. `ValidValues`
holds a schema snapshot and explicit `PendingValidation` obligations; it is not
yet a runtime proof. Resolution consumes that token and checks full rules and
conditional policies before producing `ResolvedValues`.

`resolve_data` is the synchronous, no-engine path. It rejects any compiled
expression with `expression.forbidden`; it does not skip final validation.

## Value model

| Type | Meaning |
|------|---------|
| `ValueTree<E>` | `Literal(ScalarValue)`, `Object(IndexMap<String, Self>)`, `List(Vec<Self>)`, `Expression(E)`, or `Secret(SecretValue)` |
| `AuthoredValue` | `ValueTree<Expression>`: explicit authoring sources |
| `CompiledValue` | `ValueTree<CompiledProgram>`: retained programs in prepared input |
| `ResolvedValue` | `ValueTree<Infallible>`: expressions are uninhabited |

`ScalarValue::try_from(json)` rejects objects and arrays; `as_json` and
`into_json` expose only its scalar. Every container has one tree representation.
A mode value is an ordinary object with `mode` and optional `value` properties,
interpreted according to a `ModeField`, not a separate tree variant.

- `ValueTree::from_data(json)` is literal-only ingestion. Template-looking
  strings and objects containing `$expr` stay data.
- `AuthoredValue::from_template_json(json)` explicitly enables authoring
  shorthand: template strings and exact `{"$expr": "source"}` objects, using
  AUTO compilation so lone envelopes retain their JSON type.
- `Expression::template(source)` explicitly authors text interpolation, always
  producing a string, including lone `{{ 7 }}`. `Expression::new(source)` uses
  AUTO; `Expression::with_syntax(source, ProgramSyntax::Expression)` selects raw
  expression grammar. `syntax()` is immutable and retained in `CompiledProgram`.
- `insert(key, tree)` returns `Result<Option<Self>, ValidationError>` and
  requires an object receiver. `insert_data` performs literal-only conversion.
- `get(key)` looks up an exact property name; `get_path(&ValuePath)` navigates
  an RFC6901 JSON Pointer. Data keys may be empty, numeric, Unicode, or contain
  dots, slashes, tildes, and brackets.

`FieldKey` and schema `FieldPath` still identify declarations and indexed schema
locations such as `items[0].name`. They are not restrictions on JSON property
names. Data diagnostics use `ValuePath`: `""` is the root, `/` is the empty key,
and `/a~1b/~0` addresses keys `a/b` then `~`. Numeric segments index a list only
when the current node is a list.

## Construction APIs

- `ValidSchema::root_shape()` is the authoritative `RootShape`: `Any`,
  `Scalar(ScalarSchema)`, `Record(RecordShape)`, or `Union(UnionShape)`.
  Kind, declarations, and root rules are derived views, not independent state.
  Unit types and unit structs describe JSON `null`; empty braced structs describe
  objects. Primitive schemas retain their concrete type and numeric bounds.
  Integral JSON numbers are normalized losslessly before typed integer decoding.
- `ValidSchema::scalar(ScalarSchema)` builds a checked scalar root without a
  synthetic field key. Scalar roots admit data only, and their rules still run
  through the same staged validation pipeline.
- `Schema::builder()` and `SchemaBuilder::add` accumulate draft fields;
  `build()` runs structural lint and returns `Result<ValidSchema, ValidationReport>`.
  `Schema::lint()` reports errors and advisory warnings without producing proof.
- `HasSchema::schema()` and `schema_of::<T>()` return
  `Result<ValidSchema, ValidationReport>`. Derived implementations cache either
  the checked schema or its construction report, not a panic fallback.
- `Field` builders require a checked `FieldKey`; use `field_key!("name")` for
  static names or `Field::try_*` for fallible dynamic construction.
- `Transformer::regex(pattern, group)` returns `Result<Transformer, ValidationError>`.
  The `Regex(RegexCapture)` variant contains a compiled pattern and checked
  capture index. Construction and serde reject malformed patterns or unavailable
  groups; `apply` stays string-only and preserves unmatched input.
- `ExpressionContext::evaluate` receives `&CompiledProgram` and returns
  `EvalFuture`. `EngineExpressionContext` delegates to `nebula-expression`.
  Evaluator output is decoded as literal data, never reparsed as authoring syntax.

## Wire and identity

Tree serde uses the authored v2 envelope, independently of JSON views and
canonical byte encodings:

```json
{
  "version": 2,
  "data": {"message": null, "literal": {"$expr": "ordinary data"}},
  "expressions": [{"path": "/message", "syntax": "template", "source": "{{ $input.message }}"}]
}
```

Each expression entry requires `path`, `syntax`, and `source`; syntax is exactly
`auto`, `expression`, or `template`, with no missing-field default. The path
identifies an existing null placeholder. Only
`AuthoredValue` deserializes; decoding does not compile expressions or mint a
proof. Serialization is available for all three phases, but any explicit secret
causes rejection. The decoder rejects unknown or duplicate fields, duplicate
data keys, unsupported versions, invalid or overlapping pointers, and missing
or non-null placeholders. Logical value depth is bounded at 64.

`to_json` is a secret-redacted view, not a lossless authoring wire format;
authored and compiled views still contain expression sources. Schema-bound
`to_wire_json` applies output aliases and omits secrets. Neither view certifies
that values may be persisted or logged.

Tree canonical bytes use `VALUE_CANON_VERSION = 2`. Expression equality,
content IDs, and keyed commitments include both authored syntax and exact source;
AUTO, raw EXPRESSION, and TEMPLATE remain distinct even when their results agree.
The separate
`canonical_json_v1(&json)` preserves the existing raw-JSON v1 bytes used by
durable identities. It does not interpret expression syntax or redact data;
callers must exclude secrets. Tree content IDs reject secrets unless the caller
explicitly requests a keyed commitment. Historical record, union, and unknown
schema encodings retain `SCHEMA_WIRE_VERSION = 1` and their existing bytes.
Scalar roots add a separately versioned descriptor under `kind: "scalar"`;
persisted plans require an explicitly supporting compiler/schema envelope.
Old empty record snapshots remain records, never implicitly become `null`.

## Contract

- **L1-3.5:** schema owns the typed-configuration contract, not integration runtime
  state or persistence authority.
- **L1-4.5:** a tree phase alone is not validation proof. `ValidValues` and
  `ResolvedValues` have private custody and cannot be minted by deserialization
  or a runtime flag.
- Expressions require permission at their exact declared field. Required
  expressions reject literal input with `expression.required`; opaque or
  undeclared descendants do not inherit expression permission. A prohibition on
  an ancestor cannot be reopened by a child declaration.
- Rules and conditional policies cross into `nebula-validator` through
  `validate_rules_with_ctx` and `resolve_field_policies`. Validator-native rule
  codes are preserved; data paths use RFC6901.
- Field and root checks share a prepared predicate context. Schema secrets,
  explicit secret nodes, and expression sources are unavailable there; pending
  expression paths are supplied separately. Containers remain addressable and
  predicate arrays remain opaque leaves.
- Built-in value rules inspect actual protected values through a private,
  temporary zeroizing projection; redaction markers never satisfy a value rule.
  Predicate context remains scrubbed, and custom deferred evaluators cannot
  receive protected input. Secret-bearing declarations protect aggregate rules
  even for malformed, absent, or inactive values. Errors and source chains remain
  redacted.
- Schema-bound loader dispatch creates bounded, alias-aware redacted snapshots,
  including nested secrets, wrong secret-bearing container shapes, and unknown
  mode payloads. Raw `LoaderContext` construction alone is not schema redaction.
- Declared secrets are protected during preparation, before a `ValidValues`
  escapes. Secret and expression diagnostics redact payloads, including retained
  parser, evaluator, regex, and typed-decoder error chains.
- `ResolvedValues::into_typed<T>()` refuses secret-bearing trees.
  `into_typed_exposing_secrets<T>()` is an explicit trusted disclosure boundary;
  it borrows protected text directly during deserialization instead of constructing
  a plaintext `serde_json::Value::String`. Its sensitive projection recursively
  applies schema output aliases and mode payload schemas before root union tagging.
  A derived `#[field(secret)]` property must use a leaf type implementing the
  explicit `SecretInput: DeserializeOwned + ZeroizeOnDrop` contract; `Option<T>` is
  supported when `T: SecretInput`, while bare `String` is rejected. `get_secret`,
  `expose`, and `SecretWire` are likewise explicit access, not ordinary JSON
  serialization.

## JSON Schema export

With the optional `schemars` feature, `ValidSchema::json_schema()` returns
`Result<schemars::Schema, JsonSchemaExportError>` for Draft 2020-12. Shape and
rules use standard keywords; `x-nebula-*` extensions carry expression policies,
requiredness, visibility, root rules, and UI/runtime hints.
Exported JSON Schema does not replace the proof pipeline.

## Non-goals

- Not a validation rules engine: `nebula-validator` owns predicates and rules.
- Not an expression evaluator: `nebula-expression` owns compilation and execution.
- Not a credential store, encryption service, or KDF; cryptographic primitives
  belong to `nebula-crypto` and disclosure/persistence policy belongs to consumers.
- Not a UI form renderer: schema carries hints as data.

## Maturity

The internal API is `frontier` and may change incompatibly. Supported downstream
contracts are curated through `nebula-sdk`. See [AGENTS.md](AGENTS.md) for the
relevant checks when changing a crate contract.

## Related

- [Design](docs/DESIGN.md) and [changelog](CHANGELOG.md).
- [Workspace agent guide](../../AGENTS.md) and [integration model](../../docs/INTEGRATION_MODEL.md).
- [Validator](../validator/README.md) and [expression](../expression/README.md) ownership.
