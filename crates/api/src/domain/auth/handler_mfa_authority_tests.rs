use chrono::{TimeDelta, Utc};
use nebula_core::{Principal, UserId};

use super::user_id_from_fresh_mfa_session;
use crate::{
    access::Grant,
    error::ApiError,
    middleware::auth::{AuthContext, AuthMethod},
};

fn auth(auth_method: AuthMethod) -> AuthContext {
    AuthContext {
        principal: Principal::User(UserId::new()),
        auth_method,
        grant: Grant::UnrestrictedIdentity,
    }
}

#[test]
fn fresh_session_is_the_only_mfa_enrollment_authority() {
    let fresh = auth(AuthMethod::Session {
        authenticated_at: Utc::now(),
    });
    assert!(user_id_from_fresh_mfa_session(&fresh).is_ok());

    for method in [AuthMethod::Pat, AuthMethod::ApiKey, AuthMethod::Jwt] {
        let error = user_id_from_fresh_mfa_session(&auth(method))
            .expect_err("header authority must not enroll MFA");
        assert!(matches!(error, ApiError::Forbidden(_)));
    }
}

#[test]
fn stale_or_future_session_requires_primary_reauthentication() {
    for authenticated_at in [
        Utc::now() - TimeDelta::minutes(11),
        Utc::now() + TimeDelta::minutes(1),
    ] {
        let error = user_id_from_fresh_mfa_session(&auth(AuthMethod::Session { authenticated_at }))
            .expect_err("out-of-window session must fail closed");
        assert!(matches!(error, ApiError::Unauthorized(_)));
    }
}
