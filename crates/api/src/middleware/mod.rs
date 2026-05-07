//! Middleware
//!
//! Custom middleware для API: auth, rate limiting, request ID, tenancy,
//! RBAC, CSRF, etc.

pub mod auth;
pub mod csrf;
pub mod idempotency;
pub mod internal_auth;
pub mod rate_limit;
pub mod rbac;
pub mod request_id;
pub mod security_headers;
pub mod tenancy;
pub mod webhook_ratelimit;

pub use auth::auth_middleware;
pub use csrf::csrf_middleware;
pub use idempotency::{IdempotencyLayer, IdempotencyStore, InMemoryIdempotencyStore};
pub use internal_auth::{X_INTERNAL_TOKEN, internal_auth_middleware};
pub use rate_limit::RateLimitState;
pub use rbac::rbac_middleware;
pub use request_id::RequestIdLayer;
pub use security_headers::security_headers_middleware;
pub use tenancy::tenancy_middleware;
