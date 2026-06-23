//! Catalog-backed schema resolver for TypeDAG per-edge validation (ADR-0100 T3.3).
//!
//! [`CatalogSchemaResolver`] implements the [`NodeSchemaResolver`] trait
//! defined in `nebula-workflow`, bridging the workflow validation seam to the
//! `ActionRegistry` owned by `nebula-engine`. The workflow crate never imports
//! `ActionRegistry` directly (that would close a cycle); this module lives in
//! `nebula-api`, which already depends on both crates.

use std::sync::Arc;

use nebula_engine::ActionRegistry;
use nebula_workflow::{NodeIoSchemas, NodeSchemaResolver};
use semver::Version;

/// Resolves a workflow node's I/O schemas from the in-process action catalog.
///
/// Wraps an `Option<Arc<ActionRegistry>>` — the same optional handle that
/// `AppState.action_registry` carries. When the registry is absent (`None`),
/// every `io_schemas` call returns `None`, causing all edges to be skipped
/// (fail-open, ADR-0100 T3.2). Behaviour is identical to the structural-only
/// `validate_workflow` path — no new hard errors, no new 422s.
///
/// # Versioned lookup
///
/// When the node carries a pinned `interface_version`, the resolver calls
/// [`ActionRegistry::get_factory_versioned`]. When no version is pinned
/// (`interface_version = None`), it falls back to
/// [`ActionRegistry::get_factory`] (latest registered version).
///
/// # Fail-open
///
/// An unregistered `action_key` or an unknown version returns `None`, causing
/// the calling edge to be skipped. Unknown keys are already caught by the
/// structural validator (`WorkflowError::InvalidActionKey`) before the schema
/// check runs, so fail-open here is safe.
pub struct CatalogSchemaResolver {
    registry: Option<Arc<ActionRegistry>>,
}

impl CatalogSchemaResolver {
    /// Construct a resolver from the optional catalog handle in `AppState`.
    ///
    /// Pass `state.action_registry.clone()` directly — the `Option<Arc<_>>`
    /// is cloned cheaply (one `Arc` increment or a `None` copy).
    #[must_use]
    pub fn new(registry: Option<Arc<ActionRegistry>>) -> Self {
        Self { registry }
    }
}

