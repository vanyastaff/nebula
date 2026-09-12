---
name: Schema, metadata and slot authoring
status: proposed
implementation: not shipped
last-reviewed: 2026-09-12
related:
  - ../../../docs/INTEGRATION_MODEL.md
  - DESIGN.md
  - ../../action/docs/DESIGN.md
  - ../../credential/docs/DESIGN.md
  - ../../resource/docs/DESIGN.md
  - ../../metadata/README.md
  - ../../plugin/docs/DESIGN.md
---

# Schema, metadata and slot authoring

## Status and Decision

This is the proposed implementation contract for revising design-only PR1027.
The user authorizes a breaking architectural redesign, superseding issue 992's
earlier ratify-only restriction. This document does not ship any implementation.
The private design vault was unavailable: ADR text is unverified and unmodified.
Private ADR-0108 remains unverified; only its preceding summary was available.

Use structured `#[property(display(...), input(...), validate(...), options(...))]`
on data fields and a separate `#[slot(...)]` on dependency fields. Permanently
support associated data types as the canonical authoring model. Do not force
`Input = Self`, `Properties = Self`, or `Config = Self`.

| Earlier summarized decision | Proposed replacement |
|---|---|
| One author struct per abstraction; external types migration-only | Separate receiver and associated data types are first-class and canonical. |
| One flat property grammar with credential/resource modes | Structured value properties plus a distinct slot grammar. |
| Derive Credential and infer capabilities from other methods | Preserve the existing impl-level `#[credential]` macro. |
| Resource derive supplies Config while Provider stays handwritten | Handwritten Provider owns its complete impl, including Config. |
| Preserve current FromWorkflowNode unchanged | Preserve the entrypoint, extend its owning port with prepared-input evidence. |
| No slot-only schema wire bump | Still true, but new value descriptors and policy semantics require versioning. |

A shared vocabulary is the ergonomic goal; identical struct shape is not.
The current action factory defaults non-slot receiver fields, while dispatch
decodes a separate input. Making that input `Self` duplicates values and leaves
different slots populated on the two instances. Provider similarly receives
`&self` and `&Self::Config`. Removing Clone cannot repair this ownership split.
An explicitly handwritten `Properties = Self` remains possible when meaningful;
there is no generated single-instance mode or new action behavior family.

Use schema-owned `#[schema_type(input)]`, `#[schema_type(output)]` or
`#[schema_type(input, output)]` to own codec generation on the same data definition.
The alternative of treating all independently authored codecs as trusted by default
is rejected: normal DTO authoring can provide stronger structural guarantees and
simpler derive ownership. Property sections retain their separate semantics;
receiver rewriting and hidden companion data types remain outside this contract.

## Existing Owners and Reuse

| Owner | Reuse and required responsibility |
|---|---|
| schema | HasSchema, PropertyType, schema_type and directional codec contracts, Field, RootShape, checked ValidSchema, value/proof pipeline, serde projections, LoaderRegistry and redacted loader context. |
| validator | Rule, Predicate, FieldPath, budgets, pending evaluation; checked Condition refinement and policy semantic v2. |
| core | Dependencies, SlotField, SlotKind and typed keys; remains condition-free. |
| action | Action traits, FromWorkflowNode, input preparation, factories, leaf slot declarations and invocation decisions. |
| resource | Provider, ResourceConfig, resource leases, SlotCell generations, slot rotation and resource leaf declarations. |
| credential | Credential, Scheme/State separation, projected CredentialGuard, resolution/refresh authority and impl capability inference. |
| plugin | Frozen catalog and pure compiler; versioned recorded slot requirements, dependency-closure checks and exact plan readmission. |
| metadata and leaf drafts | MetadataName, MetadataVersion, Icon, BaseMetadata and three concrete MetadataDraft types; checked admission remains with leaf factories/registry. |
| sdk | Curated author exports and hygienic macro paths, with no exposed admission or tenant authority. |
| schema-codegen | New compiler-only normal library nebula-schema-codegen, owned by schema at Core, sharing syntax/models/diagnostics across four companion macro consumers. |

No new canonical value crate, generic IntegrationDraft, extension trait for
metadata setters, condition evaluator, or engine-owned Slots object is proposed.
A Condition is a checked subset of Rule, not another rule representation.
A small field-descriptor trait is justified below where HasSchema is insufficient.
Runtime proof objects are never author property types or serde-constructible.
The new normal library lives at `crates/schema/codegen`; proc-macro crates cannot
export arbitrary parser types for other proc-macro crates to import. It owns the
property/slot syntax model, not runtime slot policy. Dependencies are compiler
tools such as syn/quote and narrow existing macro support, never runtime authority.
This extraction replaces real parser duplication across Schema, Action,
Resource and credential authoring consumers. Each invocation parses independently
using the shared implementation: no global cache, inter-derive state dependency, or
claim of one parsed instance shared between derives. Update workspace metadata
and deny wrappers for this permitted dependency direction; metadata-driven CI
discovers the new package without handwritten package-selection lists.

## Macro Ownership

Each Rust trait implementation has exactly one owner. A derive cannot inspect
unrelated impl blocks, append fields, modify serde derives, or discover arbitrary
trait implementations. Shared parsing means shared code, not shared macro state.

| Existing family | Keep or change |
|---|---|
| schema_type attribute | New data-definition owner; insert real library Serde derives for requested directions plus Schema, and emit directional codec implementations. |
| Schema derive | Keep schema-only HasSchema, PropertyType and checked declarations; emit neither Serde implementations nor directional codec evidence. |
| EnumSelect derive | Keep for option labels; consume the same serde enum-domain declaration as Schema, without an independent casing/domain algorithm. |
| Action derive | Keep; emit the complete Action impl and FromWorkflowNode adapter, using explicit input/output types and separate slots. |
| action_phantom attribute | Remains a separate existing adapter; does not turn CredentialRef into a new slot shape or participate in property inference. |
| credential impl attribute | Keep; own the complete Credential/capability impls from the annotated impl's recognized items. No new Credential struct derive. |
| AuthScheme/capability macros | Keep their existing scheme and capability responsibilities. |
| Resource derive | Keep slot plumbing and existing factory contribution responsibilities; never emit a partial Provider impl. |
| ResourceConfig derive | Keep config identity/validation; require HasSchema and both codec directions, normally supplied by schema_type(input, output), for every shape. |
| ClassifyError | Keep; typed resource errors remain separate from authoring grammar. |
| Plugin and Validator derives | Keep their own responsibilities; Schema stops consuming legacy validate helpers, but Validator's independent grammar is not removed. |

ResourceConfig no longer opportunistically emits HasSchema for unit/empty types;
use schema_type there too. Remove the need for `config(schema = external)`.
Retain `config(validate = RustPath)` and `config(skip_fingerprint)` as config
identity hooks, not property/serde omission flags. Identity exclusions must not
affect validation, persistence, slot decisions, or security policy.
This proposal does not remove ResourceConfig's current Clone bound merely to
accommodate a slot-bearing config; canonical configs contain data, not guards.

### Directional Codec Ownership

The following schema-owned signatures are unshipped targets; Serde supplies the
actual codec implementations, never a Nebula reimplementation of Serde:

```rust
pub trait InputCodec: HasSchema + PropertyType + serde::de::DeserializeOwned {}
pub trait OutputCodec: HasSchema + PropertyType + serde::Serialize {}
```

`schema_type(input)` inserts the real Deserialize derive, `output` Serialize,
and `input, output` both; every mode inserts Schema on that same definition.
Only this owner generates the corresponding structural codec trait impls;
Schema alone remains valid for description, including serde helpers, but produces
no codec evidence even beside separately authored Serde derives. Evidence rests
on joint generation from the same supported declaration and recursive bounds,
not marker presence alone. Neither a helper flag nor a blanket implementation
for HasSchema plus Serde may manufacture that structural relationship.

Action::Input and Credential::Properties require InputCodec; Action::Output
requires OutputCodec, including outgoing trigger payloads. PollAction has no
independent Event associated type in the target; poll results and pushed-event
outcomes carry Self::Output under that same HasSchema/OutputCodec contract.
Provider::Config deliberately requires both directions as a target round-trip
authoring contract for config decoding and outbound encoding
through paired projections. This is a new requirement: current ResourceConfig
does not require Serialize, and fingerprint() -> u64 is an independent contract.
Keep existing Send/Sync/Clone bounds where applicable; input-only DTOs gain no
unconditional Serialize requirement.
Every field, enum payload and transparent newtype requires its child's respective
codec direction, including external types and all optional/inactive variants.
Provide reviewed schema-owned primitive/unit codec impls and Vec<T>/Option<T>
impls conditional on T's direction; aliases reuse the actual type's evidence.
Generic owners propagate bounds on used field types and preserve where clauses.
SecretInput is an additional input destination contract, not codec evidence.

