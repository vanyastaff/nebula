//! Credential lifecycle orchestration (ADR-0092).
//!
//! These modules host the runtime resolution/dispatch primitives the execution
//! engine drives. They were relocated here from `nebula-engine::credential` so
//! the whole credential subsystem lives in one crate; they depend only on the
//! contract types in this crate (no `nebula-engine` / `nebula-storage` edge).

pub mod dispatchers;
pub mod executor;
pub mod lease;
pub mod oauth_egress;
pub mod refresh;
/// Resolution error taxonomy + fail-closed owner/tombstone gates (split from
/// `resolver` for size; behaviour-preserving).
mod resolve_error;
pub mod resolver;

pub use dispatchers::{dispatch_release, dispatch_revoke, dispatch_test};
pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
pub use lease::{
    LeaseLifecycle, LeaseLifecycleConfig, LeaseLifecycleError, LeaseToken, RenewalPolicy,
    StalenessCeiling, StalenessCeilingError,
};
pub use oauth_egress::{
    OAUTH_DNS_MAX_ANSWERS, OAUTH_ENDPOINT_MAX_BYTES, OAuthDnsAnswerError, OAuthEndpointError,
    OAuthServerEndpoint, oauth_egress_ip_is_globally_routable, validate_oauth_dns_answers,
};
pub use refresh::{
    ConfigError, OAuthProviderErrorCode, ReclaimSweepHandle, RefreshCoordConfig,
    RefreshCoordMetrics, RefreshCoordinator, RefreshDisposition, RefreshError, RefreshRecheckError,
    RefreshTransport, RefreshTransportError, SentinelThresholdConfig, SentinelTrigger,
    TokenPostRequest, TokenPostResponse, TokenPostResponseError,
};
pub use resolve_error::ResolveError;
pub use resolver::CredentialResolver;
