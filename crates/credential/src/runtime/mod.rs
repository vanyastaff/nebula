//! Credential-owned execution and lifecycle machinery.
//!
//! Projection is read-only and shared by management and worker runtimes.
//! Acquisition, refresh coordination, and lease lifecycle belong to the
//! management runtime. Provider transport implementations and persistence
//! backends are injected by the application composition roots.

pub mod acquisition;
pub mod dispatchers;
pub mod executor;
pub mod lease;
mod lifecycle;
pub mod oauth_egress;
/// Read-only projection shared by management and execution-worker runtimes.
pub(crate) mod projection;
pub mod refresh;
/// Resolution error taxonomy + fail-closed owner/tombstone gates (split from
/// `resolver` for size; behaviour-preserving).
mod resolve_error;
pub mod resolver;
/// Configured source of credential material, independent of management authority.
pub(crate) mod state_source;

pub use acquisition::{AcquisitionTransport, AcquisitionTransportError};
pub use dispatchers::{dispatch_release, dispatch_revoke, dispatch_test};
pub use executor::{
    ExecutorError, ResolveResponse, execute_begin, execute_continue, execute_resolve,
};
pub use lease::{
    LeaseLifecycle, LeaseLifecycleConfig, LeaseLifecycleError, LeaseToken, RenewalPolicy,
    StalenessCeiling, StalenessCeilingError,
};
pub use lifecycle::CredentialLifecycleRuntime;
pub use oauth_egress::{
    OAUTH_DNS_MAX_ANSWERS, OAUTH_ENDPOINT_MAX_BYTES, OAuthDnsAnswerError, OAuthEndpointError,
    OAuthServerEndpoint, oauth_egress_ip_is_globally_routable, validate_oauth_dns_answers,
};
pub use refresh::{
    ConfigError, ReclaimSweepHandle, RefreshCoordConfig, RefreshCoordMetrics, RefreshCoordinator,
    RefreshDisposition, RefreshError, RefreshRecheck, RefreshRecheckError, RefreshTransport,
    RefreshTransportError, SentinelEscalationPolicy, SentinelEscalationPolicyError,
    TokenPostRequest, TokenPostResponse, TokenPostResponseError,
};
pub use resolve_error::ResolveError;
pub use resolver::CredentialResolver;
