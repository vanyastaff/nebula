# nebula-schema design

| Property | Contract |
|----------|----------|
| Status | Frontier internal API; incompatible changes are possible |
| Layer | Core, subject to the workspace dependency map |
| Owns | Schema definitions, canonical value trees, preparation, proof custody, schema-aware projections |
| Delegates | Rules and conditional policies to `nebula-validator`; compilation and evaluation to `nebula-expression` |
| Product boundary | `nebula-sdk` is the sole curated, supported Rust surface; this crate is an internal technical boundary |

## Ownership

`nebula-schema` supplies the typed-configuration model shared by Actions,
Credentials, and Resources (canon L1-3.5). It owns `Schema`, field declarations,
structural lint, phase-indexed values, schema-bound proofs, secret wrappers,
option/record loader interfaces, and optional JSON Schema export.

It does not own a rules engine, expression execution semantics, credential
lifecycle, persistence authority, resource binding slots, or UI rendering.
Cryptographic primitives belong to `nebula-crypto`; KDF/password hashing must
not be added here. Consumers decide when validated data is persisted and which
trusted boundary may expose protected material. This design does not claim
that any particular consumer has adopted the complete pipeline.

## One tree, separate proofs

`ValidSchema` owns one `RootShape`. `Any` deliberately provides no shape proof;
`Scalar` carries a checked null/boolean/string/integer/number domain; `Record`
owns declarations and root rules; `Union` owns a required mode and its serde
tagging. Field indexes are derived accelerators, not another root description.
`SchemaKind`, fields, and tagging cannot disagree inside an admitted schema.

`()` and unit structs use the null domain. Empty braced structs use an empty
record, which still requires an object. Primitive `HasSchema` implementations
declare their known types and exact numeric bounds rather than advertising
`Any`. Preparation preserves lossless integral-number normalization, and final
validation rechecks the scalar domain and rules. Scalar roots do not acquire
expression permission through a synthetic declaration.

`ValueTree<E>` has exactly five variants:

- `Literal(ScalarValue)` for null, boolean, number, or string data.
- `Object(IndexMap<String, Self>)` for arbitrary JSON property names.
- `List(Vec<Self>)` for ordered data.
- `Expression(E)` for the expression capability of the current phase.
- `Secret(SecretValue)` for explicitly protected material.

`ScalarValue::try_from(serde_json::Value)` rejects objects and arrays. Containers
cannot be hidden in a literal node. A mode envelope is an ordinary object with
`mode` and optional `value` properties; its interpretation comes from the
selected `ModeField` declaration.

| Tree alias | Expression parameter | Meaning |
|------------|----------------------|---------|
| `AuthoredValue` | `Expression` | Authored source, not yet admitted by a schema |
| `CompiledValue` | `CompiledProgram` | Retained immutable programs |
| `ResolvedValue` | `Infallible` | No constructible expression node |

These aliases control representation, not proof. A caller can construct a
data-only tree without validating it. Only `ValidValues` and `ResolvedValues`
certify the appropriate checks against an immutable `ValidSchema` snapshot;
neither proof has a public constructor or a deserialization bypass (L1-4.5).

## Checked transitions

1. `SchemaBuilder::build()` runs structural lint and returns
   `Result<ValidSchema, ValidationReport>`. `HasSchema::schema()` and
   `schema_of::<T>()` return the same checked result type. Derived schemas cache
   success or a construction report. Type-level schema discovery is pure and
   does not depend on runtime values.
2. `ValidSchema::validate(AuthoredValue)` consumes the authored tree. It checks
   depth, folds read aliases at each declared scope, applies field transforms,
   promotes declared string secrets, and compiles admitted expressions. The
   resulting `ValidValues` retains `CompiledValue`, pending value/rule/policy
   obligations, warnings, and the schema snapshot.
3. `ValidValues::resolve(self, &dyn ExpressionContext).await` evaluates retained
   programs. `ExpressionContext::evaluate` takes `&CompiledProgram` and returns
   `EvalFuture`, an object-safe boxed future. `EngineExpressionContext` calls
   the expression engine's compiled-program API, not a source parser.
4. Returned JSON is decoded with literal-only ingestion and prepared at the
   expression's declared location. Final full-mode structural, rule, and
   conditional-policy checks must succeed without pending obligations before
   `ResolvedValues` is constructed.

The synchronous alternative, `ValidValues::resolve_data(self)`, never creates
or invokes an engine. It rejects compiled expressions with
`expression.forbidden` and still performs full final validation. Data-only
configuration does not need a dummy evaluator or a validation-skipping flag.

### Preparation and admission

Canonical input wins over read aliases; otherwise the first declared alias
wins. Every alias is consumed, including losing aliases that may carry secrets.
Transforms run once for each newly prepared scalar or secret. Existing literal
siblings are not transformed again during resolution; newly evaluated subtrees
receive their own preparation once. Field transformer metadata remains intact.

