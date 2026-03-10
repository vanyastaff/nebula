//! Authentication Middleware
//!
//! JWT-based authentication.

use axum::{
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};

/// Simple auth middleware (JWT validation можно добавить позже)
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    if let Some(_token) = auth_header {
        // TODO: Validate JWT token
        // let claims = validate_jwt(token)?;
        // req.extensions_mut().insert(claims);
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Auth middleware struct для использования с Tower
pub struct AuthMiddleware;