The owner precedes all derives and schema/serde helpers; `schema(...)` remains
an inert helper, never the owner's name. Permit whole-item cfg gating only;
initially reject field/variant cfg and cfg_attr composition. Other transforming
attributes on the declaration are unsupported in either order; reject those
visible to the owner without claiming to detect already-expanded attributes.
Accept only Serde forms specified here; reject other attributes and empty/duplicate
directions at their spans. Other non-transforming derives may coexist.
Do not inspect unrelated impls: rustc coherence rejects a conflicting manual
Serde or Schema impl, including a separately requested duplicate derive.
Reuse nebula_macro_support's final path resolution and curated SDK exports for
the owner, Schema and library Serde derives; generate the matching serde(crate)
path through narrow macro support, reject author overrides, and test renamed SDK
consumers without direct leaf/Serde dependencies. No expansion-state side channel.

Manual/foreign codecs require an explicit, reviewed local adapter newtype with
matching HasSchema/PropertyType, directional Serde implementations and explicit
InputCodec/OutputCodec impls for the directions it supports. These adapter impls
assert a reviewed contract, not generated structural provenance; requirements
propagate through parents, with no implicit promotion of the underlying type.
Custom semantic preservation is trusted and cannot be proved by descriptor
validation or round trips; validate actual boundary values as well. Native code,
framework leaf codecs and macro implementations remain trusted (canon 12.6).
Downstream expansion has no private-item privilege: SDK __private exports are
callable support, not a security boundary or unforgeable witness constructor.
Neither codec trait grants admission authority or replaces runtime proof custody.

## Property Grammar

The following key sets are exhaustive for the new property grammar.
Rust literals mean string, bool, or finite numeric literals; negative numbers
are signed expressions parsed deliberately, not misclassified as unsigned Lit.
Paths are Rust paths resolved by rustc, never strings evaluated as code.

```text
property := #[property(section, ...)]
section  := display(...) | input(...) | validate(...) | options(...)
reference := field(identifier) | root("/absolute/json/pointer")
```

Sections may appear in any order, at most once per field, across all property
attributes on that field. Duplicate singleton keys, conflicting modes, unknown
keys and empty argument lists where a value is required are errors at their spans.
Property and slot on the same field are incompatible.
Field attributes are optional; the Rust data domain supplies the baseline.

| Section | Accepted keys and argument shapes |
|---|---|
| display | `label = "..."`, `description = "..."`, `placeholder = "..."`, `hint = "..."`, `group = "..."`, `example = literal`, `widget = token`, `hidden`, `visible_when(C)`. |
| input | `required`, `required_when(C)`, `expressions = allowed\|forbidden\|required`, `secret`. |
| validate | `non_empty`, `length(min = n, max = n)`, `range(min = number, max = number)`, `pattern = "..."`, `url`, `email`. Either bound may be omitted, but not both. |
| options | `source = RustPath`, `depends_on(reference, ...)`, `mode = closed\|suggestions`. Source is required; mode defaults to suggestions. |

Widgets are `auto`, `text`, `textarea`, `password`, `number`, `checkbox`,
`select`, `radio`, `object`, or `list`.
A widget must fit the existing field domain; it never changes type, enum
membership, expression permission or secrecy. Password rendering is not secret
protection. `hint` is short presentation text; semantic url/email checks belong
in validate. Existing InputHint values may back a widget, not redefine this key.
Enum variants accept only `property(display(label = "...", description = "..."))`.
Other property sections on variants or fields of unsupported shapes are rejected.

`hidden` conflicts with `visible_when`; `required` conflicts with `required_when`.
There is no flat property key/default/emit_as/skip/skip_fingerprint, no
`validate(required)`, and no expression flags outside input.
`expressions = allowed` is the baseline for data fields; credential property
admission narrows it to forbidden and rejects explicit allowed/required intent.
An ancestor prohibition applies to its entire subtree. Required expressions
cannot be satisfied by a literal default. Data ingestion never infers expressions.
`input(secret)` explicitly requires SecretInput on the decoded non-null leaf;
never infer secrets from names or widgets. Schema-only and credential data may
contain protected leaves. Resource admission rejects them recursively, including
external child schemas; this is not an arbitrary compile-time proof of secrecy.
ResourceConfig derive also rejects a directly annotated input(secret) field at
its attribute span. That early diagnostic complements, never replaces, recursive
admission checks on associated configs and external child schemas.

Rules are built through validator's typed Rule constructors. Non-empty is a
value rule, not presence. Length applies to supported strings/collections,
range to numbers, and pattern/url/email to strings or declared secret strings.
Unsupported domain/rule combinations are errors, never ignored decorators.
Explicit null allowed by Option skips non-null value rules; non_empty does not
make a nullable field non-null. Choose a non-null Rust domain to forbid null.

## Serde Is the Wire Authority

`serde(rename)`, `rename_all`, directional rename/rename_all and `alias`
determine wire identity. Field rename overrides the respective container rule;
without either, serde's field/variant spelling applies, including raw identifiers.
Aliases are inbound only. Canonical inbound and canonical outbound keys are
recorded separately and collision-checked within each projection's scope.
Duplicate read aliases on one field are deduplicated; ambiguous cross-field
input keys or output keys are rejected. No property key override competes here.

Alias normalization consumes aliases once: canonical input wins, otherwise the
first declared alias wins. Normalize to the canonical INBOUND name before
conditions, validation and typed decoding. Outbound names apply only to export.
The existing implementation at `src/validated/typed.rs:92` projects emit_as
before `T::deserialize`; the implementation must split that projection path.
Deleting emit_as from the grammar alone does not fix directional serde names.

Required regression: `rename(deserialize = "input_name", serialize = "output_name")`
plus `alias = "legacy_name"` accepts either inbound spelling, canonicalizes to
input_name, and passes input_name to Deserialize; output export uses output_name.
Cover nested records, lists, unions, secrets and defaults, not only a flat field.
Secrets stay protected through the internal typed projection and explicit trusted
decode. Every action family rejects protected Action::Output domains recursively
at action admission, before any handler execution or serialization: stateless,
stateful, control, trigger, resource and all paging/batch/stream specializations.
Check nested/external records, newtypes, list/Option items and every enum variant,
even absent or inactive ones; apply this to manual adapters too. A local diagnostic
does not replace this admission gate. Actual ordinary output is serialized once
and validated as literal data against the exact admitted outbound schema before
publish/persist, including branch, partial, stream, deferred and trigger payload
publication paths. Never apply inbound aliases, transforms or defaults to repair
output. Serialization and output validation failures carry typed payload-free
errors, without raw values or
upstream messages/source chains; a redacting Serialize impl is no secret exemption.

Workflow assignability compares the producer's outbound projection with the
consumer's inbound projection, not their two inbound field sets. Preserve domain,
requiredness, aliases and secret boundaries in that directional comparison.
Equal inbound names do not make an edge compatible when the producer writes a
different outbound name. The plugin compiler must use that directional schema
API before recording connections; test both rejection and explicit compatible
consumer renames/aliases. Revision compatibility remains a separate contract.

No property-owned omission vocabulary is introduced. Initially reject
serde(skip), skip_serializing, skip_deserializing and skip_serializing_if on
schema data fields. A skipped field's fallback is invoked during Deserialize
even if a value was materialized beforehand, violating this data-proof contract.
Support for internal skipped state would need an explicitly separate, non-data
contract; do not silently omit it from a schema and still claim exact decoding.
Reject flatten, untagged unions, custom with/serialize_with/deserialize_with,
from/try_from/into and container defaults until an exact checked bridge exists.
HasSchema alone never establishes codec fidelity. Unsupported codecs belong only
on the explicit reviewed adapter path above, with checked descriptors/projections;
validation does not prove arbitrary adapter semantics.

Slots never become Field entries. Independently deriving Serialize/Deserialize
on a slot-bearing receiver requires explicit `serde(skip)` for every slot.
Guard fields without a real Default still cannot derive Deserialize merely by
being skipped; canonical receivers need no serde derives.
Schema on a receiver does not turn slots into data: retain visible slot markers
and require the owning Action/Resource declaration witness, with no standalone
Schema slot acceptance. No helper marker silently fabricates that witness.

### Trigger Output Contract (Target, Unshipped)

