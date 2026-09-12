---
name: Unified property authoring
status: accepted
last-reviewed: 2026-09-12
related:
  - ../../../docs/INTEGRATION_MODEL.md
  - DESIGN.md
  - ../../action/docs/DESIGN.md
  - ../../credential/docs/DESIGN.md
  - ../../resource/docs/DESIGN.md
  - ../../engine/docs/DESIGN.md
---

# Unified property authoring

This spec ratifies ADR-0108 against the September 2026 tree. It is the
implementation contract for one declarative authoring shape across schema-only
types, actions, credentials, and resources.

The decision is intentionally breaking: new derives use one field-level
`#[property(...)]` grammar for author-owned values and typed dependency slots.
Concept-level attributes (`#[action(...)]`, `#[credential(...)]`,
`#[resource(...)]`) still describe the catalog leaf. Hand-written behavior
stays hand-written.

## Design Delta From ADR-0108

ADR-0108 chose one author struct per abstraction and rejected hidden companion
input structs. That remains accepted. The current tree and issue 992 use the
name `#[property]`; this spec therefore renames ADR-0108's value-field helper
from `#[field]` to `#[property]` for the new surface.

ADR-0101's engine-owned `Slots` value remains deferred. The Phase-5 derives
continue to generate the existing `FromWorkflowNode` consumer seam until a
separate engine design reopens that boundary.

## Authoring Model

Every integration author writes one Rust type per concept:

| Concept | Author type | Value fields | Slot fields | Hand-written behavior |
|---|---|---|---|---|
| Schema-only data | `#[derive(Schema)]` | `#[property(...)]` | none | none |
| Action | `#[derive(Action, Schema, Deserialize)]` | `#[property(...)]` | `#[property(credential, ...)]`, `#[property(resource, ...)]` | `impl StatelessAction` / `StatefulAction` / other action trait |
| Credential | `#[derive(Credential, Schema, Deserialize)]` | `#[property(...)]` for `Properties = Self` | none | resolve/project/capability methods |
| Resource | `#[derive(Resource, Schema, Deserialize)]` | `#[property(...)]` for `Config = Self` | `#[property(credential, ...)]` | `impl Provider` lifecycle |

For action, credential, and resource derives, the generated associated property
type is `Self` unless the implementation explicitly opts out with
`properties = ExternalType`, `input = ExternalType`, or `config = ExternalType`.
Opt-out is for staged migration only during the breaking implementation PRs; the
stable authored form is `Self`.

## Attribute Grammar

`#[property(...)]` has three mutually exclusive modes. Unknown keys are compile
errors at the offending token.

### Value Property

Used on ordinary parameter/config/property fields:

```rust
#[property(
    key = "optional_wire_name",
    label = "Display label",
    description = "Longer help text",
    placeholder = "shown in forms",
    default = "literal",
    hint = "short hint",
    secret,
    multiline,
    no_expression,
    expression_required,
    enum_select,
    group = "advanced",
    emit_as = "output_wire_name",
    validate(required, length(min = 1, max = 100), pattern = "^[a-z]+$")
)]
field: String,
```

Allowed value keys:

| Key | Meaning |
|---|---|
| `key = "..."` | Override input/schema key; otherwise serde rename rules, then field ident. |
| `label`, `description`, `placeholder`, `hint`, `group` | UI/catalog metadata. |
| `default = ...` | String, integer, float, or bool literal default. |
| `secret` | Promote string leaves to protected schema secrets; target type must implement `SecretInput`. |
| `multiline` | UI string widget hint. |
| `no_expression` | Forbid expression authoring at this exact schema node. |
| `expression_required` | Require expression authoring at this exact schema node. |
| `enum_select` | Use `HasSelectOptions` for a select field. |
| `emit_as = "..."` | Output projection key. |
| `validate(...)` | Rules: `required`, `length(min,max)`, `range(min,max)`, `pattern`, `url`, `email`. |
| `skip` | Exclude the field from the schema and fingerprint. |
| `skip_fingerprint` | Exclude resource config field from `ContentId` / hot-reload fingerprint only. |

`#[validate(...)]`, `#[field(...)]`, and `#[config(skip_fingerprint)]` are
replaced by this grammar in Phase-5 derives. The lower schema crate may keep
temporary parser support only inside the migration PR; no stable docs should
teach both surfaces.

### Credential Slot

Used on action or resource fields that receive projected auth material:

```rust
#[property(credential, key = "slack_auth", purpose = "Slack API auth")]
slack: CredentialSlot<SlackCredential>,
```

Allowed keys:

