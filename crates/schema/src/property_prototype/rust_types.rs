use std::{any::TypeId, collections::HashMap};

use super::graph::{DefinitionKey, GraphDiagnostic};

#[derive(Debug, Default)]
pub(super) struct RustTypeRegistry {
    definitions_by_type: HashMap<TypeId, DefinitionKey>,
    types_by_definition: HashMap<DefinitionKey, TypeId>,
}

impl RustTypeRegistry {
    pub(super) fn register<T: 'static>(
        &mut self,
        definition: DefinitionKey,
    ) -> Result<(), GraphDiagnostic> {
        let type_id = TypeId::of::<T>();
        if let Some(existing) = self.definitions_by_type.get(&type_id) {
            return if existing == &definition {
                Ok(())
            } else {
                Err(GraphDiagnostic::root("graph.rust_type_conflict"))
            };
        }
        if self.types_by_definition.contains_key(&definition) {
            return Err(GraphDiagnostic::root("graph.definition_key_conflict"));
        }

        self.definitions_by_type.insert(type_id, definition.clone());
        self.types_by_definition.insert(definition, type_id);
        Ok(())
    }

    pub(super) fn definition_for(&self, type_id: TypeId) -> Option<&DefinitionKey> {
        self.definitions_by_type.get(&type_id)
    }
}
