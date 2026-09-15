# JSON Schema Export Extensions

`ValidSchema::json_schema()` projects the admitted schema into Draft 2020-12
plus the extensions below. It is a structural input projection, not a lossless
encoding of the schema definition, authored-value wire, or runtime proof.
Generic JSON Schema success does not replace `ValidSchema::validate` and full
resolution, secret handling, loader admission, or host-owned file and access checks.
The exporter rejects historical schema policies and unsupported field kinds;
preserving their definition wire does not make them exportable current contracts.

## Locations and Values

A **field schema** is the object emitted for a `Field`: a record/object property,
a declared list item, or a mode/union payload. Read-alias properties repeat the
same field schema. Nested field schemas also appear inside literal branches and
`x-nebula-resolved-value-schema` copies. The synthetic `$expr` wrapper's `$expr`
property, mode selector, and select option entries are not field schemas.
Record/scalar/union roots do not receive the common field extensions merely
because they are roots; a scalar root's rules have the separate annotation below.

| Extension | Exact emitted location and condition | JSON value | Current meaning and enforcement |
|---|---|---|---|
| `x-nebula-schema-version` | Every document root: record, scalar, Any, external union, and adjacent union. Not copied onto nested field schemas. | JSON integer `2`, from `SCHEMA_WIRE_VERSION`. | Identifies the definition/export writer contract. Nebula-aware consumers must reject missing, malformed, or unsupported versions before interpreting extensions. Generic validators do not enforce this marker; it is neither schema-policy admission nor runtime authority. |
| `x-nebula-field-kind` | Every field schema. | String: `"string"`, `"secret"`, `"number"`, `"boolean"`, `"select"`, `"object"`, `"list"`, `"mode"`, `"code"`, `"file"`, `"computed"`, `"dynamic"`, or `"notice"`. | Identifies the declared family. Unknown field kinds are rejected before export, including nested declarations and inactive variants. A recognized family name alone grants no admission authority. |
| `x-nebula-expression-mode` | Every field schema. | String: `"forbidden"`, `"allowed"`, or `"required"`. | Runtime expression policy at this declaration. Standard keywords describe a literal shape and/or `$expr` wrapper; they do not compile or authorize a program. |
| `x-nebula-resolved-value-schema` | Every field schema, including expression-forbidden fields. | JSON Schema object; may be `{}` or contain nested field extensions. | The projected literal shape before the outer expression wrapper and common annotations. This is not proof that resolution or all runtime constraints succeed. |
| `x-nebula-required-mode` | Every field schema. | String: `"never"`, `"always"`, or `"when"`. | Requiredness mode. `"when"` omits the predicate. Current runtime also rejects required null/empty values; static presence constraints no longer depend on visibility under policy v2. |
| `x-nebula-visibility-mode` | Every field schema. | String: `"always"`, `"never"`, or `"when"`. | Visibility mode; `"when"` omits the predicate. Policy v2 treats visibility as display metadata. Historical legacy behavior could suppress missing-required errors; this annotation alone does not identify that policy boundary. |
| `x-nebula-root-rules` | Record or scalar root, only when root rules are nonempty. Not emitted on union roots. | Array of serialized `Rule` objects, in declaration order. | Retains root-rule descriptions. Scalar basic rules can also become standard constraints; contextual/custom obligations remain runtime-owned. Generic validators do not execute this array. |
| `x-nebula-read-aliases` | Field schema with nonempty read aliases, including its alias-property copies. | Nonempty array of strings, in admitted alias order. | Inbound aliases also become typed properties. Required aliases use `allOf`/`anyOf` presence clauses. Generic validation does not consume aliases or apply canonical-key precedence. |
| `x-nebula-emit-as` | Field schema with an output alias. | String. | Output projection key for `to_wire_json`; it does not rename input properties or make that name an input alias. |
| `x-nebula-file-accept` | File field schema with `accept: Some(...)`. | String, preserved verbatim. | File-picker/reference hint. Runtime validates a string reference or string-reference list; it does not read files, verify MIME, or check this pattern against content. |
| `x-nebula-file-max-size` | File field schema with `max_size: Some(...)`, including zero. | Nonnegative JSON integer from `u64`, intended byte unit. | Requested file-size hint, not an enforced byte limit. Reference-string length is not file size; neither this extension nor current schema validation checks referenced bytes. |
| `x-nebula-select-dynamic` | Every select field schema, including when false. | Boolean. | Records dynamic-option intent. It does not perform loader calls or certify that a loader is available. |
| `x-nebula-select-multiple` | Every select field schema, including when false. | Boolean. | Mirrors single-value versus array selection. Standard shape keywords and runtime enforce the declared container shape. |
| `x-nebula-select-allow-custom` | Every select field schema, including when false. | Boolean. | True permits values outside static options. The export omits option membership constraints for custom values and empty dynamic option sets; the extension itself performs no validation. |
| `x-nebula-disabled` | Disabled option's `anyOf` entry in a single select, or `items.anyOf` entry in a multiple select; only when static options are projected. Also repeated in resolved-schema copies. | Literal `true`; absent for enabled options. | Presentation hint. The option's `const` remains in the allowed domain, and runtime membership checks do not exclude disabled options. Custom-value selects do not project these option entries. |
| `x-nebula-mode-default-variant` | Mode field schema with a default variant; not the root of a serde-tagged union. | String containing the declared variant key. | Runtime can choose this variant when the selector is omitted. The corresponding standard `oneOf` branch permits an omitted `mode`; generic validation does not insert the selector. |