Current `PollAction` in `crates/action/src/poll/mod.rs` declares an independent
`Event: Serialize + Send + Sync`; its adapter serializes and emits events one at
a time. Current `TriggerEventOutcome` in `crates/action/src/trigger/mod.rs` exposes
`Emit(Value)` and `EmitMany(Vec<Value>)`. Checking Action::Output alone cannot
protect those payloads while these independent output paths remain.

Unify every outgoing workflow-start payload with Action::Output. Remove
PollAction::Event; `poll` returns `PollResult<Self::Output>`, including Ready and
Partial batches. Make the public author outcome generic, with no default Value
type parameter or raw-output compatibility overload. Illustrative, uncompiled:

```rust
pub enum TriggerEventOutcome<T> {
    Skip,
    Emit(T),
    EmitMany(Vec<T>),
}
```

`TriggerAction::handle` returns `Result<TriggerEventOutcome<Self::Output>, Self::Error>`.
Webhook author responses likewise carry `TriggerEventOutcome<Self::Output>` through
`WebhookResponse<Self::Output>`; HTTP response data remains transport-owned.
The raw inbound `TriggerEvent` envelope and `TriggerSource::Event` remain independent
transport input types. They do not become Action::Output or acquire output codec
bounds. This changes payload typing within existing behavior families, adding none.

Admission derives and retains the exact output schema from Action::Output and
rejects protected domains recursively before trigger activation, poll setup,
poll calls, event callbacks or output serialization. This includes nested/external
records, newtypes, list/Option items and every enum variant, even absent or inactive
ones, through generated and reviewed manual adapters. A redacting serializer, an
idle poll or a handler that would return Skip cannot exempt the declared domain.

For each completed event-handler call or emission batch, including a poll result,
serialize each outgoing item exactly once into private staging and validate it
as literal data against the retained schema's outbound projection. Never infer
expressions or apply inbound
aliases, transforms or defaults. Only after validation may the owning adapter
construct the runtime erased Value output boundary; publication consumes those
same checked values, without invoking author serialization again. No raw
`Emit(Value)` adapter, constructor or compatibility path may bypass this gate.

Whole-batch staging must enforce the owning runtime's explicit item-count and
aggregate encoded-byte caps before unbounded allocation or work. Check the item
count before reserving staging storage or serializing items. Use a bounded
serializer with checked size accounting as encoding proceeds, stopping at the
aggregate byte budget; serializing an unbounded value and measuring it afterward
does not satisfy this contract. Staging allocations and validation work must stay
within the admitted runtime budgets. These limits belong to existing runtime
policy, not author metadata, and apply equally to checked and explicit trusted
adapters. Limit or accounting overflow produces a typed payload-free error and
zero workflow publications from the call/batch.

Validate every element of EmitMany and poll Ready/Partial batches before any
workflow publication from that call/batch begins. A serialization or validation
failure rejects the batch with zero workflow publications, including when an
earlier item was valid; errors and observability remain typed and payload-free.
Staging state and this validation guarantee are scoped to one event-handler
call/emission batch, not a long-running start lifecycle or multiple poll cycles.
Skip/empty batches publish nothing. Durable transactions, publication failures,
cursor progress, retries and
cancellation remain with their existing runtime/trigger orchestration owners;
this gate does not promise an atomic transaction across all emitted workflows.

The public erased `TriggerHandler` is an independently callable ingress surface,
even though its implementations are sealed. All supported ingress and dispatch
paths, including factory handles, direct dyn calls, webhook callbacks, poll loops,
SDK harnesses and event sources, must route through the checked adapter or an
explicit trusted adapter enforcing the same admission and outbound gate under
the exact output schema. `accepts_events()` and metadata access alone are not
output admission. Author-facing lifecycle/context emission must also carry
Self::Output through that gate; raw ExecutionEmitter access belongs behind the
runtime boundary and cannot provide an alternate author publication path.

## Trait-Driven Data Domains

Reuse HasSchema for checked root schemas and Field/RootShape for representation.
Syntax-only matching of type names is insufficient for aliases, generics and
transparent newtypes. Existing derive rejection of generics and nested scalar
roots is a migration target, not a universal-inference implementation.

Introduce one schema-owned `PropertyType` field-descriptor trait: it supplies
`fn property(key: FieldKey, ctx: &mut SchemaBuildContext) -> Result<Field, ValidationReport>`,
including the domain/nullability and baseline missing-value policy. The narrow
schema-owned context enforces construction budgets before descending; it is not
a schema representation or runtime validation context. HasSchema describes
roots, not omission of an object member, and cannot currently describe every
nested Vec/Option field. Do not add a second data representation or evaluator.
Schema derives emit both HasSchema and PropertyType for supported data types.
Use explicit primitive impls, generic Vec<T>/Option<T> impls, and derived/manual
user-type impls; no overlapping blanket `T: HasSchema` specialization.
Aliases automatically reuse the actual type's implementation.

| Shape | Required descriptor behavior |
|---|---|
| Named record | Preserve all nested domains, root rules and canonical projections. |
| Vec<T> field | List domain with T's exact item descriptor and per-row paths. |
| Option<T> field | Permit null and omission unless explicit input policy requires presence. |
| Fieldless enum | Closed scalar domain from actual serde variant values; optional display labels. |
| Serde tagged enum | Exact external/internal/adjacent tagging and variant payload schema; reject impossible tag/payload combinations. |
| Transparent single-field newtype | Delegate root and field domain/projections to the inner type, retaining declared constraints. |
| Generic record/newtype | Generate bounds on the field types actually used and preserve existing where clauses. |

Schema-only lifetimes remain possible when no decoded owned-input contract is
claimed. Typed consumption additionally requires the directional codec contracts.
Unsupported tuples, unions, ambiguous wire shapes and custom codecs in the
generated path get explicit diagnostics, not Any/empty-object fallbacks.
Known numeric ranges and enum membership must survive nesting and newtypes.

Generic schema construction is uncached initially: a function-local static is
shared across monomorphizations. Do not add TypeId caches or specialization.
Non-generic successful/failed checked construction may retain current caching.
Type-dependent generic dependency descriptors obey the same rule: admit per
concrete factory; change static-only getter signatures where necessary instead
of leaking allocations or sharing one generic OnceLock.
PropertyType has no implicit Serialize, Clone or Debug bound. Default-bearing
fields add only the encoding bounds needed by their supported default bridge.

The initial descriptor model is a finite tree, not a recursive-reference schema.
Reject recursive data definitions, including Node with Vec<Node>, mutual cycles
and recursive generic instantiations. Charge depth and node budgets at every
descriptor expansion through SchemaBuildContext, before recursing, and return a
typed construction diagnostic when the bound is reached. Finite deep schemas
that exceed the same bounds are rejected too; do not guess recursive identity
from type names. Non-finite generic monomorphization may instead fail rustc.
HasSchema creates one context for the root and nested derives use PropertyType
with that same context, never nested cached HasSchema entrypoints. Cache only
the outer completed construction; no recursive OnceLock initialization. Manual
PropertyType implementations must use the same checked expansion API. Direct,
mutual and generic recursion fixtures must fail without a hang or stack overflow.

## Presence, Nullability and Defaults

Policy semantic v2 defines required as key presence after normalization/defaults.
Type/domain decides nullability; non_empty separately rejects empty strings or
collections. Display state never changes any of these decisions.
Baseline omission is allowed for Option or a supported serde fallback; otherwise
the field is required. `required_when(C)` replaces unconditional requirement only
for an omission-capable destination. It cannot make a missing String decodable.

| Input after alias normalization | Destination/policy | Result |
|---|---|---|
| Missing | String, no fallback | Required error; no decode. |
| Missing | Option<T>, no explicit requirement | Remains omitted and decodes None. |
| Missing | Option<T>, required or required_when(true) | Required error. |
| Missing | Option<T>, required_when(false) | Accepted as None. |
| Missing | String, required_when(false), no fallback | Reject declaration: not omission-capable. |
| Missing | Supported serde default | Insert default once, then validate domain and rules. |
| Explicit null | String, even with a default | Type error; never replace null with default. |
| Explicit null | Option<T>, including input(required) | Present and nullable; accepted. |
| Empty string/list | Matching type and required | Present; accepted unless non_empty/length rejects. |
| Nonempty value | Hidden or visible | Run the same domain and value validation. |
| Missing | Hidden and required | Required error, exactly once. |
| Pending display condition | Any value policy | No blockade on input admission. |
| Pending required condition | Missing omission-capable field | Retain an obligation; no proof or early success. |

