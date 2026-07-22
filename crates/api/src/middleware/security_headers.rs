//! Security Headers Middleware
//!
//! Adds security headers to all responses.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};

/// Mark a route response as carrying or rotating authentication authority.
///
/// This route-level middleware is the canonical cache policy for session
/// cookies, one-time challenges, enrollment seeds, and credentials returned
/// exactly once. It deliberately overwrites a weaker inner cache policy and
/// applies to error responses too, so a future handler branch cannot silently
/// make the same security-sensitive route cacheable.
pub async fn no_store_authority_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// Security headers middleware
pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();

    // X-Content-Type-Options: nosniff
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    // X-Frame-Options: DENY
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));

    // Content-Security-Policy: default-src 'none'; frame-ancestors 'none'
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );

    // Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );

    // Referrer-Policy: no-referrer
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );

    // Permissions-Policy: camera=(), microphone=(), geolocation=()
    if let Ok(val) = HeaderValue::from_str("camera=(), microphone=(), geolocation=()") {
        headers.insert(HeaderName::from_static("permissions-policy"), val);
    }

    response
}
