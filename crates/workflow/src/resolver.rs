//! Schema resolver trait for TypeDAG per-edge type-checking (ADR-0100 T3).
//!
//! `nebula-workflow` cannot depend on `nebula-action` (the `action → workflow`
//! edge already exists; the reverse would be a cycle). Instead the workflow
//! validator accepts a `NodeSchemaResolver` injected by the layer that owns
//! the catalog (`nebula-api`). The registry-free seam mirrors the
//! `DefinitionRoutingResolver` precedent from ADR-0095 D1.

use nebula_core::ActionKey;
use nebula_schema::{InputSchema, OutputSchema};
use semver::Version;

/// Input and output schemas for one workflow node, resolved from the action catalog.
///
/// Direction-typed (ADR-0100 C15): the input is an [`InputSchema`] (consumer)
/// and the output an [`OutputSchema`] (producer), both cheap `Arc`-backed
/// newtypes over `ValidSchema`, so the per-edge check
/// `is_assignable_schema(&producer.output, &consumer.input)` cannot transpose
/// the two. An empty/`Any` schema on either side still acts as the gradual
/// escape (ADR-0100 L2): an untyped node is compatible with any neighbour.
#[derive(Debug, Clone)]
pub struct NodeIoSchemas {
    /// The schema describing this node's input (what it consumes).
    pub input: InputSchema,
    /// The schema describing this node's output (what it produces).
    pub output: OutputSchema,
}

/// Resolver that maps a workflow node's action identity to its I/O schemas.
///
/// Implemented by the catalog-owning layer (`nebula-api`'s
/// `CatalogSchemaResolver`) and by stub impls in tests. The workflow crate
/// defines only the contract; it never knows about `ActionRegistry`.
///
/// ## Object safety
///
/// The trait is object-safe: `io_schemas` takes `&self` plus two reference
/// parameters with no generics. Callers pass `&dyn NodeSchemaResolver`.
///
/// ## Fail-open contract (T3.2)
///
/// `io_schemas` returns `None` when the catalog is absent or when the
/// `action_key` is not registered. The per-edge check **skips** any edge
/// for which either endpoint returns `None`, treating it as `Any`-typed
/// (ADR-0100 T3.2 / L2 gradual-typing escape). This means:
/// - A registry-less environment (`action_registry = None`) silently passes
///   all edges — identical to today's structural-only behaviour.
/// - An unknown `action_key` is already caught by the structural validator
///   (`WorkflowError::InvalidActionKey`); fail-open here is safe.
pub trait NodeSchemaResolver: Send + Sync {
    /// Resolve the input and output schemas for a node identified by
    /// `action_key` and an optional pinned `interface_version`.
    ///
    /// Returns `None` when the catalog cannot resolve the action (absent
    /// registry, unregistered key, or unknown version). An `None` result
    /// causes the calling per-edge check to skip that edge (fail-open,
    /// ADR-0100 T3.2).
    fn io_schemas(
        &self,
        action_key: &ActionKey,
        interface_version: Option<&Version>,
    ) -> Option<NodeIoSchemas>;
}

#[cfg(test)]
mod tests {
    use nebula_schema::{Field, FieldKey, Schema, ValidSchema};

    use super::*;

    /// Build a `ValidSchema` with a single string field.
    fn schema_with_field(key: &str, required: bool) -> ValidSchema {
        let fk = FieldKey::new(key).unwrap();
        let field = if required {
            Field::string(fk).required()
        } else {
            Field::string(fk)
        };
        Schema::builder().add(field).build().unwrap()
    }

    struct StubResolver {
        input: InputSchema,
        output: OutputSchema,
    }

    impl NodeSchemaResolver for StubResolver {
        fn io_schemas(
            &self,
            _action_key: &ActionKey,
            _interface_version: Option<&Version>,
        ) -> Option<NodeIoSchemas> {
            Some(NodeIoSchemas {
                input: self.input.clone(),
                output: self.output.clone(),
            })
        }
    }

    #[test]
    fn node_io_schemas_clones_cheaply() {
        let input = schema_with_field("x", true);
        let output = schema_with_field("y", false);
        let schemas = NodeIoSchemas {
            input: InputSchema::new(input.clone()),
            output: OutputSchema::new(output.clone()),
        };
        // Verify the Arc-backed cheap-clone invariant holds (through the newtype).
        assert!(schemas.input.as_schema().ptr_eq(&input));
        assert!(schemas.output.as_schema().ptr_eq(&output));
    }

    #[test]
    fn stub_resolver_returns_expected_schemas() {
        let action_key = ActionKey::new("test.action").unwrap();
        let input = schema_with_field("in_field", true);
        let output = schema_with_field("out_field", false);
        let resolver = StubResolver {
            input: InputSchema::new(input.clone()),
            output: OutputSchema::new(output.clone()),
        };
        let result = resolver.io_schemas(&action_key, None).unwrap();
        assert!(result.input.as_schema().ptr_eq(&input));
        assert!(result.output.as_schema().ptr_eq(&output));
    }
}