| Key | Meaning |
|---|---|
| `credential` | Select credential-slot mode. |
| `key = "..."` | Slot key; defaults to field ident. |
| `purpose = "..."` | Catalog/UI reason for the binding. |
| `optional` | Missing binding is accepted and the accessor yields `None`. |
| `lazy` | Preserve lazy acquisition semantics where the owning crate supports it. |

Accepted field shapes are `CredentialSlot<C>` and
`Option<CredentialSlot<C>>`. The slot resolves to `CredentialGuard<C::Scheme>`;
authors never receive stored credential state or storage authority.

### Resource Slot

Used on action fields that receive managed resources:

```rust
#[property(resource, key = "http_client", purpose = "Outbound HTTP")]
http: ResourceSlot<HttpClient>,
```

Allowed keys:

| Key | Meaning |
|---|---|
| `resource` | Select resource-slot mode. |
| `key = "..."` | Slot key; defaults to field ident. |
| `purpose = "..."` | Catalog/UI reason for the binding. |
| `optional` | Missing binding is accepted and the accessor yields `None`. |
| `lazy` | Preserve lazy acquisition semantics where the owning crate supports it. |

Accepted field shapes are `ResourceSlot<R>` and `Option<ResourceSlot<R>>`.

## Schema and JSON Schema

Slots do not become schema fields. They are `#[serde(skip)]`, excluded from
`HasSchema`, and excluded from authored value persistence. `slot_bindings`
remain outside `parameters` and outside credential/resource config values.

JSON Schema export for `ValidSchema` therefore does not include slots and does
not need a schema-wire bump for slots alone. Integration catalog export may place
slot declarations beside the value schema under `x-nebula-slots`:

```json
{
  "type": "object",
  "properties": {
    "channel": {"type": "string"}
  },
  "x-nebula-slots": [
    {"kind": "credential", "key": "slack_auth", "required": true},
    {"kind": "resource", "key": "http_client", "required": false}
  ]
}
```

`SCHEMA_WIRE_VERSION` changes only if `ValidSchema` itself changes its durable
definition format. Adding catalog-level `x-nebula-*` ornaments outside
`ValidSchema` is an integration-catalog version change, not a schema-definition
wire change.

## Generated Items

The derives generate these items together from the same parsed property model:

| Derive | Generated schema | Generated slots | Generated metadata |
|---|---|---|---|
| `Schema` | `HasSchema` from value properties only | none | none |
| `Action` | `type Input = Self` unless external; `type Output` remains explicit | `DeclaresDependencies`, `FromWorkflowNode`, slot accessors | `ActionMetadataDraft` from `#[action(...)]`, with input/output schemas admitted by factory |
| `Credential` | `type Properties = Self` unless external | none | `CredentialMetadataDraft` from `#[credential(...)]`, with properties schema admitted by registry |
| `Resource` | `type Config = Self` unless external | `DeclaresDependencies`, `HasCredentialSlots`, slot accessors | `ResourceMetadataDraft` from `#[resource(...)]`, with config schema admitted by factory |

The macro expansion path for downstream SDK-only crates must resolve through
`nebula_sdk::__private`, not direct leaf-crate names. Leaf crates may still
expand through their own canonical crate names internally.

## Compile-Fail Contracts

The implementation must add compile-fail tests for:

| Contract | Required diagnostic |
|---|---|
| Unknown property key | Names the unknown key and the valid keys for that mode. |
| Two modes on one field | Rejects `#[property(credential, resource)]` and value keys mixed with slot modes. |
| Slot type mismatch | Names accepted `CredentialSlot<C>` / `ResourceSlot<R>` shapes. |
| Slot serialized as data | Rejects slot field missing `serde(skip)` in generated expansion or direct serde field inclusion. |
| Secret destination not safe | `#[property(secret)]` requires `SecretInput` for the decoded destination. |
| Unsupported slot in schema-only type | `#[property(credential)]` and `#[property(resource)]` are invalid under schema-only `#[derive(Schema)]`. |
| Duplicate slot key | Points to both fields. |
| Duplicate value key / alias collision | Reuses the existing schema collision report. |
| Unknown concept-level key | `#[action]`, `#[credential]`, and `#[resource]` reject typos at token span. |
| SDK-only path resolution | A crate depending only on `nebula-sdk` can use the derives. |

Secrets must not appear in `Debug`, compile errors generated from user input,
or runtime validation reports. Slot guards must not be `Clone` unless the guard
type already explicitly permits that semantic.

## Migration

### Schema-Only Struct

Before:

```rust
#[derive(Schema, Deserialize)]
struct SearchInput {
    #[field(label = "Query")]
    #[validate(required, length(min = 1, max = 256))]
    query: String,
}
```