The sole author source of a semantic default is field-level `serde(default)`
or `serde(default = "path")`. The bridge invokes that same deterministic typed
Default/provider, encodes it using the field's paired projections, and stores
its checked canonical value in the admitted descriptor. A default on a nested
record needs exact outbound-to-inbound conversion; lossy codecs are rejected.
Each default-bearing field therefore needs OutputCodec evidence as well as its
input direction, recursively for the bridge; this does not require Serialize on
the containing input DTO. Adapter fidelity remains an explicit review obligation.
Validate default domain and context-free rules at admission; cross-field rules
are checked with the complete invocation values. Encoding/provider failures
produce redacted admission evidence, never an empty/default-success substitute.
Provider signatures are the serde signatures, not a new fallible-default DSL.

Default providers must be deterministic, bounded, pure and free of I/O, clocks,
randomness or mutable process state. Rust macros cannot prove this: review and
determinism fixtures enforce the author contract. A provider panic is a fatal
author-contract violation, not a recoverable default/admission error. Release
uses panic=abort; catch_unwind cannot recover it. Even under unwinding a panic
hook may print its payload before a catch. Do not promise hook redaction or
install process-global hooks from schema code. Trusted default providers must
not panic or include secrets in panic payloads. Ordinary encoding/validation
errors still use the owned payload-free admission boundary.
Secret defaults, including defaults containing nested protected leaves, are
forbidden. Reject unsupported container-default extraction rather than guessing.

Materialize defaults only at missing paths, once per prepared subtree, before
validation and the condition snapshot. Newly evaluated subtrees undergo their
own first preparation; previously prepared siblings are never prepared again.
Validate constraints after insertion; preserve explicit null, false, zero and
empty values. Complete all pending obligations before trusted typed decode.
Serde must not supply additional undeclared values after the proof: supported
default paths are already materialized, while omitted Option decodes to None.
Schema export's default annotation never performs this mutation.
`display(example = literal)` is a non-mutating suggestion, never a fallback.
Unknown fields, invalid data and unsupported codec output cannot become accepted
by serde dropping them; schema rejection precedes decoder invocation.

## Conditions and Presentation

Add validator-owned Condition as a checked Rule refinement accepting predicates
and all/any/not only. Construction/deserialization must enforce that subset.
Reuse Predicate, FieldPath, evaluation outcomes, error types and Rule budgets.
No arbitrary expression script, closure serialization or separate evaluator.

```text
C := eq(reference, literal) | ne(reference, literal)
   | gt(reference, number) | gte(reference, number)
   | lt(reference, number) | lte(reference, number)
   | one_of(reference, [literal, ...]) | is_true(reference) | is_false(reference)
   | all(C, C, ...) | any(C, C, ...) | not(C) | condition(identifier)
```

All/any require at least one operand; one_of is nonempty. Comparisons lower to
the corresponding Predicate; one_of lowers to In. Numeric operands must be finite.
Missing and type-mismatch behavior follows the checked predicate contract;
unresolved values are Pending, never missing/false. Configuration/unavailable
errors stay errors through not and combinations, never negated into success.
Reuse MAX_RULE_DEPTH/NODES/OPERANDS/TEXT/JSON budgets, including after expansion
of named conditions; hosts may impose lower bounds, never bypass checks.

`#[schema(condition(name, C))]` declares a named condition in the data type's
schema scope; multiple differently named declarations are allowed.
`condition(name)` references that table. Duplicate, undefined or recursive names
are errors; no condition definition implicitly depends on another's UI state.
Typed builders provide the same checked named/inline behavior for manual schemas.

`field(identifier)` means a field in the containing record, resolved using its
canonical inbound serde key. The local derive checks identifier existence.
Inside list item records it refers to the same row; diagnostics use concrete
RFC6901 row paths. `root("/path")` means the absolute canonical data root.
Check pointer syntax at expansion and targets/domains against the complete
admitted schema. No wildcard, implicit row selection or ambiguous union target.
Paths into guarded variants must have statically well-defined domains.

Cross-type references cannot be proven by a proc macro reading one struct.
Slot policies over a separate associated input/config use named conditions
declared there or root references; direct field references on the receiver are
rejected. External named/absolute targets are checked at leaf admission.
Aliases are already consumed when references are evaluated. Sources of expressions
and plaintext secrets are unavailable; secret targets are rejected at admission.
Binding selectors additionally must be declared literal-only and nonsecret.
Every ancestor capable of producing a selector subtree must also prohibit
expressions, including the associated input/config root. The owning declaration
records these restrictions for the selector's entire contributing path. Admission
rejects conflicting explicit expression permission; preparation rejects programs
at any of those paths. Evaluator output becoming literal data is not evidence of
literal authoring. The prepared witness retains authored provenance, including
for selectors inside nested objects or tagged variants.

Reject cycles in conditional requirement dependencies and loader dependency
graphs; include named-condition expansion and nested paths in the graph.
Pure display reads depend on data, not recursively computed show-state.
Display conditions may remain pending without delaying data proof.

Presentation-only `host("registered.fact")` is an additional reference form.
It reads a separately registered, typed presentation context, not the schema
value namespace. Unregistered/unsupported facts are errors, not false.
Host facts provide no authority and are invalid in required_when, bind_when,
loader data dependencies, or named conditions used in those positions.
Neither a hidden field nor a visible privileged control authorizes an operation.

## Options and Loaders

`options(source = RustPath, depends_on(...), mode = suggestions)` names a
statically registered typed provider descriptor, not an arbitrary function to
serialize. Reuse LoaderRegistry, LoaderResult and schema-bound redacted contexts.
A narrow registration adapter may associate provider identity/value domain and
selected-label lookup with those existing loaders; it is not another registry.
Schema discovery performs no I/O. Registration checks provider identity, field
domain and dependency targets before catalog publication.

Initially reject options on secret-bearing domains, including nested protected
leaves and external child schemas. Reject obvious input(secret)+options at
expansion and check the entire result domain again at admission. Existing
SelectOption values, Serialize/Debug and selected-label lookup are public-data
surfaces; redacting the request context does not protect their results. No loader
or selected-value lookup may run for a rejected protected field. A future secret
choice protocol would need a separate explicit disclosure contract.

Static fieldless enum options are closed by the actual schema domain, whether
or not a select widget or EnumSelect labels are used. A static typed option set
marked closed adds an explicit admitted finite-domain rule.
Remote/mutable providers are suggestions by default. Initial implementation
rejects closed remote mode at admission: an authoritative membership rule
provider has not been selected. A successful remote list is never an
authorization decision or the sole validation proof for a submitted value.

Loader requests include only declared, normalized, nonsecret dependencies.
Host dispatch binds tenant, concrete bindings, schema identity, field path,
dependency values, query, cursor and provider/data revision to the cache key.
Apply timeout/cancellation and existing page/depth/byte budgets. Unknown or
stale cursors/revisions are errors; do not serve another scope's cached results.
Check returned option values against the declared domain and redact diagnostics.
Fetch labels for already selected values even when outside the current page;
unavailable/deleted selections have explicit unresolved labels, not silent removal.
A provider failure is unavailable/error, not a successful empty options page.

## Slot Grammar and Identity

```text
#[slot(credential | resource,
       key = "...", purpose = "...", bind_when(C), binding = required | optional)]
```

Exactly one kind is required. Key/purpose/bind_when are optional singletons.
Key defaults to the Rust receiver field name, not a serde name.
Binding describes whether an ACTIVE slot may lack a binding; the Rust wrapper
describes whether the receiver can represent absence. Resource credential cells
default to required. Action Option<Guard> defaults to optional, plain Guard to
required. An Option<Guard> may explicitly use binding = required: false condition
still yields None, but true requires successful acquisition. Plain Guard cannot
use binding = optional or bind_when. There is no separate optional flag.
Lazy and every Lazy wrapper are explicitly unsupported in the initial grammar:
the current action lazy expansion already resolves before Lazy::with_value.

| Receiver | Accepted field type | Meaning |
|---|---|---|
| Action | CredentialGuard<S> | Required projected auth scheme S. |
| Action | Option<CredentialGuard<S>> | Inactive yields None; active absence follows the binding policy. |
| Action | ResourceGuard<R> | Required lease for Provider R. |
| Action | Option<ResourceGuard<R>> | Inactive or optional resource dependency, with independent active binding policy. |
| Resource | CredentialSlot<S> / SlotCell<CredentialGuard<S>> | Generation-stamped cell of projected auth scheme S; binding controls absence. |

S consistently means the auth Scheme, never a Credential implementation type.
Preserve credential-owned guards and resource-owned CredentialSlot/SlotCell.
Action resolution bridges must resolve by admitted scheme compatibility instead
of treating S as a provider with Credential::KEY. Different credential providers
may project the same scheme after registry validation, without branching stored
provider mechanics. Distinct setup data shapes use a tagged enum.
No concrete-provider restriction syntax is shipped;
adding one later requires a separately typed, checked catalog restriction.
Resource slots on Resource, slots on credential property data, Option<Cell>,
guard aliases/new wrappers without a supported slot-shape contract, and
bind_when on a non-Option action guard are rejected.

