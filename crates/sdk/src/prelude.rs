//! Prelude module for Nebula SDK.
//!
//! This module re-exports the most commonly used types and traits.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_sdk::prelude::*;
//! ```

// Core traits and types
// Error handling
pub use anyhow::{Context, Result as AnyhowResult, anyhow, bail};
// Async traits
pub use async_trait::async_trait;
// DX trait families: stateful, trigger
pub use nebula_action::poll::{PollAction, PollCycle};
// Testing harness — context builder, spy emitter/logger/scheduler.
pub use nebula_action::testing::{
    SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
    TriggerTestHarness,
};
// Action traits and types
pub use nebula_action::{
    Action, ActionDependencies, ActionError, ActionResult, Parameter, ParameterCollection,
    StatelessAction, context::Context as ActionContext, metadata::ActionMetadata, port::InputPort,
    port::OutputPort,
};
pub use nebula_action::{
    PollTriggerAdapter, StatefulActionAdapter, StatelessActionAdapter, TriggerContext,
    TriggerEvent, TriggerEventOutcome, WebhookRequest, WebhookTriggerAdapter,
    result::BreakReason,
    stateful::{BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction},
    webhook::{WebhookAction, WebhookHttpResponse, WebhookResponse},
};
// DX codegen macros — re-exported so authors can write `impl_paginated_action!(...)`
// without reaching into `nebula_action::`.
pub use nebula_action::{impl_batch_action, impl_paginated_action};
// Core macros
pub use nebula_core::action_key;
pub use nebula_core::{
    ActionKey, ExecutionId, InterfaceVersion, NodeId, PluginKey, ScopeLevel, Version, WorkflowId,
};
// Credential types (v2)
pub use nebula_credential::{
    // Built-in credentials
    ApiKeyCredential,
    BasicAuthCredential,
    Credential,
    CredentialDescription,
    CredentialError,
    // Typed credential access
    CredentialSnapshot,
    CredentialState,
    // Auth schemes (universal types)
    IdentityPassword,
    OAuth2Credential,
    OAuth2Token,
    SecretString,
    SecretToken,
    SnapshotError,
};
pub use nebula_credential::{CredentialContext, CredentialId};
// Derive macros (re-exported from their respective domain crates)
// Action, Credential, and Plugin derive macros are already in scope from the
// domain crate imports above (same names, macro namespace).
pub use nebula_parameter::Parameters;
// Parameter types
pub use nebula_parameter::prelude::*;
// Plugin types
pub use nebula_plugin::{
    Plugin, PluginMetadata,
    descriptor::{ActionDescriptor, CredentialDescriptor, ResourceDescriptor},
};
pub use nebula_resource::Resource;
pub use nebula_validator::Validator;
// Validator traits
pub use nebula_validator::foundation::{Validate, ValidateExt, ValidationError, ValidationErrors};
// Workflow traits and types
pub use nebula_workflow::{
    ParamValue, WorkflowBuilder as CoreWorkflowBuilder, WorkflowDefinition, connection::Connection,
    node::NodeDefinition,
};
// Serialization
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Map, Value, json};
pub use thiserror::Error;

// SDK builders and result types
pub use crate::action::ActionBuilder;
// In-process run harness for single-action examples and tests.
pub use crate::runtime::{RunReport, TestRuntime};
pub use crate::{Error as SdkError, Result as SdkResult, workflow::WorkflowBuilder};
// Re-export SDK macros
pub use crate::{params, simple_action, workflow};