After:

```rust
#[derive(Schema, Deserialize)]
struct SearchInput {
    #[property(label = "Query", validate(required, length(min = 1, max = 256)))]
    query: String,
}
```

Existing compile coverage: `crates/schema/tests/derive_schema.rs` and
`crates/schema/tests/compile_fail.rs`.

### Action

Before:

```rust
#[derive(Action)]
#[action(key = "slack.send", name = "Send Slack", input = SendInput, output = SendOutput)]
struct SendSlack {
    #[credential(key = "slack_auth")]
    auth: CredentialGuard<SlackScheme>,
}
```

After:

```rust
#[derive(Action, Schema, Deserialize)]
#[action(key = "slack.send", name = "Send Slack", output = SendOutput)]
struct SendSlack {
    #[property(label = "Channel", validate(required))]
    channel: String,

    #[property(credential, key = "slack_auth", purpose = "Slack API auth")]
    #[serde(skip)]
    auth: CredentialSlot<SlackCredential>,
}
```

Existing compile coverage: `crates/action/tests/derive_action.rs` and
`crates/action/tests/derive_action_compile_fail.rs`.

### Credential

Before:

```rust
#[derive(Schema, Deserialize)]
struct ApiKeyProperties {
    #[field(secret)]
    key: SecretString,
}

#[credential(key = "api_key", name = "API Key")]
impl ApiKeyCredential {
    type Properties = ApiKeyProperties;
    type Scheme = SecretToken;
    type State = SecretToken;
}
```

After:

```rust
#[derive(Credential, Schema, Deserialize)]
#[credential(key = "api_key", name = "API Key", scheme = SecretToken, state = SecretToken)]
struct ApiKeyCredential {
    #[property(secret, validate(required))]
    key: SecretString,
}
```

Resolve/project behavior remains hand-written or generated from explicitly
recognized methods; capability membership is still inferred from method
presence, not from flags.

Existing compile coverage: credential compile-fail tests under
`crates/credential/tests/compile_fail_*.rs`.

### Resource

Before:

```rust
#[derive(Resource)]
struct Postgres {
    #[credential(key = "db_auth")]
    auth: CredentialSlot<PostgresCredential>,
}

#[derive(ResourceConfig, Schema, Deserialize)]
#[config(schema = external)]
struct PostgresConfig {
    #[field(label = "URL")]
    url: String,
}
```

After:

```rust
#[derive(Resource, Schema, Deserialize)]
#[resource(key = "postgres", topology = Pooled)]
struct Postgres {
    #[property(label = "URL", validate(required))]
    url: String,

    #[property(credential, key = "db_auth", purpose = "Database auth")]
    #[serde(skip)]
    auth: CredentialSlot<PostgresCredential>,
}
```

Provider lifecycle remains hand-written. `ResourceConfig: Clone` is dropped in
favor of schema-bound config identity and `ContentId` / fingerprint evidence.

Existing compile coverage: `crates/resource/tests/resource_config_derive.rs`,
`crates/resource/tests/derive_resource_compile_fail.rs`, and SDK
`derive_external_contract`.

## Programmatic Metadata Builders

The unified derive grammar does not subsume the programmatic metadata builders.
`ActionMetadataDraft`, `CredentialMetadataDraft`, and `ResourceMetadataDraft`
remain the programmatic authoring surface for factories, registries, tests, and
manual integration code. Issue 1018 must still make those three draft builders
symmetric. That issue should delegate icon setters to `nebula-metadata::Icon`
and remove any author-facing `icon`/`icon_url` pair.

The derives should emit those draft builders only through the symmetric surface
once issue 1018 lands.

## Non-Goals

- Do not serialize schema proof tokens or typed guards.
- Do not move slot bindings into `parameters`, credential properties, or resource config values.
- Do not implement ADR-0101's engine-owned `Slots` value in this change.
- Do not add process, WASM, or dynamic plugin isolation.
- Do not expose registries, stores, tenant authority, or admitted metadata as SDK authoring APIs.
- Do not keep stable duplicate attribute surfaces after migration.

## Maintainer Review Notes

API-design review: acceptable with one mandatory constraint: the value schema
and slot graph must stay separate. A single author struct is a DX improvement
only if `HasSchema` continues to describe values and dependency slots remain
catalog/activation declarations.

Macro-specialist review: acceptable with two constraints. First, parse into one
intermediate property model shared by the four derives instead of four local
parsers. Second, compile-fail diagnostics are part of the contract and must pin
unknown keys, mode conflicts, and SDK path resolution.