| Identity | Scope and purpose |
|---|---|
| Slot key | Local declaration/binding address, unique across both kinds on a receiver. |
| Catalog type key | Stable resource/credential provider identity, checked in the catalog; not inferred from a scheme's Rust name. |
| Rust TypeId | In-process type compatibility only; never serialized, persisted or treated as an instance ID. |
| Concrete instance ID | Selected CredentialId/resource registration identity within the authorized owner/scope. |

A type declaration, type key, default ID or matching TypeId grants no authority.
Keep existing explicit-node-binding then default-ID lookup, but only an owner's
typed NotFound result for an unconfigured default may count as optional absence.
Configured missing/denied/type-mismatched/timed-out/revoked bindings fail.
Do not swallow these errors because the slot is optional or a field is hidden.

## Slot Declaration and Runtime Contract

Leaf richer declarations are the SINGLE authored source of slot definitions.
Derives mechanically project existing core requirements and slot_fields from
them; authors do not maintain a second independent list beside Dependencies.
Admission checks identical keys, kinds, type compatibility, requiredness and
default binding identity in every projection before publication.
Current core credential identity assumes a concrete Credential rather than
a Scheme: revise that descriptive contract explicitly; do not invent a provider
key or misuse a scheme TypeId to satisfy it. Core gains no Rule/Condition import.
Action/resource leaf descriptors compose checked Condition with core declarations.

All candidate branches are statically declared and validated. Invocation
decisions never mutate static declarations or remove slots from the catalog.
Bind conditions read prepared associated input/config only. No slot guard,
host fact, secret value or runtime-selected undeclared slot enters that graph.

| Condition/binding state | Resolution and author-visible state |
|---|---|
| False | Inactive; no lookup/acquisition; optional action guard None, resource cell empty. |
| Pending/error | No action/provider operation; retain obligation or return typed error. |
| True, required, missing | Error before operation. |
| True, optional, unconfigured/default NotFound | Absent; None/empty cell. |
| True, configured binding fails | Typed error, never None success. |
| True, successful | Action owns guard/lease; resource cell holds the projected guard snapshot. |

Keep FromWorkflowNode in action as the construction entrypoint. Extend the
owning action port to receive or consume a prepared invocation witness that
binds exact schema/policy identity, canonical selector values and node bindings.
The engine supplies it through downward ports; no action-to-engine dependency.
Preparation and condition evaluation precede slot resolution and operation.
Do not reread raw parameters or independently deserialize a second input.
The typed input moved into execute must be the one validated by that witness.
Factory/handle sequencing changes are required; merely retaining today's
(node, ctx) signature cannot prove this relationship.

Resource registration analogously pins admitted config and exact bindings
before create. Config reload reevaluates conditions and reconciles cells,
generation/epoch and live instance lifecycle through resource-owned hooks.
An inactive or revoked slot cannot leave a live authenticated resource silently
using a stale binding. Cancellation drops acquired leases; no partial operation.
Resource `self.auth_slot()` returns Option<Arc<CredentialGuard<S>>>: retain that
owned snapshot across await; never return a borrow from a temporary Arc load.
Action guard borrows are bounded by the receiver's lifetime. Resource lease
ownership/release remains with ResourceGuard; no accidental guard Clone bound.
Runtime keeps inactive versus absent decision evidence even if both map to None;
user code handles None explicitly and never relies on unconditional Deref.
Long-lived trigger/resource operations use their owning lifecycle/rebind policy,
not an unbounded assumption that one invocation's binding proof stays fresh.

### Durable Compilation

The plugin compiler remains pure: it neither resolves concrete tenant bindings
nor acquires guards. Its new recorded binding contract pins the stable scheme
contract identity/version, slot key/kind, active binding policy and checked
condition with its exact input-schema/policy identity. Scheme identity must be
an explicit checked scheme-owned key/version, never a Rust name or TypeId.
Record the deterministically ordered compatible provider definitions selected
from the exact frozen catalog, with provider keys/versions and required
capabilities. Invocation resolution may select only from that admitted set;
adding providers requires a fresh plan, not ambient global lookup.

The candidate set is bounded by the composition's admitted frozen plugin
closure, including inactive branches; no ambient registry can add candidates.
Preserve required Cargo/manifest edges for actual cross-plugin Rust type
references. A scheme-only declaration depends on its scheme contract; it must
not invent a concrete provider type dependency on every compatible adapter.
Plugin-owned compilation checks both the declared type-dependency graph and
candidate membership/version compatibility in the frozen composition. This
explicitly replaces today's concrete-provider-key lookup for scheme slots;
it does not weaken closure checks for concrete resource/provider references.
Recorded slot requirements describe possible dependencies, not tenant authority
or credential instances. Concrete binding evidence is supplied only by owners
at invocation. Compare the complete recorded declaration and candidate set at
readmission against the exact frozen catalog. A changed condition, provider,
scheme version or capability requirement invalidates stale evidence.

This requires a new plugin compiler epoch and versioned binding/schema records,
not mutation of RecordedBindingContractV1. Existing epoch 1/3 schema-envelope-v1
and epoch 4 scalar-envelope-v2 rules stay closed. Choose a new schema envelope
version for policy-v2 definitions; envelope v2 is already used for scalar roots.
Preserve old plan/hash golden bytes and canonical_json_v1. Add new-epoch golden
records plus negative dependency-closure, candidate drift and readmission tests.

## Metadata Construction and Admission

**Current:** shared setters and factory/registry schema admission already exist.
**Target, unshipped:** issue 1018 closes constructor/SDK parity; Metadata Evolution
below adds substantive catalog contracts.
Keep concrete ActionMetadataDraft, CredentialMetadataDraft, ResourceMetadataDraft.
Their target signatures (uncompiled):

```rust
fn new(key: K, name: MetadataName, description: impl Into<String>) -> Self;
fn try_new(key: K, name: impl Into<String>, description: impl Into<String>)
    -> Result<Self, MetadataError>;
```

Each K is the leaf's typed key. Constructors take three arguments; derive pattern
from C::Scheme at admission, replacing Credential's fourth AuthPattern argument.
No inferred display name from a key/from_key path.
No generic IntegrationDraft or setter extension trait is needed.

Retain with_version(MetadataVersion), typed Icon methods, with_tags/add_tag,
mark_experimental/mark_beta/mark_stable and with_deprecation. The target refines
deprecation payloads and documentation storage as specified below; existing
with_documentation_url remains a convenience over that single representation.
An attached notice still wins over active maturity; construction/admission is checked.
Drafts are private-field, consuming/must_use and non-deserializable; no public
build, schema setter or caller-supplied proof. BaseMetadata stays getter-only;
shared authoring delegates to MetadataDraft.
The lower MetadataDraft::bind_schema transition becomes fallible:
`fn bind_schema(self, schema: ValidSchema) -> Result<BaseMetadata<K>, MetadataBuildError>`.
It enforces all shared field and aggregate limits, including on intent built
through infallible new/with_* methods. Leaf factories propagate this failure;
only they add associated-type and leaf admission checks. Checked primitive
constructors and try_new provide earlier diagnostics, not a final-proof bypass.

Macros reject typos, require explicit key/name/description and emit the same
constructors/with_* methods; container-specific behavior stays with its owner.
Factories/credential registry alone bind schemas and leaf invariants. Hidden SDK
expansion paths grant no authority. Exact readmission covers shared fields plus
the full schema/policy/default/projection/options-provider and slot contract.

## Metadata Evolution

**Core Phase5:** discovery, typed links/deprecation, derived leaf projections and
a closed catalog protocol with compatibility rules. New names are unshipped targets.
No generic metadata container, arbitrary JSON extensions or author supports_* flags.

| Owner | Authored declarations | Derived evidence / boundary |
|---|---|---|
| metadata | Canonical name/description, categories, search tags, Icon, links, lifecycle | Checked shared catalog fields; no leaf runtime dependencies. |
| schema | Property display, input, rules and options declarations | Exact associated-type schema and preparation policy; property hints stay here. |
| action / credential / resource | Existing leaf policies and single slot declarations | Factory/registry facts and checked projections; no second declaration list. |
| PluginManifest / packaging | Bundle metadata, author/license, package and SDK constraints | Plugin membership/dependency closure; no duplicated leaf package fields or leaf schema on manifests. |
| host | Deployment and access policy | Tenant availability, selected instances, trust and future locale overlays; never authored leaf authority. |

