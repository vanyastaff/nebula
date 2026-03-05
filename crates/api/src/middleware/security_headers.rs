//! Security Headers Middleware
//!
//! Adds security headers to all responses.

use axum::{http::{header, HeaderName, HeaderValue}, response::Response};

/// Security headers middleware
pub async fn security_headers_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    
    let headers = response.headers_mut();
    
    // X-Content-Type-Options: nosniff
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    
    // X-Frame-Options: DENY
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    
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
        headers.insert(
            HeaderName::from_static("permissions-policy"),
            val,
        );
    }
    
    response
}


