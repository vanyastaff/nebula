//! Authentication endpoint handlers.
//!
//! These endpoints handle user registration, login, logout,
//! password management, MFA, and OAuth flows.
//! No tenant scope required — these are global auth endpoints.

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    errors::{ApiError, ApiResult},
    state::AppState,
};

/// POST /api/v1/auth/signup
pub async fn signup(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Implement user registration
    // - Validate email, password strength
    // - Check email not already registered
    // - Hash password with argon2
    // - Create user record
    // - Send verification email
    // - Return user profile (no sensitive data)
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/login
pub async fn login(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Implement login
    // - Validate credentials (argon2 verify, < 200ms p99)
    // - Check account not locked
    // - If MFA enabled, return MFA_REQUIRED
    // - Create session
    // - Set nebula_session cookie + nebula_csrf cookie
    // - Return user profile
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/logout
pub async fn logout(State(_state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Invalidate session, clear cookies
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/forgot-password
pub async fn forgot_password(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Send password reset email
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/reset-password
pub async fn reset_password(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Verify reset token, update password
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/verify-email
pub async fn verify_email(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Verify email token, mark email verified
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/mfa/enroll
pub async fn mfa_enroll(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Generate TOTP secret, return QR code
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/auth/mfa/verify
pub async fn mfa_verify(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Verify TOTP code, complete login
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/auth/oauth/{provider} — start OAuth flow
pub async fn oauth_start(
    State(_state): State<AppState>,
    Path(_provider): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Generate OAuth state, redirect to provider
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/auth/oauth/{provider}/callback — OAuth callback
pub async fn oauth_callback(
    State(_state): State<AppState>,
    Path(_provider): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Exchange code for token, create/link user, create session
    Err(ApiError::Internal("not implemented".to_string()))
}
