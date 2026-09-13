#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "borrowed admitted views are reserved for iterative runtime traversal"
    )
)]

use super::{
    DefinitionKey,
    admission::{AdmittedGraph, AdmittedSchemaGraph},
    model::{AdditionalProperties, Body, DefinitionIndex, UseSiteCore},
};
use crate::FieldKey;

pub(crate) struct GraphView<'a> {
    graph: &'a AdmittedGraph,
}

impl AdmittedSchemaGraph {
    pub(crate) fn view(&self) -> GraphView<'_> {
        GraphView { graph: &self.0 }
    }
}

impl<'a> GraphView<'a> {
    pub(crate) fn root(&self) -> UseView<'a> {
        UseView {
            graph: self.graph,
            core: &self.graph.graph.root.0,
            role: UseRole::Root,
        }
    }

    pub(crate) fn definitions(&self) -> impl ExactSizeIterator<Item = DefinitionView<'a>> + 'a {
        (0..self.graph.graph.definitions.len()).map(|position| DefinitionView {
            graph: self.graph,
            index: DefinitionIndex(position),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DefinitionView<'a> {
    graph: &'a AdmittedGraph,
    index: DefinitionIndex,
}

impl<'a> DefinitionView<'a> {
    pub(crate) fn canonical_id(&self) -> u32 {
        self.graph.canonical_numbers[self.index.0]
    }

    pub(crate) fn key(&self) -> &'a DefinitionKey {
        &self.graph.graph.definitions[self.index.0].key
    }

    pub(crate) fn body(&self) -> BodyView {
        match &self.graph.graph.definitions[self.index.0].body {
            Body::Any => BodyView::Any,
            Body::Null => BodyView::Null,
            Body::Boolean { .. } => BodyView::Boolean,
            Body::Integer(_) => BodyView::Integer,
            Body::Number(_) => BodyView::Number,
            Body::String { .. } => BodyView::String,
            Body::Bytes => BodyView::Bytes,
            Body::Record {
                additional_properties,
                ..
            } => BodyView::Record {
                additional_properties: match additional_properties {
                    AdditionalProperties::Open => AdditionalPropertiesView::Open,
                    AdditionalProperties::Closed => AdditionalPropertiesView::Closed,
                    AdditionalProperties::Typed(_) => AdditionalPropertiesView::Typed,
                },
            },
            Body::Array(_) => BodyView::Array,
            Body::Union(_) => BodyView::Union,
            Body::Alias(_) => BodyView::Alias,
        }
    }

    pub(crate) fn uses(&self) -> Vec<UseView<'a>> {
        let definition = &self.graph.graph.definitions[self.index.0];
        match &definition.body {
            Body::Record {
                properties,
                additional_properties,
                ..
            } => {
                let mut uses = Vec::with_capacity(
                    properties.len()
                        + usize::from(matches!(
                            additional_properties,
                            AdditionalProperties::Typed(_)
                        )),
                );
                uses.extend(properties.iter().map(|property| UseView {
                    graph: self.graph,
                    core: &property.core,
                    role: UseRole::Property(&property.key),
                }));
                if let AdditionalProperties::Typed(core) = additional_properties {
                    uses.push(UseView {
                        graph: self.graph,
                        core,
                        role: UseRole::AdditionalProperty,
                    });
                }
                uses
            },
            Body::Array(array) => vec![UseView {
                graph: self.graph,
                core: &array.element.0,
                role: UseRole::Element,
            }],
            Body::Union(union) => union
                .variants
                .iter()
                .filter_map(|variant| {
                    variant.payload.as_ref().map(|payload| UseView {
                        graph: self.graph,
                        core: &payload.0,
                        role: UseRole::VariantPayload(&variant.key),
                    })
                })
                .collect(),
            Body::Alias(alias) => vec![UseView {
                graph: self.graph,
                core: &alias.0,
                role: UseRole::Alias,
            }],
            Body::Any
            | Body::Null
            | Body::Boolean { .. }
            | Body::Integer(_)
            | Body::Number(_)
            | Body::String { .. }
            | Body::Bytes => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyView {
    Any,
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Bytes,
    Record {
        additional_properties: AdditionalPropertiesView,
    },
    Array,
    Union,
    Alias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdditionalPropertiesView {
    Open,
    Closed,
    Typed,
}

#[derive(Clone, Copy)]
pub(crate) struct UseView<'a> {
    graph: &'a AdmittedGraph,
    core: &'a UseSiteCore,
    role: UseRole<'a>,
}

impl<'a> UseView<'a> {
    pub(crate) const fn role(&self) -> UseRole<'a> {
        self.role
    }

    pub(crate) fn target(&self) -> Option<DefinitionView<'a>> {
        let index = *self.graph.lookup.get(&self.core.target)?;
        Some(DefinitionView {
            graph: self.graph,
            index,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UseRole<'a> {
    Root,
    Alias,
    Property(&'a FieldKey),
    AdditionalProperty,
    Element,
    VariantPayload(&'a FieldKey),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::SchemaGraphDocument;

    fn admitted() -> AdmittedSchemaGraph {
        serde_json::from_value::<SchemaGraphDocument>(json!({
            "version": 3,
            "root": {"target":"root"},
            "definitions": [
                {"key":"root","body":{"kind":"record","properties":[
                    {"key":"items","target":"items"}
                ],"additional_properties":{"typed":{"target":"text"}}}},
                {"key":"items","body":{"kind":"array","element":{"target":"root"}}},
                {"key":"text","body":{"kind":"string"}}
            ]
        }))
        .expect("view fixture decodes")
        .admit()
        .expect("view fixture admits")
    }

    #[test]
    fn views_keep_canonical_ids_targets_and_roles_consistent() {
        let graph = admitted();
        let view = graph.view();
        assert_eq!(view.root().role(), UseRole::Root);
        assert_eq!(
            view.root()
                .target()
                .expect("root target resolves")
                .canonical_id(),
            0
        );

        let definitions = view.definitions().collect::<Vec<_>>();
        assert_eq!(definitions.len(), graph.definition_count());
        let ids = definitions
            .iter()
            .map(DefinitionView::canonical_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids, BTreeSet::from([0, 1, 2]));

        let root = definitions
            .iter()
            .find(|definition| definition.key().as_str() == "root")
            .expect("root definition is present");
        assert_eq!(
            root.body(),
            BodyView::Record {
                additional_properties: AdditionalPropertiesView::Typed
            }
        );
        let roles = root
            .uses()
            .into_iter()
            .map(|use_site| use_site.role())
            .collect::<Vec<_>>();
        assert_eq!(
            roles[0],
            UseRole::Property(&FieldKey::new("items").unwrap())
        );
        assert_eq!(roles[1], UseRole::AdditionalProperty);
    }

    #[test]
    fn caller_worklist_terminates_on_cycles_without_internal_indices() {
        let graph = admitted();
        let mut pending = vec![graph.view().root().target().expect("root target resolves")];
        let mut visited = BTreeSet::new();
        while let Some(definition) = pending.pop() {
            if !visited.insert(definition.canonical_id()) {
                continue;
            }
            pending.extend(
                definition
                    .uses()
                    .into_iter()
                    .filter_map(|use_site| use_site.target()),
            );
        }
        assert_eq!(visited, BTreeSet::from([0, 1, 2]));
    }
}
