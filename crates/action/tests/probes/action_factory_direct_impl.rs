use std::{future::Future, pin::Pin, sync::Arc};

use nebula_action::{
    ActionContext, ActionError, ActionFactory, ActionHandle, ActionMetadata,
};
use nebula_core::Dependencies;
use nebula_workflow::NodeDefinition;

struct ForgedFactory;

impl ActionFactory for ForgedFactory {
    fn metadata(&self) -> &Arc<ActionMetadata> {
        panic!("compile-only probe")
    }

    fn dependencies(&self) -> &Dependencies {
        panic!("compile-only probe")
    }

    fn instantiate<'a>(
        &'a self,
        _node: &'a NodeDefinition,
        _context: &'a dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionHandle, ActionError>> + Send + 'a>> {
        panic!("compile-only probe")
    }
}

fn main() {}
