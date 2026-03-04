# API

## Public Surface

- **Stable:** Derive macros Action, Resource, Plugin, Credential, Parameters, Validator, Config and their container/field attributes. Documented in crate rustdoc. Patch/minor: additive attributes only; no change to generated signatures or required attributes.
- **Experimental:** None; all public derives are part of the authoring contract.
- **Hidden/internal:** support, types modules; expansion implementation details.

## Usage Patterns

- **Action:** #[derive(Action)] #[action(key="...", name="...", description="...", optional: credential, resource, parameters)] on unit struct. See rustdoc for full attribute list.
- **Resource/Plugin/Credential:** #[derive(Resource)] #[resource(...)]; #[derive(Plugin)] with metadata; #[derive(Credential)] with key and state. See rustdoc.
- **Parameters:** #[derive(Parameters)] with #[param(...)] on fields. Generates parameter definitions for action metadata.
- **Config/Validator:** #[derive(Config)] for env-loaded config; #[derive(Validator)] for field validation.

## Minimal Example

See crate lib.rs and rustdoc. Example: #[derive(Action)] #[action(key="http.request", name="HTTP Request", description="...")] pub struct HttpRequestAction;

## Error Semantics

- **Compile errors:** Invalid or missing required attribute (e.g. missing key or name) produces compile_error! or syn::Error; author fixes and recompiles. No runtime errors (macros are compile-time).
- **Compatibility:** If action/plugin/credential/resource crate changes trait and macro is not updated, author gets compile error at use site; we document compatible versions in MIGRATION/README.

## Compatibility Rules

- **Major bump:** Breaking change to attribute set (removal, behavior change) or to generated code (signature change). MIGRATION.md required; authors must update attributes or code.
- **Minor:** Additive attributes; backward-compatible output (e.g. new optional field in generated impl). No removal.
- **Deprecation:** Deprecated attribute gets at least one minor version with deprecation notice before removal (major).
