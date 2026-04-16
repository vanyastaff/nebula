//! Execution context for resource operations.
//!
//! [`Ctx`] provides cancellation, scope information, and an extensible type-map
//! for threading arbitrary data through the resource subsystem. [`BasicCtx`] is
//! the default concrete implementation.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

use nebula_core::{ExecutionId, WorkflowId};
use tokio_util::sync::CancellationToken;

/// Scope level for resource isolation.
///
/// Determines the lifecycle boundary of a resource instance. Finer scopes
/// (e.g., `Execution`) are cleaned up more aggressively than coarser ones
/// (e.g., `Global`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum ScopeLevel {
    /// Application-lifetime singleton.
    #[default]
    Global,
    /// Scoped to an organization.
    Organization(String),
    /// Scoped to a workspace (formerly "project").
    Workspace(String),
    /// Scoped to a workflow definition.
    Workflow(WorkflowId),
    /// Scoped to a single execution run.
    Execution(ExecutionId),
}

/// Execution context for resource operations.
///
/// Carries cancellation, scope, and arbitrary typed extensions. Every
/// resource lifecycle method receives a `&dyn Ctx`.
///
/// Use the free function [`ctx_ext`] to retrieve typed extensions from
/// a `&dyn Ctx`.
pub trait Ctx: Send + Sync {
    /// Returns the scope level for this operation.
    fn scope(&self) -> &ScopeLevel;

    /// Returns the execution ID for tracing / correlation.
    fn execution_id(&self) -> &ExecutionId;

    /// Returns the cancellation token for cooperative shutdown.
    fn cancel_token(&self) -> &CancellationToken;

    /// Returns the raw extension value for a given [`TypeId`], if present.
    ///
    /// Prefer the free function [`ctx_ext`] for a typed API.
    fn ext_raw(&self, type_id: TypeId) -> Option<&(dyn Any + Send + Sync)>;
}

/// Retrieves a typed extension from a context.
///
/// This is a convenience wrapper around [`Ctx::ext_raw`].
pub fn ctx_ext<T: Send + Sync + 'static>(ctx: &dyn Ctx) -> Option<&T> {
    ctx.ext_raw(TypeId::of::<T>())
        .and_then(|any| any.downcast_ref())
}

/// Type-map for arbitrary typed extensions.
///
/// Stores at most one value per concrete type, keyed by [`TypeId`].
#[derive(Debug, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Creates an empty extension map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value, replacing any previous value of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Retrieves a reference to a stored value by type.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Returns the raw `dyn Any` for a given `TypeId`.
    pub fn get_raw(&self, type_id: TypeId) -> Option<&(dyn Any + Send + Sync)> {
        self.map.get(&type_id).map(|b| b.as_ref())
    }
}

/// Basic implementation of [`Ctx`].
///
/// # Examples
///
/// ```
/// use nebula_core::ExecutionId;
/// use nebula_resource::ctx::{BasicCtx, Ctx, ScopeLevel};
///
/// let ctx = BasicCtx::new(ExecutionId::new());
/// assert_eq!(*ctx.scope(), ScopeLevel::Global);
/// ```
pub struct BasicCtx {
    scope: ScopeLevel,
    execution_id: ExecutionId,
    cancel: CancellationToken,
    extensions: Extensions,
}

impl BasicCtx {
    /// Creates a new context with the given execution ID and default scope.
    pub fn new(execution_id: ExecutionId) -> Self {
        Self {
            scope: ScopeLevel::Global,
            execution_id,
            cancel: CancellationToken::new(),
            extensions: Extensions::new(),
        }
    }

    /// Sets the scope level.
    pub fn with_scope(mut self, scope: ScopeLevel) -> Self {
        self.scope = scope;
        self
    }

    /// Sets the cancellation token.
    pub fn with_cancel_token(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    /// Sets the extensions map.
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }
}

impl std::fmt::Debug for BasicCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicCtx")
            .field("scope", &self.scope)
            .field("execution_id", &self.execution_id)
            .finish_non_exhaustive()
    }
}

impl Ctx for BasicCtx {
    fn scope(&self) -> &ScopeLevel {
        &self.scope
    }

    fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    fn ext_raw(&self, type_id: TypeId) -> Option<&(dyn Any + Send + Sync)> {
        self.extensions.get_raw(type_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ctx_default_scope_is_global() {
        let ctx = BasicCtx::new(ExecutionId::new());
        assert_eq!(*ctx.scope(), ScopeLevel::Global);
    }

    #[test]
    fn basic_ctx_with_scope() {
        let exec_id = ExecutionId::new();
        let ctx = BasicCtx::new(exec_id).with_scope(ScopeLevel::Execution(exec_id));
        assert_eq!(*ctx.scope(), ScopeLevel::Execution(exec_id));
    }

    #[test]
    fn basic_ctx_preserves_execution_id() {
        let id = ExecutionId::new();
        let ctx = BasicCtx::new(id);
        assert_eq!(*ctx.execution_id(), id);
    }

    #[test]
    fn extensions_insert_and_get() {
        let mut ext = Extensions::new();
        ext.insert(42_u32);
        ext.insert("hello".to_string());

        assert_eq!(ext.get::<u32>(), Some(&42));
        assert_eq!(ext.get::<String>(), Some(&"hello".to_string()));
        assert_eq!(ext.get::<bool>(), None);
    }

    #[test]
    fn extensions_replace_existing() {
        let mut ext = Extensions::new();
        ext.insert(1_u32);
        ext.insert(2_u32);
        assert_eq!(ext.get::<u32>(), Some(&2));
    }

    #[test]
    fn ctx_ext_delegates_to_extensions() {
        let mut extensions = Extensions::new();
        extensions.insert(99_i64);

        let ctx = BasicCtx::new(ExecutionId::new()).with_extensions(extensions);
        assert_eq!(super::ctx_ext::<i64>(&ctx), Some(&99));
        assert_eq!(super::ctx_ext::<bool>(&ctx), None);
    }

    #[test]
    fn cancel_token_is_accessible() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = BasicCtx::new(ExecutionId::new()).with_cancel_token(child);
        assert!(!ctx.cancel_token().is_cancelled());
        token.cancel();
        assert!(ctx.cancel_token().is_cancelled());
    }
}
