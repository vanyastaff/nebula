//! Prelude module for Nebula SDK.
//!
//! This module re-exports the most commonly used types and traits.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_sdk::prelude::*;
//! ```
//!
//! ## Authoring a pooled resource
//!
//! Types, traits, and procedural derives for resource authoring live in the
//! prelude. Generated paths resolve through the SDK when the integration has no
//! direct leaf-crate dependency. The manual [`Provider`] surface below is also
//! curated; `type Topology = Pooled<Self>` opts
//! into pool checkout/recycle (every [`PoolProvider`] hook has a default, so an
//! empty impl suffices).
//!
//! ```rust
//! use nebula_sdk::prelude::*;
//!
//! #[derive(Clone, Debug)]
//! struct HttpClient {
//!     base_url: String,
//! }
//!
//! #[derive(Clone)]
//! struct HttpResource;
//! no_credential_slots!(HttpResource);
//!
//! #[async_trait::async_trait]
//! impl Provider for HttpResource {
//!     type Config = ();
//!     type Instance = HttpClient;
//!     type Topology = Pooled<Self>;
//!
//!     fn key() -> ResourceKey {
//!         resource_key!("http.client.sdk_prelude")
//!     }
//!
//!     async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<HttpClient, Error> {
//!         Ok(HttpClient {
//!             base_url: "https://api.example.com".to_owned(),
//!         })
//!     }
//! }
//!
//! impl PoolProvider for HttpResource {}
//!
//! // A resource with no `#[credential]` fields can skip the derive entirely.
//! # struct Slotless;
//! no_credential_slots!(Slotless);
//! ```
//!
//! Engine-side registration uses deployment/runtime APIs rather than an SDK
//! re-export of implementation crates; action code receives a [`ResourceGuard`]
//! that derefs to `Provider::Instance`.

// Core traits and types
// DX trait families: stateful, trigger
// Testing harness — context builder, spy emitter/logger/scheduler.
// Action traits and types
pub use nebula_action::{
    Action, ActionContext, ActionError, ActionResult, Field, PollTriggerAdapter, Schema,
    StatefulActionAdapter, StatelessAction, StatelessActionAdapter, StreamAction, TriggerContext,
    TriggerEvent, TriggerEventOutcome, ValidSchema, WebhookRequest, WebhookTriggerAdapter,
    field_key,
    metadata::ActionMetadata,
    poll::{DeduplicatingCursor, PollAction, PollConfig, PollCursor, PollResult},
    port::{InputPort, OutputPort},
    result::BreakReason,
    stateful::{BatchAction, BatchItemResult, PageResult, PaginatedAction, StatefulAction},
    testing::{
        SpyEmitter, SpyLogger, SpyScheduler, StatefulTestHarness, TestContextBuilder,
        TriggerTestHarness,
    },
    webhook::{WebhookAction, WebhookHttpResponse, WebhookResponse},
};
// DX codegen macros — re-exported so authors can write `impl_paginated_action!(...)`
// without reaching into `nebula_action::`.
pub use nebula_action::{impl_batch_action, impl_paginated_action};
pub use nebula_core::AuthScheme as AuthSchemeContract;
pub use nebula_core::auth::NoAuthFamily;
pub use nebula_core::{
    ActionKey, AuthPattern, ExecutionId, NodeKey, PluginKey, ResourceKey, ScopeLevel, WorkflowId,
    action_key, resource_key,
};
// Credential types (v2)
pub use nebula_credential::{
    // Built-in credentials
    ApiKeyCredential,
    BasicAuthCredential,
    Credential,
    CredentialError,
    // Integration-catalog metadata (key, name, parameters, pattern)
    CredentialMetadata,
    // Runtime operational state (created_at, version, ...)
    CredentialRecord,
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
pub use nebula_credential::{AuthScheme, credential};
pub use nebula_credential::{CredentialContext, CredentialId};
// Shared catalog-metadata vocabulary — the `Metadata` trait plus the
// `BaseMetadata` prefix and value types that `ActionMetadata`,
// `CredentialMetadata`, and `ResourceMetadata` all compose. Re-exported so the
// uniform `metadata()` accessor surface (`key`/`name`/`version`/`icon`/…) is
// usable across all three catalog leaves from a single import.
pub use nebula_metadata::{BaseMetadata, DeprecationNotice, Icon, MaturityLevel, Metadata};
// Plugin types
pub use nebula_plugin::{Plugin, PluginManifest};
// Resource authoring surface — mirrors `nebula_resource::prelude` plus the
// `Resource` / `ResourceConfig` / `ClassifyError` derive names.
//
// `Error` here is the resource error *type*; `thiserror::Error` below is a
// derive *macro* — different namespaces, so both live in the glob.
//
// Engine-only types (`Manager`, `Registry`, `ReleaseQueue`,
// `credential_fanout`) are deliberately absent from the supported SDK.
pub use nebula_resource::{
    AcquireOptions, Bounded, BoundedMode, BoundedProvider, ClassifyError, Error, ErrorKind,
    HasCredentialSlots, PoolConfig, PoolProvider, Pooled, Provider, RegistrationSpec,
    ReloadOutcome, Resident, ResidentConfig, ResidentProvider, Resource, ResourceConfig,
    ResourceContext, ResourceGuard, ResourceMetadata, SlotCell, SlotIdentity, TopologyTag,
    no_credential_slots,
};
// Derive names are re-exported from their respective domain crates. Generated
// paths prefer a direct (including renamed) leaf dependency, then the SDK's
// hidden macro namespace.
// Schema types — Field/Schema/ValidSchema/field_key already re-exported via nebula_action
// above.
pub use nebula_schema::{
    BooleanField, CodeField, ComputedField, DynamicField, Expression, ExpressionMode, FieldKey,
    FieldPath, FieldValue, FieldValues, FileField, InputHint, ListField, LoaderContext,
    LoaderRegistry, ModeField, NoticeField, NumberField, ObjectField, RequiredMode, ResolvedValues,
    SchemaBuilder, SecretField, SelectField, SelectOption, Severity, StringField, Transformer,
    ValidValues, ValidationError, ValidationReport, VisibilityMode,
};
pub use nebula_validator::Validator;
// Validator traits
pub use nebula_validator::foundation::{Validate, ValidateExt};
pub use nebula_workflow::Version;
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