## Current Limits

Export has two resource budgets: **1 MiB of compact JSON for an unelided,
borrowed source descriptor**, and **8 MiB cumulatively for compact-JSON inputs
to expansion copies**. The source measurement includes every field slot, even
defaults omitted by the definition writer, so it conservatively bounds source
wire size without invoking the cloning `Field` wire serializer. Defaults,
options, rules, aliases, and display/loader metadata participate. Counting uses
a `Write` sink without materializing JSON. Arbitrary JSON in defaults/options
must also fit the existing 64-level value-depth limit to be measured safely.
Source-byte or copy-byte exhaustion returns payload-free
`SourceBudgetExceeded` or `CopyBudgetExceeded`; failed source measurement,
including excessive JSON depth, returns payload-free `BudgetSerialization`. These failures emit
fixed tracing codes and never retain source values in the error chain.

The copy budget is charged **before** deep-cloning expression cores, alias
properties, or repeated adjacent-union tag/content names. Declaration depth
alone is insufficient: expression-forbidden/allowed nesting duplicates resolved
schemas at each level, and aliases multiply those copies. These are source and
cumulative-copy measurements, **not an exact final-output byte cap, heap limit,
or CPU deadline**. Original input allocations and caller serialization/validation
of a successful export remain outside this guard. Successful projection shapes
and their writer version are unchanged; resource rejection grants no validation
authority and invokes no loaders, expression engines, or other runtime callbacks.

Labels and descriptions become `title` and `description`, never replacement
validation keywords. Placeholder, group, hint, and widget data are not emitted by
this exporter. A password widget is not secret protection. The standard `default`
annotation is copied from field metadata; neither generic JSON Schema validation
nor the current field-default hint materializes a missing value.

Fresh definitions carry `policy_version: 2`: `RequiredMode::Always` produces
unconditional presence constraints even with `VisibilityMode::Never`. Supplied
required values also carry non-null/nonempty constraints where applicable.
Conditional requiredness remains runtime-owned, and pending policies can delay
proof. Historical legacy visibility could suppress requiredness for an absent
key. Historical definitions retain their wire representation but cannot validate
current values or export a current contract; migration requires fresh admission.
Do not generalize policy-v2 display separation to legacy readers or stored exports.

Basic value rules are projected where supported. Conditional predicate bodies,
transformers, and arbitrary field-rule semantics are not completely represented.
Open record/object properties and permissive dynamic shapes also mean
this document cannot be used to infer a closed runtime contract. File annotations
provide no content, storage, scanning, tenant, or capability evidence.

Unknown extension keywords do not make a generic validator fail closed.
[Draft 2020-12 section 6.5](https://json-schema.org/draft/2020-12/json-schema-core#section-6.5)
treats unrecognized keywords as annotations. A consumer requiring Nebula semantics
must explicitly check its supported extension set and contract versions before
using the export. A required vocabulary would need a corresponding meta-schema
referenced through `$schema`; merely adding a keyword is insufficient.
The root version marker makes the projection self-identifying, not trusted.
File, display, and option hints retain the limitations above even when a consumer
recognizes that version.

## Versioning and Evidence

- Any addition, removal, relocation, JSON value-type change, or semantic change to
  an export extension requires a `SCHEMA_WIRE_VERSION` bump and corresponding
  registry/export evidence in the same review. The root marker uses that shared
  definition/export writer version, not an independently advancing export counter.
- Changes to persisted schema-definition wire shape also bump
  `SCHEMA_WIRE_VERSION`. Changed admission semantics require a policy-version
  bump even if the shape stays identical. Change both when both contracts change;
  never reinterpret historical bytes in place. Schema policy, graph-document v3,
  plugin plan-envelope v3, authored-value wire, and canonical encodings are
  separate contracts, not interchangeable version numbers.
- Nebula-aware consumers must check the integer root version and supported
  semantic requirements explicitly. Unversioned earlier exports cannot silently
  acquire the current contract. An unchanged `$schema` draft URI is not Nebula
  compatibility evidence, and a recognized marker does not replace fresh admission.
  Consumers persisting exports still own their containing envelope and trust policy.
- `tests/evolution_wire_snapshot.rs`, with `schemars`, freezes full exported
  values and locations in `json_schema_literal_extensions` and
  `json_schema_expression_modes`. The literal fixture covers every registered
  extension; the expression fixture covers all three expression modes. Existing
  schema-definition wire snapshots remain independent.
- `tests/json_schema_extension_contract.rs` checks display keyword isolation,
  opaque file-reference behavior, the root version on every root shape, and the
  difference between generic validation and Nebula obligations. Hint and authority
  checks are coverage of current behavior, not fixes for the documented limitations.
- `tests/json_schema_export_budget.rs` checks depth and alias amplification,
  repeated union names, escaped metadata, and useful deep projections. The private
  budget tests compare counting with `serde_json`, cover exact boundaries, and
  verify that exhausted copy budgets do not invoke `Clone`.

Run from the workspace root:

```sh
cargo nextest run -p nebula-schema --features schemars --test evolution_wire_snapshot --test json_schema_extension_contract --test json_schema_smoke
```
