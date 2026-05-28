//! OAuth userinfo + verified-emails GET helpers.
//!
//! Used by `PgAuthBackend::complete_oauth` and
//! `InMemoryAuthBackend::complete_oauth` after the token POST: fetch
//! the IdP's `userinfo_url` to get `sub`/`email`/`email_verified`,
//! and (for providers like GitHub whose userinfo lacks
//! `email_verified`) fetch the optional `verified_emails_url` and
//! pick the primary+verified entry.
//!
//! Per ADR-0085:
//! - **D-7**: tokens discarded after the userinfo lookup (borrow
//!   checker enforces; we never persist access_token).
//! - **D-9-WAVE6**: every server-side URL goes through the strict
//!   `validate_oauth_outbound_url` gate, including userinfo +
//!   verified_emails — defensive check even though boot validation
//!   already vetted Manual fields.
//! - **D-16**: userinfo response is authoritative for `(sub, email)`;
//!   no JWKS validation of id_token in 1.0.
//!
//! Body cap: same 256 KiB ceiling as the token endpoint (token POST
//! uses `OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES`); we cap here too so a
//! hostile IdP cannot DoS via an unbounded userinfo response.

use futures::StreamExt;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

use super::flow::validate_oauth_outbound_url;
use super::http::{OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES, oauth_token_http_client};

