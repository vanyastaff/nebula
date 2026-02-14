#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Ports
//!
//! Backend interface traits (ports) for the Nebula workflow engine.
//!
//! This crate defines the **port** traits that backend drivers implement.
//! It follows the Ports & Drivers (hexagonal) architecture pattern:
//!
//! - [`WorkflowRepo`] -- persistence for workflow definitions
//! - [`ExecutionRepo`] -- execution state, journals, and leases
//! - [`TaskQueue`] -- work distribution queue
//! - [`SandboxRunner`] -- isolated action execution
//!
//! All traits are `async_trait` and object-safe, suitable for use as
//! `Box<dyn Trait>` or `Arc<dyn Trait>` behind dependency injection.

pub mod error;
pub mod execution;
pub mod queue;
pub mod sandbox;
pub mod workflow;

pub use error::PortsError;
pub use execution::ExecutionRepo;
pub use queue::TaskQueue;
pub use sandbox::SandboxRunner;
pub use workflow::WorkflowRepo;

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify all four traits are object-safe by constructing trait object types.
    /// This is a compile-time test -- if it compiles, the traits are object-safe.
    #[test]
    fn traits_are_object_safe() {
        fn _assert_workflow_repo(_: &dyn WorkflowRepo) {}
        fn _assert_execution_repo(_: &dyn ExecutionRepo) {}
        fn _assert_task_queue(_: &dyn TaskQueue) {}
        fn _assert_sandbox_runner(_: &dyn SandboxRunner) {}
    }

    /// Verify traits can be used as `Box<dyn Trait>` (the common DI pattern).
    /// Another compile-time test.
    #[test]
    fn traits_work_as_boxed_dyn() {
        fn _takes_workflow(_: Box<dyn WorkflowRepo>) {}
        fn _takes_execution(_: Box<dyn ExecutionRepo>) {}
        fn _takes_queue(_: Box<dyn TaskQueue>) {}
        fn _takes_sandbox(_: Box<dyn SandboxRunner>) {}
    }

    /// Verify traits can be wrapped in `Arc` for shared ownership.
    #[test]
    fn traits_work_as_arc_dyn() {
        use std::sync::Arc;
        fn _takes_workflow(_: Arc<dyn WorkflowRepo>) {}
        fn _takes_execution(_: Arc<dyn ExecutionRepo>) {}
        fn _takes_queue(_: Arc<dyn TaskQueue>) {}
        fn _takes_sandbox(_: Arc<dyn SandboxRunner>) {}
    }
}
