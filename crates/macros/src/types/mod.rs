//! Parsed attribute types for derive macros.
//!
//! Each macro (Action, Credential, etc.) may have its own attrs type
//! mirroring the target struct's fields.

mod action_attrs;
mod credential_attrs;
mod param_attrs;
mod plugin_attrs;

pub use action_attrs::ActionAttrs;
pub use credential_attrs::CredentialAttrs;
pub use param_attrs::ParameterAttrs;
pub use plugin_attrs::PluginAttrs;