Expression permission belongs to the exact declaration, not its ancestors.
Opaque or undeclared descendants cannot acquire code capability merely because
their parent permits expressions. A parent's `Forbidden` restriction does apply
to its whole subtree; a child cannot reopen that restriction. `ExpressionMode::Required` rejects authored
literals with `expression.required`. Template-like strings or `$expr` objects
returned by the evaluator remain data and cannot trigger another evaluation.

`Transformer::regex(pattern, group)` and `RegexCapture::new(pattern, group)`
return `Result<_, ValidationError>`. The capture-specific configuration owns a
compiled `regex::Regex` and a checked group index; the validator's `RulePattern`
does not expose capture extraction. Invalid patterns and nonexistent groups fail
at construction or serde, with `transformer.invalid_pattern` or
`transformer.invalid_capture_group`. There is no lazy failed-compilation cache,
warning containing a pattern, or invalid-configuration no-op. Valid no-match and
unmatched optional groups retain the original string; non-strings pass through.

## Data paths and schema paths

`FieldKey` is a checked schema identifier. `FieldPath` and `PathSegment` address
declarations and indexed schema locations using forms such as `items[0].name`.
They remain appropriate for schema lookup, not arbitrary JSON traversal.

`ValuePath` is the RFC6901 data path, re-exported from the validator foundation.
Data errors, pending obligations, and tree lookup use this type. The root is
`""`; `/` denotes an empty property name; `~0` and `~1` escape `~` and `/`.
`ValuePath::parse` returns `Option<ValuePath>`, and `push` accepts an exact data
segment. A numeric segment selects a list index only at a list; numeric object
keys remain keys. Lists require canonical decimal indices without leading zeros.

`get(key)` is exact-key lookup, while `get_path(&ValuePath)` traverses containers.
`insert(key, tree)` returns `Result<Option<Self>, ValidationError>` and rejects
non-object receivers. Key insertion grants neither schema admission nor proof.
Data property names need not satisfy `FieldKey` syntax.

## Wire, views, and identities

There are independent contracts, not interchangeable serialization helpers:

| Boundary | Representation and policy |
|----------|---------------------------|
| Literal ingestion | `ValueTree::from_data(json)` interprets no code syntax |
| Explicit authoring shorthand | `AuthoredValue::from_template_json(json)` recognizes template strings and exact `$expr` objects as AUTO programs |
| Tree serde | Authored v2 `{version, data, expressions}` envelope |
| JSON view | `to_json` redacts secret leaves but preserves authored/compiled expression sources |
| Schema projection | `project`/`to_wire_json` apply output aliases and omit secrets; not proof or authored persistence |
| Tree canonical encoding | Version 2 content-addressing; expression and literal identities stay distinct |
| Durable raw JSON encoding | `canonical_json_v1` retains the existing JSON-v1 byte contract |
| Schema-definition serde | Historical record/union/unknown v1 bytes; scalar roots have a separately versioned descriptor |

Authored v2 has this shape:

```json
{
  "version": 2,
  "data": {"result": null, "literal": "{{ not code }}"},
  "expressions": [{"path": "/result", "syntax": "auto", "source": "{{ $input.result }}"}]
}
```

The expression table is separate from ordinary data. Every entry is exactly
`{path, syntax, source}` and targets an existing null placeholder, including the root
when its pointer is empty. The decoder rejects unsupported versions, unknown,
missing, or duplicate envelope/entry fields, duplicate data keys, invalid,
duplicate, or overlapping paths, and missing or non-null placeholders. Entry
order does not change decoding; serialization follows tree traversal order. Syntax
is required and closed: `auto`, `expression` (raw), or `template` (always string).
`Expression::new` and `from_template_json` keep AUTO semantics; use
`Expression::template` for explicit text interpolation. `with_syntax` creates a
fresh immutable source/syntax pair with its own shared lazy compilation cache.

All three tree phases serialize through this format, rejecting any explicit
secret before writing envelope content. Only the authored phase deserializes;
it restores source and syntax without compilation or proof construction. Compiled
programs retain requested syntax, even when AUTO chooses a template body. The logical
depth limit is 64, including empty containers, with the root at depth zero.
The envelope does not double the nesting of the data or disable parser limits.

`VALUE_CANON_VERSION = 2` governs tree canonical bytes, not serde or schema
definitions. Secret-free tree content IDs are insertion-order independent;
keyed secret commitments require an explicit `CommitmentKey` and produce
`CommitmentId`, not a portable plaintext content ID.
Expression identity is exact source plus authored syntax, not parsed AST equality.
An expression encodes as tag `0x08`, then syntax tag `0` (AUTO), `1` (raw
EXPRESSION), or `2` (TEMPLATE), then length-prefixed source bytes. The same framing
applies to keyed commitments. Pure-data tree encoding is unchanged.