**Discovery.** Add with_categories over checked CatalogCategoryKey values.
Categories are stable structured filter keys; existing with_tags/add_tag supply
search keywords ("postgres", "sql"); no second Keywords field.
Canonicalize category sets and trimmed search tags by sorting/deduplicating;
category labels and host ranking cannot change entity identity or admission.

**Documentation.** Add add_link(CatalogLink), with closed relations Overview,
Setup, Reference, Migration and Troubleshooting and a checked URL target.
with_documentation_url sets/replaces the single Overview link; there is no
independent documentation_url storage. Conflicting Overview entries are errors;
identical relation/target pairs deduplicate. New link targets accept HTTPS or
root-relative paths, rejecting scheme-relative paths, userinfo and executable,
file or data URLs. Discovery/admission performs no URL fetch or provider I/O.
Resolve root-relative targets only against an explicitly configured host
documentation origin; without one they remain unresolved links. Use a URL parser
and require the resolved origin to match that base, including after normalization;
reject backslashes and other authority-changing spellings. Absolute HTTPS links
remain explicit external links, subject to host navigation policy.
Host rendering treats text as text; link publication is not fetch authorization.

**Evolution guidance.** Replace raw replacement strings with CatalogReference:
Action(ActionKey), Credential(CredentialKey), Resource(ResourceKey) or
Plugin(PluginKey), using existing core keys and optional target VersionReq.
Keep cross-family intent: an action may recommend a resource plus a Migration
link. References neither replace the source key nor prove assignability or grant
bindings. Validate kind/key/version syntax locally; an absent target is unresolved
guidance, not an implicit runtime dependency or a reason to drop the source entry.

RemovalSchedule distinguishes OnDate(checked calendar date), AtVersion(Version)
and Milestone(checked nonblank label); absence means no announced removal.
AtVersion refers to the source entity's interface version, or bundle version
for a manifest. Require since <= current version by SemVer precedence and an
AtVersion removal later than since; a passed schedule remains valid evidence.
Schedules announce intent; no automatic deletion, execution ban or migration.
The existing notice-implies-Deprecated invariant survives setter order and serde.

Illustrative target declarations, not executable Rust or current wire syntax:

```text
action: postgres.connect; categories: [database]; tags: [postgres, sql]
links: [Setup -> /docs/postgres/setup, Migration -> /docs/postgres/v2]
deprecation: since 2.0.0, replacement Resource(postgres.client) @ ^2
removal: AtVersion(3.0.0)
```

**Derived leaf facts.** Extend existing CredentialTypeInfo/TypeCapabilities and
its registry projection to include INTERACTIVE and DYNAMIC alongside REFRESHABLE,
TESTABLE and REVOCABLE. These five facts come from capability membership, never
draft flags. Derive pattern from C::Scheme; AuthPattern remains cosmetic and cannot
prove slot compatibility. Export the checked scheme key/version defined by the
slot contract, never TypeId or a Rust type name. Reuse existing plugin snapshots
for action kind/schemas/effects and action/resource slot evidence.

Static topology export is deferred until a pure factory projection of TopologyTag
exists; today tag(&self) requires an instance. Discovery never constructs instances.
Tags, especially Custom/Bounded, do not certify concurrency guarantees.

**Protocol and readmission.** The integration catalog export MUST have an explicit
catalog wire version and closed typed requirements for its actual schema wire,
schema policy, slot and options contracts. Derive requirements from admitted
definitions; they are not author-selected runtime features or SDK constraints.
Keep these version domains distinct from plugin compiler/schema envelopes and
interface SemVer. Unsupported versions/required fields fail explicitly at ingress;
no default-empty semantics, ignored obligations or promotion of unknown evidence.
Recorded shared and leaf DTOs remain evidence. Compare every authored field and
recompute leaf facts against the selected fresh definition/frozen snapshot before
readmission; return fresh values only. A compatible revision is not an exact match.

| Changed contract | Revision compatibility | Exact readmission |
|---|---|---|
| Entity key | Immutable; replacement is a reference to another definition. | Reject mismatch. |
| Fallback text, categories, tags, links | No interface major required; publish fresh catalog evidence. | Reject changed canonical fields even at equal SemVer. |
| Deprecation/removal guidance | Check chronology and lifecycle; no automatic execution ban. | Reject changed notice/schedule. |
| Schema, including UI/defaults/options | Keep conservative schema equality gate requiring a major bump. | Reject any definition or policy mismatch. |
| Kind, effects, scheme, slots, capabilities, execution policy | Leaf owners specify version rules; capability removal or changed required slots/scheme requires major. | Recompute and compare the complete leaf contract. |
| Catalog wire / required protocol semantics | Explicit versioned migration; reject unsupported versions. | Never reinterpret old evidence under new semantics. |

Phase5 bounds: at most 16 categories (96 UTF-8 bytes/key), 32 tags (64 bytes/tag),
16 links (2048 bytes/target), 8 KiB description and 256 bytes/milestone.
Limit serialized shared authored fields, excluding bound schemas, to 32 KiB;
existing name/key checks and schema budgets still apply. Bound decoding before
unbounded allocation, and allow hosts only lower limits. Dynamic authoring and
recorded ingress use the same checks. Errors/spans carry codes and field locations,
never supplied text, URLs, schema values or parser source payloads.

**Later delivery.** Canonical author text is the fallback now. A future host/plugin
locale overlay must address existing entity identity, exact definition revision
(not SemVer alone), schema path and text slot. Missing/stale overlays use fallback;
they cannot modify validation, defaults or canonical evidence. No schema -> metadata
DisplayText dependency: metadata already depends on schema. Locale bundles and
overlay services are deferred, as are automatic migration services and independently
versioned presentation identity. Manifest deserialization proves structural validity,
not publisher trust; tenant availability and trust remain host-owned projections.

## Versioning and Breaking Migration

Introduce an explicit schema policy semantic v2 envelope, conceptually
`{ policy_version: 2, schema_wire_version: N, definition: ... }`.
This envelope identifies admission semantics, independently of encoded shape.
Policy version alone must change even when definition bytes would be identical.
Adding nullable/condition/default/projection fields to durable definitions also
requires a SCHEMA_WIRE_VERSION bump from historical v1; v2 is the target here.
That schema crate constant is not the plugin's schema-envelope discriminator:
plugin envelope v2 already identifies scalar roots and cannot be repurposed.
Do not label new definition fields as policy-only metadata to avoid that bump.
Catalog slot/options extensions have their own integration catalog version.

Historical schema wire v1, authored-value wire, tree canonical formats and
canonical_json_v1 are distinct contracts. Preserve all prior persisted bytes and
canonical_json_v1 encoding. New admission rejects unsupported old policy
envelopes explicitly; it never silently reinterprets them as v2.
Migration creates new versioned definitions and requires fresh admission;
old stored records remain evidence, not automatically upgraded proof tokens.

Replace visibility-waived requiredness with explicit required_when on an
omission-capable field, or preferably a tagged union for exclusive auth modes.
Migrate old required/nonempty intent separately. Replace UI-only defaults with
display examples or deliberate serde defaults, reviewing the behavioral change.
Move legacy field/validate helpers to structured property, field-level
credential/resource helpers to slot, and emit_as/key intent to directional serde.
Remove temporary legacy parser aliases at the breaking release boundary.

For triggers, move the outgoing `PollAction::Event` type to `Action::Output` and
return `PollResult<Self::Output>`. Replace raw author outcomes with
`TriggerEventOutcome<Self::Output>`, including `WebhookResponse<Self::Output>`, and
supply schema_type(output) or an explicit reviewed OutputCodec adapter for that
DTO. Preserve inbound TriggerEvent/TriggerSource::Event transport contracts.
Migrate factory/handle adapters, direct TriggerHandler consumers, lifecycle/context
emitters, webhook responses, poll Ready/Partial dispatch, SDK exports, examples
and fixtures together. Remove raw Emit(Value) compatibility paths; establish the
checked batch boundary before forwarding to existing transaction orchestration.
Thread the runtime-owned item and aggregate encoded-byte limits through every
adapter; stage with bounded serialization and checked size accounting.
Refresh catalog/plan evidence when the declared output changes, using the same
versioning and exact readmission rules above; old metadata is not output proof.

ADR-0101's engine Slots move remains deferred; full auth-runtime rearchitecture,
performance claims/optimizations and new canonical value storage are non-goals.

## Authoring Examples

All AFTER snippets are illustrative new syntax, UNCOMPILED. Imports and required
methods explicitly noted as omitted must be supplied by implementation fixtures.
Before excerpts reference observed files; they are not invented passing tests.

### Schema Data

