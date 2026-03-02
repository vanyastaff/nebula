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
    Action,
    ActionError,
    ActionResult,
    ParameterCollection,
    ParameterDef,
    // TODO: Action types temporarily unavailable
    // InteractiveAction, ProcessAction, SimpleAction, StatefulAction, StreamingAction,
    // TransactionalAction, TriggerAction,
    metadata::ActionMetadata,
    port::InputPort,
    port::OutputPort,
};

// Workflow traits and types
pub use nebula_workflow::{
    ParamValue, WorkflowBuilder as CoreWorkflowBuilder, WorkflowDefinition, connection::Connection,
    node::NodeDefinition,
};

// Parameter types
pub use nebula_parameter::prelude::*;

// Credential types
pub use nebula_credential::{
    // Core
    core::CredentialContext,
    core::CredentialDescription,
    core::CredentialError,
    core::CredentialState,
    // Protocols — StaticProtocol
    protocols::ApiKeyProtocol,
    protocols::ApiKeyState,
    protocols::AuthStyle,
    protocols::BasicAuthProtocol,
    protocols::BasicAuthState,
    protocols::DatabaseProtocol,
    protocols::DatabaseState,
    protocols::GrantType,
    protocols::HeaderAuthProtocol,
    protocols::HeaderAuthState,
    protocols::KerberosConfig,
    protocols::LdapConfig,
    protocols::LdapProtocol,
    protocols::LdapState,
    protocols::MtlsConfig,
    protocols::OAuth2Config,
    protocols::OAuth2ConfigBuilder,
    // Protocols — FlowProtocol
    protocols::OAuth2Protocol,
    protocols::OAuth2State,
    protocols::SamlBinding,
    protocols::SamlConfig,
    protocols::TlsMode,
    traits::CredentialResource,
    traits::CredentialType,
    traits::FlowProtocol,
    traits::Refreshable,
    traits::Revocable,
    // Traits
    traits::StaticProtocol,
};

// Plugin types
pub use nebula_plugin::{Plugin, PluginComponents, PluginMetadata};

// Macros
pub use nebula_macros::{Action, Credential, Parameters, Plugin, Resource, Validator};

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