/// Failure modes for [`fetch_userinfo`] and
/// [`fetch_primary_verified_email`].
#[derive(Debug, Error)]
pub enum UserinfoError {
    /// The strict gate rejected the URL before any HTTP call.
    #[error("OAuth userinfo URL `{url}` rejected by anti-SSRF gate: {reason}")]
    UrlRejected {
        /// The rejected URL.
        url: String,
        /// Reason from `validate_oauth_outbound_url`.
        reason: String,
    },
    /// Network or HTTP-level failure during the GET.
    #[error("OAuth userinfo GET failed for `{url}`: {source}")]
    HttpError {
        /// The URL being fetched.
        url: String,
        /// Underlying `reqwest` error.
        #[source]
        source: reqwest::Error,
    },
    /// Non-2xx response from the IdP userinfo endpoint.
    #[error("OAuth userinfo `{url}` returned HTTP {status}")]
    NonSuccessStatus {
        /// The URL being fetched.
        url: String,
        /// HTTP status code.
        status: u16,
    },
    /// Response body exceeded the 256 KiB cap.
    #[error("OAuth userinfo `{url}` body exceeded {max} bytes")]
    BodyTooLarge {
        /// The URL being fetched.
        url: String,
        /// The byte cap that was hit.
        max: usize,
    },
    /// Response body did not deserialize.
    #[error("OAuth userinfo `{url}` body parse failed: {source}")]
    ParseError {
        /// The URL being fetched.
        url: String,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
    /// Userinfo response is missing the required `sub` claim.
    #[error("OAuth userinfo `{url}` response is missing the `sub` claim")]
    MissingSub {
        /// The URL being fetched.
        url: String,
    },
    /// Verified-emails endpoint returned a list with no primary AND
    /// verified entry. GitHub: this means the user has no verified
    /// primary email; OAuth identity link cannot proceed.
    #[error("OAuth verified-emails `{url}` returned no primary+verified entry")]
    NoVerifiedPrimaryEmail {
        /// The URL being fetched.
        url: String,
    },
}

/// Resolved subset of an IdP userinfo response that Plane A consumes
/// per REQ-oauth-004 / -005 / -006.
#[derive(Debug, Clone)]
pub struct UserinfoClaims {
    /// IdP stable subject claim. Required \u2014 missing this is an error.
    pub sub: String,
    /// User's email. May be `None` for providers that don't return it
    /// (e.g. GitHub without `user:email` scope). The first-login /
    /// link-existing-user branches require this; the REQ-oauth-006
    /// short-circuit does not (the `sub` linkage takes precedence).
    pub email: Option<String>,
    /// Whether the IdP claims the email is verified. `None` when the
    /// claim is absent from the response \u2014 caller may fall through
    /// to a verified_emails_url lookup (GitHub pattern).
    pub email_verified: Option<bool>,
}

/// GET the IdP's `userinfo_url` with `Authorization: Bearer <access_token>`,
/// parse the response into [`UserinfoClaims`], and return.
///
/// Anti-SSRF: `validate_oauth_outbound_url(url)` runs first; the GET
/// uses the production `oauth_token_http_client` (5-redirect cap).
/// Body capped at 256 KiB.
///
/// # Errors
///
/// Returns [`UserinfoError`] on any of: URL rejected, network /
/// non-2xx response, body too large, parse failure, or missing `sub`.
#[tracing::instrument(level = "debug", skip(access_token), fields(url_host = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()))]
pub async fn fetch_userinfo(
    url: &str,
    access_token: &str,
) -> Result<UserinfoClaims, UserinfoError> {
    validate_oauth_outbound_url(url).map_err(|reason| UserinfoError::UrlRejected {
        url: url.to_owned(),
        reason,
    })?;
    let response = oauth_token_http_client()
        .get(url)
        .bearer_auth(access_token)
        // GitHub /user requires User-Agent; harmless for OIDC providers.
        .header(reqwest::header::USER_AGENT, "nebula-api/1.0")
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|source| UserinfoError::HttpError {
            url: url.to_owned(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(UserinfoError::NonSuccessStatus {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }
    let body = read_response_limited(url, response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES).await?;
    let raw: serde_json::Value =
        serde_json::from_slice(&body).map_err(|source| UserinfoError::ParseError {
            url: url.to_owned(),
            source,
        })?;

    // `sub` is required. OIDC providers return `sub`; GitHub `/user`
    // returns `id` as an integer instead \u2014 we accept either.
    // Per CodeRabbit wave-1 H.2: restrict the `id` fallback to scalar
    // values (string or number) only — a JSON object/array/bool would
    // otherwise stringify to garbage like `"true"` or `"{...}"` and
    // create an invalid (provider, subject) link.
    let sub_from_id = raw.get("id").and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let sub = raw
        .get("sub")
        .and_then(|v| v.as_str().map(str::to_owned))
        .or(sub_from_id)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| UserinfoError::MissingSub {
            url: url.to_owned(),
        })?;
    let email = raw
        .get("email")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty());
    let email_verified = raw
        .get("email_verified")
        .and_then(serde_json::Value::as_bool);
    Ok(UserinfoClaims {
        sub,
        email,
        email_verified,
    })
}

/// Entry in a GitHub-style `/user/emails` response.
///
/// Only the three fields Plane A consumes are deserialized.
#[derive(Debug, Clone, Deserialize)]
struct VerifiedEmailEntry {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

/// GET the IdP's `verified_emails_url` (e.g. GitHub's
/// `https://api.github.com/user/emails`) with the same Bearer token,
/// then pick the entry where `primary == true AND verified == true`.
/// Returns the email string.
///
/// Used by `complete_oauth` for Manual providers whose primary
/// `userinfo_url` does not include `email_verified` (per ADR-0085
/// D-5 wave-6).
///
/// # Errors
///
/// Returns [`UserinfoError`] on URL rejection, HTTP failure, body
/// too large, parse failure, or no primary+verified entry.
#[tracing::instrument(level = "debug", skip(access_token), fields(url_host = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()))]
pub async fn fetch_primary_verified_email(
    url: &str,
    access_token: &str,
) -> Result<String, UserinfoError> {
    validate_oauth_outbound_url(url).map_err(|reason| UserinfoError::UrlRejected {
        url: url.to_owned(),
        reason,
    })?;
    let response = oauth_token_http_client()
        .get(url)
        .bearer_auth(access_token)
        .header(reqwest::header::USER_AGENT, "nebula-api/1.0")
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|source| UserinfoError::HttpError {
            url: url.to_owned(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(UserinfoError::NonSuccessStatus {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }
    let body = read_response_limited(url, response, OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES).await?;
    let entries: Vec<VerifiedEmailEntry> =
        serde_json::from_slice(&body).map_err(|source| UserinfoError::ParseError {
            url: url.to_owned(),
            source,
        })?;
    entries
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email)
        .ok_or_else(|| UserinfoError::NoVerifiedPrimaryEmail {
            url: url.to_owned(),
        })
}

/// Streamed read with a byte cap. Mirrors the token-response limiter
/// pattern from `transport/oauth/http.rs`.
async fn read_response_limited(
    url: &str,
    response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, UserinfoError> {
    if let Some(claimed) = response.content_length()
        && claimed > max as u64
    {
        return Err(UserinfoError::BodyTooLarge {
            url: url.to_owned(),
            max,
        });
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| UserinfoError::HttpError {
            url: url.to_owned(),
            source,
        })?;
        if buf.len().saturating_add(chunk.len()) > max {
            return Err(UserinfoError::BodyTooLarge {
                url: url.to_owned(),
                max,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}
