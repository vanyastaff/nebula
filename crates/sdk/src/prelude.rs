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
//! #[derive(Debug)]
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
//!
//!     async fn destroy(&self, instance: HttpClient, _cx: TeardownCx) -> Result<(), Error> {
//!         // This client needs only RAII cleanup; omitting this override is equivalent.
//!         drop(instance);
//!         Ok(())
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
//! A real adapter's consuming [`Provider::destroy`] owns all async flush, stop,
//! close, and worker-join work. [`TeardownCx`] supplies the deadline and
//! [`TeardownReason`]; use `cx.deadline.into()` with Tokio's `timeout_at`.
//! Errors still consume the instance and are never retried by the framework.
//! Cleanup must tolerate manager cancellation and a dropped future at any await,
//! so owned tasks and handles need synchronous drop fallback. The default hook
//! only runs Drop; neither path promises cleanup after a process crash.
//!
//! Engine-side registration uses deployment/runtime APIs rather than an SDK
//! re-export of implementation crates; action code receives a [`ResourceGuard`]
//! that derefs to `Provider::Instance`.

// Core traits and types
// DX trait families: stateful, trigger
// Testing harness — context builder, spy emitter/logger/scheduler.
// Action traits and types
pub use nebula_action::{
    Action, ActionContext, ActionEffectContract, ActionError, ActionMetadataDraft, ActionResult,
    CheckpointPolicy, Field, IsolationLevel, RemoteDestinationGuarantee, RemoteEffectDescriptor,
    RemoteEffectPolicy, RemoteEffectPolicyBuilder, RemoteEffectPolicyError, Schema,
    StatelessAction, StreamAction, TriggerContext, TriggerEvent, TriggerEventOutcome,
    TriggerHealthSnapshot, WebhookRequest, field_key,
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
    ActionKey, AuthPattern, CredentialKey, Dependencies, ExecutionId, NodeKey, OperationCallId,
    PluginKey, ResourceKey, ScopeLevel, WorkflowId, action_key, credential_key, resource_key,
};
// Credential types (v2)
pub use nebula_credential::{
    // Built-in credentials
    ApiKeyCredential,
    BasicAuthCredential,
    Credential,
    CredentialError,
    // Author-owned integration-catalog metadata intent
    CredentialMetadataDraft,
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
// Shared authoring vocabulary used by action, credential, and resource drafts.
pub use nebula_metadata::{
    DeprecationNotice, Icon, MaturityLevel, MetadataError, MetadataName, MetadataVersion,
    metadata_name,
};
// Plugin types. `ManifestError`/`PluginDependency`/`PluginManifestBuilder`
// join `Plugin`/`PluginManifest`: `PluginManifestBuilder` is already named as
// a parameter type by a consumer (`crates/plugin/tests/frozen_registry.rs`),
// so it was reachable only anonymously via method chaining, not nameable.
pub use nebula_plugin::{
    ManifestError, Plugin, PluginDependency, PluginManifest, PluginManifestBuilder,
};
// Resource authoring surface — mirrors `nebula_resource::prelude` plus the
// `Resource` / `ResourceConfig` / `ClassifyError` derive names.
//
// `Error` here is the resource error *type*; `thiserror::Error` below is a
// derive *macro* — different namespaces, so both live in the glob.
//
// Engine-only types (`Manager`, `Registry`, `ReleaseQueue`,
// `credential_fanout`) are deliberately absent from the supported SDK.
pub use nebula_resource::{
    Bounded, BoundedMode, BoundedProvider, ClassifyError, Error, ErrorKind, HasCredentialSlots,
    PoolConfig, PoolProvider, Pooled, Provider, ReleaseOutcome, ReloadOutcome, Resident,
    ResidentConfig, ResidentProvider, Resource, ResourceConfig, ResourceContext, ResourceGuard,
    ResourceMetadataDraft, SlotCell, TeardownCx, TeardownReason, TopologyTag, no_credential_slots,
};
// Derive names are re-exported from their respective domain crates. Generated
// paths prefer a direct (including renamed) leaf dependency, then the SDK's
// hidden macro namespace.
// Schema types — Field/Schema/ValidSchema/field_key already re-exported via nebula_action
// above.
pub use nebula_schema::{
    AuthoredValue, BooleanField, CodeField, ComputedField, DynamicField, EnumSelect, Expression,
    ExpressionMode, FieldKey, FieldPath, FileField, HasSchema, HasSelectOptions, InputHint,
    ListField, LoaderContext, LoaderRegistry, ModeField, NoticeField, NumberField, ObjectField,
    Predicate, ProgramSyntax, RecordShape, RequiredMode, RootShape, Rule, ScalarKind, ScalarSchema,
    ScalarValue, SchemaBuilder, SchemaKind, SecretField, SelectField, SelectOption, SerdeTagging,
    Severity, StringField, Transformer, UnionShape, UnknownField, ValidSchema, ValidationError,
    ValidationReport, ValuePath, VisibilityMode, schema_of,
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

// In-process run harness for single-action examples and tests.
pub use crate::runtime::{RunReport, TestRuntime};
pub use crate::{Error as SdkError, Result as SdkResult, workflow::WorkflowBuilder};
// Re-export SDK macros
pub use crate::{params, simple_action, workflow};