Before: [derive_schema.rs](../tests/derive_schema.rs), HttpInput URL declaration.

```rust
#[field(label = "URL", hint = "url")]
#[validate(required, url, length(max = 8192))]
url: String,
```

After, inside a data type; a serde default replaces the old UI-only default:

```rust
#[schema_type(input)]
struct HttpInput {
    #[property(display(label = "URL"), input(required),
               validate(non_empty, url, length(max = 8192)))]
    url: String,
    #[serde(default = "default_method")]
    method: String,
}
fn default_method() -> String { "GET".to_owned() }
```

### Action

Before: [derive_action.rs](../../action/tests/derive_action.rs), NoCredAction
declares `input = serde_json::Value, output = serde_json::Value`; slot-shape
probes live in [derive_action_compile_fail.rs](../../action/tests/derive_action_compile_fail.rs).
After extends that existing associated-input pattern with typed data and guards:

```rust
#[schema_type(input)]
#[schema(condition(use_auth, eq(field(authenticated), true)))]
struct SendInput {
    #[property(input(expressions = forbidden))]
    authenticated: bool,
    #[property(input(required), validate(non_empty))]
    channel: String,
}
#[schema_type(output)]
struct SendOutput {
    delivered: bool,
}
#[derive(Action)]
#[action(key = "send", name = "Send", description = "Send a message",
         input = SendInput, output = SendOutput)]
struct SendAction {
    #[slot(credential, key = "auth", binding = required,
           bind_when(condition(use_auth)))]
    auth: Option<CredentialGuard<SecretToken>>,
}
impl StatelessAction for SendAction {
    async fn execute(&self, input: SendInput, ctx: &(impl ActionContext + ?Sized))
        -> Result<ActionResult<SendOutput>, ActionError> {
        match self.auth.as_ref() {
            Some(guard) => send_authenticated(&input.channel, guard, ctx).await?,
            None => send_public(&input.channel, ctx).await?,
        }
        Ok(ActionResult::success(SendOutput { delivered: true }))
    }
}
// Imports and the two application send helpers are omitted.
```

### Credential

Before: [credential_attr_macro.rs](../../credential/tests/credential_attr_macro.rs)
uses `#[credential(...)] impl MetadataOnly` with `Properties = serde_json::Value`,
explicit Scheme/State and handwritten project/resolve. After keeps that owner:

```rust
#[schema_type(input)]
#[serde(tag = "mode", content = "credentials", rename_all = "snake_case")]
enum AuthProperties {
    ApiKey(ApiKeyProperties),
    Basic(BasicProperties),
}
#[schema_type(input)]
struct ApiKeyProperties {
    #[property(input(secret, required), validate(non_empty))]
    key: SecretString,
}
struct ApiCredential;
#[credential(key = "api", name = "API", description = "API authentication")]
impl ApiCredential {
    type Properties = AuthProperties;
    type Scheme = SecretToken;
    type State = SecretToken;
    // Required project and async resolve bodies omitted; BasicProperties omitted.
    // Additional recognized methods here determine capability trait membership.
}
```

### Resource

Before: [resource_config_derive.rs](../../resource/tests/resource_config_derive.rs)
uses NamedCfg with `ResourceConfig, Schema` and `config(schema = external)`.
[derive_slot_accessor.rs](../../resource/tests/trybuild/derive_slot_accessor.rs)
exercises the existing cell/accessor shape; its generic naming is not the new
scheme contract. After preserves separate Provider/config and owned snapshots:

```rust
#[schema_type(input, output)]
#[derive(Clone, ResourceConfig)]
#[schema(condition(use_auth, eq(field(authenticated), true)))]
struct ClientConfig {
    #[property(input(expressions = forbidden))]
    authenticated: bool,
    url: String,
}
#[derive(Resource)]
struct Client {
    #[slot(credential, key = "auth", binding = required,
           bind_when(condition(use_auth)))]
    auth: CredentialSlot<SecretToken>,
}
impl Provider for Client {
    type Config = ClientConfig;
    type Instance = HttpClient;
    type Topology = Resident<Self>;
    async fn create(&self, config: &ClientConfig, ctx: &ResourceContext)
        -> Result<HttpClient, Error> {
        let auth = self.auth_slot();
        match auth.as_deref() {
            Some(guard) => connect_authenticated(config, guard, ctx).await,
            None => connect_public(config, ctx).await,
        }
    }
    // Required key and explicit metadata methods, topology hook impl, imports,
    // HttpClient and application connect helpers omitted.
}
```

## Diagnostics and Acceptance

Compile diagnostics name offending tokens and accepted keys/shapes; combine spans
for duplicates. Rust bounds diagnose generic/secret destinations; external
schemas and runtime provider behavior are not visible to a proc macro.

| Stage | Required rejection/evidence |
|---|---|
| Compile | Unknown/duplicate property keys, flat key/default/emit_as, conflicting sections/modes and local serde key/alias collisions. |
| Compile | Slot/property mixing, wrong receiver, wrong wrappers, Option<Cell>, lazy, conditional non-Option action guard. |
| Compile | Unknown local field/name, malformed pointer/condition, unsupported serde codec/flatten, incompatible derived trait ownership. |
| Compile | Missing recursive InputCodec/OutputCodec/PropertyType bounds, default bridge encoding evidence or SecretInput; Schema-only supplies no codec witness. |
| Compile | PollAction::Event declarations and poll/event/webhook payloads differing from Self::Output fail; missing recursive output codec/schema evidence fails on trigger DTOs too. |
| Compile | Unsupported owner ordering/visible transformations/cfg composition or serde(crate) override; conflicting impls fail rustc coherence, not macro inspection. |
| Compile | Direct input(secret) on a ResourceConfig-derived field; use a credential slot on the resource receiver. |
| Schema admission | External path/name/domain errors, cycles/budget overflow, hidden secret descendants in resource configs, secret defaults and unavailable providers. |
| Leaf admission | Protected output domains in every action family before handlers/serializers; slot projection mismatch, scheme compatibility, forbidden host facts/selectors, closed remote options and stale policy versions. |
| Trigger admission | Protected Self::Output rejected before activation, poll setup/poll, event callbacks or serialization across checked and explicit trusted adapters, including direct TriggerHandler ingress. |
| Metadata authoring/admission | Invalid category/link/reference/schedule, conflicting Overview or exceeded byte/count budgets; typed payload-free errors across manual and macro paths. |
| Metadata recorded ingress | Unknown catalog/protocol versions, missing obligations or changed authored/derived evidence rejected; only fresh definitions returned. |
| Runtime | Default/condition/loader failures, incomplete proof, explicit binding errors, cancellation, rotation/reload, decode errors and invalid actual outbound payloads; payload-free codec errors. |
| Trigger runtime | Serialize once and validate all outputs under the exact admitted outbound schema before creating the erased boundary or publishing any workflow from that call/batch; no raw Value bypass. |
| Trigger runtime limits | Runtime-owned item-count and aggregate encoded-byte caps bound staging, serialization and validation work; limit/accounting overflow is payload-free and publishes nothing. |
| Review | Manual adapter semantic fidelity, arbitrary unsafe Debug/secret handling and nondeterministic defaults cannot be proved by markers, round trips or compile-fail tests. |

