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
pub use nebula_core::{
    ActionKey, ExecutionId, InterfaceVersion, NodeId, PluginKey, ScopeLevel, Version, WorkflowId,
};

// Action traits and types
pub use nebula_action::{
    Action, ActionDependencies, ActionError, ActionResult, Parameter, ParameterCollection,
    StatelessAction, context::Context as ActionContext, metadata::ActionMetadata, port::InputPort,
    port::OutputPort,
};

// Workflow traits and types
pub use nebula_workflow::{
    ParamValue, WorkflowBuilder as CoreWorkflowBuilder, WorkflowDefinition, connection::Connection,
    node::NodeDefinition,
};

// Parameter types
pub use nebula_parameter::prelude::*;

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

// Plugin types
pub use nebula_plugin::{
    Plugin, PluginMetadata,
    descriptor::{ActionDescriptor, CredentialDescriptor, ResourceDescriptor},
};

// Core macros
pub use nebula_core::action_key;

// Derive macros (re-exported from their respective domain crates)
// Action, Credential, and Plugin derive macros are already in scope from the
// domain crate imports above (same names, macro namespace).
pub use nebula_parameter::Parameters;
pub use nebula_resource::Resource;
pub use nebula_validator::Validator;

// Validator traits
pub use nebula_validator::foundation::{Validate, ValidateExt, ValidationError, ValidationErrors};

// SDK builders and result types
pub use crate::action::ActionBuilder;
pub use crate::workflow::WorkflowBuilder;
pub use crate::{Error as SdkError, Result as SdkResult};

// Async traits
pub use async_trait::async_trait;

// Serialization
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Map, Value, json};

// Error handling
pub use anyhow::{Context, Result as AnyhowResult, anyhow, bail};
pub use thiserror::Error;

// Re-export SDK macros
pub use crate::{params, simple_action, workflow};
