## Archived Ideas (from `docs/archive/architecture-v2.md`)

### Extended parameter kinds (design draft)

Early draft included additional kinds that are not finalized in current API:

- `Schema(SchemaParameter)`
- `Formula(FormulaParameter)`
- `Dynamic(DynamicParameter)` with async resolver
- `Reference(ReferenceParameter)` for node/resource/credential/variable links
- `Template(TemplateParameter)`

```rust
#[async_trait]
pub trait ParameterResolver: Send + Sync {
    async fn resolve(
        &self,
        context: &ResolutionContext,
    ) -> Result<ParameterType, Error>;
}
```

### Reference constraints concept

- `ReferenceType::Node { node_type }`
- `ReferenceType::Resource { resource_type }`
- `ReferenceType::Credential { credential_type }`
- `ReferenceType::Variable { scope }`

This remains useful as a backlog direction for richer cross-node authoring UX.