`canonical_json_v1(&json)` retains the persisted v1 domain/version, JSON
container tags, UTF-8 key ordering, depth bound, and numeric normalization.
Changing tree representation must not change these durable bytes. This encoder
does not infer secret fields, redact values, or interpret authoring syntax;
its caller must exclude secret material.

## Secrets and diagnostics

Declared string secrets are promoted during consuming preparation, before a
`ValidValues` can escape. `SecretString` and `SecretBytes` zeroize owned storage;
ordinary `Debug`, `Display`, and JSON serialization redact protected material.
`Expression` and `CompiledProgram` diagnostic output does not print source.
Authored serialization and explicit source access are not diagnostic surfaces.

Field and root rules share `prepared_predicate_context` after preparation.
Schema-declared secrets and explicit `Secret` nodes are scrubbed recursively,
including under arbitrary keys. Expressions are unavailable; pending paths are
supplied separately through the validator context. Safe whole containers remain
addressable. Arrays remain opaque predicate leaves, with unavailable elements
represented by null to preserve positions; this does not add indexed predicate
lookup.

Value rules use a private, temporary zeroizing projection of actual data, not
redaction markers. An aggregate is protected when its declaration contains a
secret or its data contains an explicit secret node. This also covers malformed
values, missing fields, and inactive mode variants: successful promotion is not
a prerequisite for protected error causes. The entire rule tree is audited
before disclosure; full-mode custom evaluators cannot receive protected input.
Public sibling-field rules retain their ordinary diagnostics.

Raw context wrappers check depth before traversal or copying and fold all read
aliases. Wrong secret-bearing container shapes and unknown mode payloads cannot
leak through projection. Schema-bound loader dispatch applies the same schema
awareness but creates redacted literal snapshots, including removal of expression
source content. A raw `LoaderContext` is not safe merely because it has the same
type; `with_secrets_redacted(&schema)` is fallible and establishes the snapshot
boundary. Direct loader calls can scrub explicit nodes but cannot infer schema
secrets without declarations.

`ResolvedValues::into_typed<T>()` rejects secret-bearing trees instead of decoding
redaction markers into plausible credentials. The separately named
`into_typed_exposing_secrets<T>()` consumes proof at a trusted disclosure boundary,
preserves union wire tagging, and drives serde from a sensitive tree that borrows
protected text rather than constructing an ordinary plaintext JSON string. The
tree recursively applies field output aliases and active mode payload schemas before
the root union's serde tagging. Its decode error erases visitor diagnostics before
they can enter a public error chain.
For derived schemas, every `#[field(secret)]` leaf (including the `T` in `Option<T>`)
must explicitly implement `SecretInput`, whose supertraits require owned
deserialization and zeroization on drop. `get_secret`, `expose`, and `SecretWire`
are explicit lower-level disclosure paths. The `audit-secret-expose` feature changes
exposure audit verbosity; it is not what makes access possible.

`ValidationError` stores its payload behind a private box and exposes `code()`,
`path()`, `severity()`, `params()`, and `message()`. Data paths are RFC6901.
Parser, evaluator, regex, and typed-decoding boundaries attach private typed
causes whose public `Error::source` chain is redacted. Never interpolate raw
source, input data, or secret-bearing upstream errors into messages or params.
Rule codes remain validator-native, without namespace remapping. Rule/policy
execution stays centralized at `validate_rules_with_ctx` and
`resolve_field_policies`; schema does not duplicate their semantics.

## Module map and checks

- `schema.rs`, `field.rs`, `builder/`, and `lint.rs`: definitions, construction,
  checked keys, aliases, and bounded structural lint.
- `value/mod.rs`, `tree.rs`, `wire.rs`, `tree_canonical.rs`, and `canonical.rs`
  under `value/`: phase-indexed representation, authored serde, tree identity,
  and the independent durable JSON-v1 encoding.
- `validated/mod.rs` and its `preparation.rs`, `validation.rs`, and `values.rs`:
  schema snapshots, consuming preparation, validator integration, and proof custody.
- `expression.rs`, `context.rs`, `loader.rs`, `secret.rs`, and `transformer.rs`:
  evaluation adapters, safe projections, loader boundaries, and checked primitives.
- `has_schema.rs` and `macros/`: checked schema discovery and derives.
- `json_schema.rs`: optional Draft 2020-12 export with `x-nebula-*` extensions;
  exported metadata does not replace validation or runtime proof.

The [agent guide](../AGENTS.md) maps changed contracts to focused tests and
commands. [README](../README.md) summarizes the API; [CHANGELOG](../CHANGELOG.md)
records breaking migrations. Workspace ownership and layering remain governed
by [the root guide](../../../AGENTS.md) and
[the integration model](../../../docs/INTEGRATION_MODEL.md).
