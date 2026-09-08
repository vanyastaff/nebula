//! Shared admission floor for the recorded graph semantics this runtime executes.

use nebula_plugin::ExecutableGraph;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RecordedGraphRejection {
    UnresolvedBindings,
    UnsupportedSemantics,
}

pub(crate) fn validate_recorded_graph(
    graph: &ExecutableGraph,
) -> Result<(), RecordedGraphRejection> {
    if !graph.bindings().is_empty() {
        return Err(RecordedGraphRejection::UnresolvedBindings);
    }
    if !graph.variables().is_empty()
        || !graph.config().checkpointing.enabled
        || graph.config().checkpointing.interval.is_some()
        || graph.nodes().iter().any(|node| node.timeout.is_some())
    {
        return Err(RecordedGraphRejection::UnsupportedSemantics);
    }
    Ok(())
}