| Acceptance scenario | Observable requirement |
|---|---|
| Rename + alias + directional output | Canonical input reaches Deserialize; output key only reaches export. |
| Codec owner and schema-only | All three owner modes work; Schema-only remains descriptive but fails typed admission bounds even with manual Serde; conflicting manual impls fail coherence. |
| Recursive codec requirements | Missing directions in external children, Vec/Option, newtypes, enum payloads and two generic instantiations fail at field bounds; reviewed adapter opt-in is explicit. |
| Owner composition / SDK hygiene | Unsupported visible transforms/cfg_attr fail; whole-item cfg works; renamed SDK-only consumers use real Serde derives without author serde(crate) overrides. |
| Default encoding direction | Input-only root works without Serialize; missing nested OutputCodec for a default bridge fails; protected defaults remain rejected. |
| Action output protection | Every behavior family rejects direct/nested/external/optional/inactive protected domains with zero handler/serializer calls. |
| Trigger typed author contract | `PollResult<Self::Output>`, `TriggerEventOutcome<Self::Output>` and `WebhookResponse<Self::Output>` compile with recursive output evidence; independent Event, mismatched payloads and missing evidence fail. Inbound transport events may differ from Self::Output. |
| Trigger protected admission | Base, poll and webhook triggers with direct/nested/external/newtype/list/optional/inactive protected outputs fail with zero activation, poll setup/poll, event callback and serializer calls, including manual adapters. Skip/Idle intent grants no exemption. |
| Trigger literal output | Valid renamed output serializes once per item and publishes the exact checked value; wrong outbound keys/domains fail without inbound repair, and expression-looking strings remain literal. Serializer/validation diagnostics expose no payload or upstream source text. |
| Trigger batch validation | An invalid or serialization-failing later item in EmitMany or poll Ready/Partial yields zero publications for that call/batch; all-valid batches publish only after every item passes, with no reserialization. Skip/empty batches publish nothing. |
| Trigger batch budgets | At-cap valid batches succeed; over-item-count batches invoke zero serializers. Aggregate encoded-byte or accounting overflow stops bounded encoding without later-item serializer calls and yields zero publications. Assert serializer/encoder-work and publication counters, including overflow after a valid earlier item, through checked and trusted adapters; diagnostics remain payload-free. |
| Trigger ingress coverage | Factory handles, direct public TriggerHandler calls, webhook callbacks, poll loops, SDK harnesses, event sources and lifecycle/context emitters use checked or explicit trusted adapters with the same gate. Raw Emit(Value) bypass and stale/mismatched output-schema evidence are rejected. |
| Actual outbound validation | Invalid branch/partial/stream/deferred ordinary output cannot publish/persist; no inbound repair; malicious serializer error text stays out of public diagnostics. |
| Directional workflow edge | Producer outbound checked against consumer inbound; equal inbound-only keys cannot hide incompatible output. |
| Skipped data fallback | Derived data with serde(skip/default) fails explicitly; no hidden default runs after proof. |
| Recursive data definitions | Direct, mutual and generic recursion rejected by bounded construction/compiler diagnostics without hanging or overflowing. |
| Panicking default provider | Release subprocess terminates without producing admitted metadata; no recoverable-error or panic-hook-redaction claim. |
| Alias/newtype/generic SDK data | Correct scalar/nested domains for two distinct instantiations, with no leaf dependency or shared generic cache. |
| Missing/null/empty/default matrix | Exactly the table's outcomes; defaults/rules once, no decode on failed proof. |
| Hidden and pending display | No required waiver or input blockade; supplied hidden data validated. |
| Tagged auth union | Invalid/mixed mode payload rejected; active variant secrets protected. |
| Direct resource secret | ResourceConfig derive rejects the property at its attribute span; schema-only and credential secret properties remain supported. |
| Nested resource secret | Admission rejects before registration/create, including external child schemas. |
| Slot branch false/true/pending | Zero resolution when inactive; exact prepared input governs activation; pending never becomes false. |
| Optional explicit binding failure | Denial/type/timeout/revoke remains error; no silent None. |
| Conditional required binding | False yields None without lookup; true with no binding fails before execute/create. |
| Nested binding selector | Expression at selector or any contributing ancestor rejected; evaluated object cannot masquerade as authored literal. |
| Recorded slot plan | Exact scheme/provider/condition evidence, plugin closure and new-epoch golden vectors; old bytes unchanged. |
| Resource rotate/reload/cancel | Owned snapshot lifetime, generation tracking and lease cleanup retain owner contracts. |
| Options paging/selected labels | Scoped cache, typed results, errors distinct from empty, no remote authority claim. |
| Protected options domain | Direct/nested/external secret fields rejected before provider dispatch, result export or selected-label lookup. |
| SDK-only and renamed SDK | All new derives/impl macros work through curated exports; no exposed store/tenant/proof constructors. |
| Persisted v1 evidence | Bytes unchanged; unsupported policy rejected; migrated definitions freshly admitted. |
| Metadata discovery and links | Categories filter, tags search; canonical set ordering and one Overview via both APIs; malformed URLs/budget overflow rejected without I/O or payload disclosure. |
| Metadata cross-family replacement | Action -> Resource reference admitted; unavailable target stays unresolved guidance; invalid key/version/removal chronology rejected; no automatic migration. |
| Metadata exact evidence | Text/link changes may pass revision compatibility but fail stale readmission; schema UI still requires major; lifecycle precedence survives setter order/serde. |
| Metadata derived facts | Existing credential projection exposes all five registry capabilities and checked scheme identity; forged/stale leaf evidence fails; discovery constructs no resource/topology instances. |
| Metadata fallback and boundary | Catalog works without localization/tenant state; fallback remains canonical; new authored support/trust/availability flags are rejected. |

## Ordered Delivery and Review

These are ordered implementation dependencies, not additional tracked plan files.
Each issue must update its original scope to this contract before implementation.

1. **1018:** constructor/SDK parity for existing metadata drafts; no new icon inventory.
2. **Metadata evolution prerequisite:** after 1018, implement shared category/link/reference/schedule contracts, bounded admission, evidence and compatibility rules; specify the mandatory catalog envelope. Leaf tasks consume this foundation and extend existing derived projections; the Plugin prerequisite completes export/readmission integration. Localization services and static topology export are deferred.
3. **Validator prerequisite:** Condition refinement, presence semantics, budgets and policy-v2 contract; independent of metadata/schema imports.
4. **995:** schema-codegen package and gates, schema_type ownership, recursive directional codec contracts and reviewed adapter path, grammar, descriptors, projections, defaults, conditions/options and versioned admission; consumes the validator prerequisite.
5. **994, contract subtask:** leaf slot declarations and prepared-input port signatures over core; consumes 995, keeps a single projected declaration source. Separate this from 994's later end-to-end production wiring.
6. **997 / 998 and a resource follow-up:** action, credential and resource implementation after the declaration/codec contracts, including all-family protected-output admission and actual outbound validation. The action task removes PollAction::Event, types trigger/webhook outcomes with Self::Output and implements per-call/batch validation before erasure/publication across checked and explicit trusted adapters, with runtime-owned item/encoded-byte caps, bounded serialization and overflow counter tests. Issue 999 is already closed; scope a new resource task instead of treating it as unfinished. No dependency on final SDK exports.
7. **Plugin prerequisite:** versioned durable slot/schema records, compiler epoch, directional connection checks and readmission after leaf contracts; coordinate with 1014 without waiting for its unrelated freeze work.
8. **994, wiring subtask:** production resolution and engine sequencing after leaf implementations and the plugin prerequisite; prove exact input/binding provenance end to end. Route every trigger ingress and lifecycle/context emission through the output gate before the existing publication/transaction owner; prove batch failure publishes nothing and preserve owner-scoped delivery/cursor contracts.
9. **1000:** SDK exports and isolated renamed-dependency consumer proofs after leaf contracts; macro hygiene uses existing narrow support paths.
10. **1001:** examples and one SDK-only end-to-end workflow release gate after 1000 and production wiring, including typed poll/pushed-event outputs, protected-trigger admission and invalid-later-item batch rejection.

The outer release gate must author a typed workflow, admit its catalog and values,
choose a conditional credential/resource branch, run behavior with typed data,
and check outputs plus negative/default/secret/rotation cases using controlled
fixtures. Existing compile fixtures alone are not that execution proof.
It must depend on SDK only for Nebula APIs; test support must not expose raw
stores, admission capabilities or tenant proofs to make it pass.

Observed prior review found P1 receiver duplication, split impl ownership, serde/
default drift, guard lifetime and lazy/conditional ambiguities, visibility policy
coupling, and P2 alias/newtype/generic gaps. The sections above address those
objections as design decisions; no implementation fix or passing test is claimed.
The former attributed approvals are withdrawn, not carried forward as sign-off.
Coordinator focused document-review verdict: COMPLETE for the trigger-output
contract, including checked public TriggerHandler routing and bounded batch
staging. This is design review only; runtime targets remain unshipped and
implementation acceptance is outstanding.
Previously reported existing-code checks passed fmt, clippy, 8542 nextest tests
(3 skipped), doctests and deny with configuration warnings. These runtime checks
were not rerun for this amendment and do not exercise its targets.
Implementation acceptance and the outer workflow remain release requirements,
not behavior proved by those existing-code checks.

## Source Rationale

Official sources checked by the coordinator on 2026-09-12:

- [Rust procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html): derive scope and adjacent generated items.
- [Serde field attributes](https://serde.rs/field-attrs.html): authoritative rename, alias and default behavior.
- [JSON Schema annotations](https://json-schema.org/understanding-json-schema/reference/annotations): default annotation versus input materialization.
- [JSON Schema conditionals](https://json-schema.org/understanding-json-schema/reference/conditionals): explicit conditional constraints.
- [Rust panic recovery](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html): aborting panics cannot be caught; hooks run before an unwind is caught.
- [OnceLock initialization](https://doc.rust-lang.org/std/sync/struct.OnceLock.html#method.get_or_init): nested construction must avoid reentrant initialization.