impl NodeSchemaResolver for CatalogSchemaResolver {
    /// Resolve the input and output schemas for `action_key`.
    ///
    /// Returns `None` when:
    /// - `self.registry` is `None` (catalog absent), or
    /// - `action_key` is not registered, or
    /// - `interface_version` is `Some(v)` and no factory with that exact
    ///   version is registered.
    fn io_schemas(
        &self,
        action_key: &nebula_core::ActionKey,
        interface_version: Option<&Version>,
    ) -> Option<NodeIoSchemas> {
        let registry = self.registry.as_ref()?;

        let (metadata, _factory) = if let Some(version) = interface_version {
            registry.get_factory_versioned(action_key, version)?
        } else {
            registry.get_factory(action_key)?
        };

        Some(NodeIoSchemas {
            // Tag the catalog schemas with their dataflow polarity (ADR-0100 C15):
            // a node's declared input is the consumer side, its output the producer.
            input: metadata.base.schema.clone().into(),
            output: metadata.output_schema.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use nebula_action::{
        Action, ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction,
    };
    use nebula_core::{ActionKey, Dependencies};
    use nebula_engine::ActionRegistry;
    use nebula_schema::{Field, FieldKey, HasSchema, Schema, ValidSchema};
    use serde::{Deserialize, Serialize};

    use super::*;

    // ── TypedOut — a concrete output type with a known, non-empty HasSchema ──
    //
    // `InstanceFactory<A>` unconditionally stamps `output_schema` from
    // `<A::Output as HasSchema>::schema()` (single-writer invariant, T2).
    // To make `registry_hit_returns_both_schemas` non-vacuous we need a type
    // whose `HasSchema` impl returns a real, distinguishable schema — not the
    // empty schema that `serde_json::Value` produces.

    #[derive(Debug, Serialize, Deserialize)]
    struct TypedOut {
        out_field: String,
    }

    impl HasSchema for TypedOut {
        fn schema() -> ValidSchema {
            Schema::builder()
                .add(Field::string(FieldKey::new("out_field").unwrap()))
                .build()
                .expect("TypedOut schema is valid")
        }
    }

    /// A no-op stateless action with a typed, schema-carrying `Output`.
    ///
    /// `Action::metadata()` returns a placeholder key; the caller supplies the
    /// real key (and the input schema) via `register_stateless_instance`.
    /// `InstanceFactory` stamps `output_schema` from `TypedOut::schema()`.
    struct TypedNoop;

    impl Action for TypedNoop {
        type Input = serde_json::Value;
        type Output = TypedOut;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                ActionKey::new("test.__typed_noop__").unwrap(),
                "TypedNoop",
                "",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for TypedNoop {
        async fn execute(
            &self,
            _input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(TypedOut {
                out_field: String::new(),
            }))
        }
    }

    /// Build a registry with one `TypedNoop` action registered under `key`.
    ///
    /// The metadata carries `input_schema` on `base.schema` (caller-controlled).
    /// `InstanceFactory` stamps `output_schema` from `<TypedOut as HasSchema>::schema()`
    /// (single-writer, T2 invariant) — the caller does not supply an output schema.
    fn registry_with_typed_action(key: &str, input_schema: ValidSchema) -> ActionRegistry {
        let action_key = ActionKey::new(key).unwrap();
        let meta = ActionMetadata::new(action_key, "Test", "test")
            .with_version(1, 0)
            .with_schema(input_schema);

        let registry = ActionRegistry::new();
        registry.register_stateless_instance(meta, TypedNoop);
        registry
    }

    fn field_schema(key: &str, required: bool) -> ValidSchema {
        let fk = FieldKey::new(key).unwrap();
        let field = if required {
            Field::string(fk).required()
        } else {
            Field::string(fk)
        };
        Schema::builder().add(field).build().unwrap()
    }

    /// The resolver reads both fields from the stored metadata:
    /// - `input`  = caller-supplied `base.schema` (preserved by `InstanceFactory`)
    /// - `output` = type-stamped `<TypedOut as HasSchema>::schema()` (single-writer)
    ///
    /// Goes RED if `CatalogSchemaResolver::io_schemas` reads the wrong field or
    /// swaps input/output.
    #[test]
    fn registry_hit_returns_both_schemas() {
        let input = field_schema("in_field", true);
        let expected_output = TypedOut::schema();

        let registry = registry_with_typed_action("test.action", input.clone());

        let resolver = CatalogSchemaResolver::new(Some(Arc::new(registry)));
        let action_key = ActionKey::new("test.action").unwrap();
        let schemas = resolver.io_schemas(&action_key, None).unwrap();

        assert_eq!(
            schemas.input.as_schema(),
            &input,
            "input schema must match caller-supplied base.schema"
        );
        assert_eq!(
            schemas.output.as_schema(),
            &expected_output,
            "output schema must match TypedOut::schema() (type-stamped by InstanceFactory)"
        );
        // Non-vacuous: the output schema must be non-empty so the assertion is meaningful.
        assert!(
            !schemas.output.as_schema().fields().is_empty(),
            "TypedOut schema must be non-empty (test would be vacuous otherwise)"
        );
    }

    #[test]
    fn registry_miss_returns_none() {
        let registry = ActionRegistry::new(); // empty
        let resolver = CatalogSchemaResolver::new(Some(Arc::new(registry)));
        let key = ActionKey::new("missing.action").unwrap();
        assert!(
            resolver.io_schemas(&key, None).is_none(),
            "unregistered key must return None"
        );
    }

    #[test]
    fn absent_registry_returns_none() {
        let resolver = CatalogSchemaResolver::new(None);
        let key = ActionKey::new("any.action").unwrap();
        assert!(
            resolver.io_schemas(&key, None).is_none(),
            "None registry must return None"
        );
    }

    /// A pinned `interface_version` that matches the registered version resolves.
    ///
    /// `register_stateless_instance` stores `metadata.base.version` so that
    /// `get_factory_versioned` can match on it. This test exercises the
    /// `Some(version)` branch of `io_schemas`.
    ///
    /// Goes RED if `io_schemas` falls through to `get_factory` (ignoring the
    /// version) or returns `None` for a known-good pinned version.
    #[test]
    fn versioned_hit_returns_schemas() {
        let input = field_schema("in_field", true);
        // registry_with_typed_action registers at version 1.0 (via with_version(1,0)).
        let registry = registry_with_typed_action("test.versioned", input.clone());
        let resolver = CatalogSchemaResolver::new(Some(Arc::new(registry)));
        let key = ActionKey::new("test.versioned").unwrap();
        let v1 = Version::new(1, 0, 0);

        let schemas = resolver
            .io_schemas(&key, Some(&v1))
            .expect("version 1.0.0 is registered; io_schemas must return Some");

        assert_eq!(
            schemas.input.as_schema(),
            &input,
            "versioned hit: input schema must match caller-supplied base.schema"
        );
        assert_eq!(
            schemas.output.as_schema(),
            &TypedOut::schema(),
            "versioned hit: output schema must match TypedOut::schema()"
        );
    }

    /// A pinned `interface_version` that is NOT registered returns `None` (fail-open).
    ///
    /// `get_factory_versioned` returns `None` for an unknown version; the
    /// resolver must propagate that as `None` so the edge is skipped rather
    /// than producing a spurious schema error.
    ///
    /// Goes RED if `io_schemas` falls through to `get_factory` (returning the
    /// latest version) instead of returning `None` for the unregistered version.
    #[test]
    fn versioned_miss_returns_none() {
        let input = field_schema("in_field", true);
        let registry = registry_with_typed_action("test.versioned", input);
        let resolver = CatalogSchemaResolver::new(Some(Arc::new(registry)));
        let key = ActionKey::new("test.versioned").unwrap();
        // Version 2.0.0 was never registered.
        let v2 = Version::new(2, 0, 0);

        assert!(
            resolver.io_schemas(&key, Some(&v2)).is_none(),
            "unregistered version must return None (fail-open)"
        );
    }
}
