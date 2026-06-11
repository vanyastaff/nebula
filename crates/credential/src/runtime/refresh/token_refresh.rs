//! OAuth2 token-refresh state logic (ADR-0092).
//!
//! SSRF endpoint validation (SEC-10), secret-scoped form composition, response
//! status interpretation, SEC-02 error redaction, and `OAuth2State` mutation all
//! live here — on the `nebula-credential` side of the [`RefreshTransport`] seam.
//! Network I/O is delegated to the injected transport; this module never links
//! reqwest.
//!
//! # Sentinel marking
//!
//! Per sub-spec `docs/INTEGRATION_MODEL.md` the holder marks the L2 claim row
//! `sentinel = RefreshInFlight` immediately before the IdP POST. That mark is
//! set by the `CredentialResolver::refresh_via_coordinator` closure (the caller
//! of `refresh_oauth2_state`) **outside** this module, so we do not have to
//! thread `RefreshClaim` + `RefreshClaimRepo` into the transport layer.
//!
//! On the success path the row is deleted entirely by
//! `RefreshCoordinator::refresh_coalesced` via `repo.release(token)` —
//! the sentinel clears by row removal, no separate "clear" call is needed.

use std::net::IpAddr;

use chrono::Utc;
use serde_json::Value;
use url::{Host, Url};

use crate::AuthStyle;
use crate::SecretString;
use crate::credentials::OAuth2State;
use crate::runtime::refresh::transport::{RefreshTransport, TokenPostRequest};

/// The SEC-01 hard cap on OAuth2 token endpoint response body size.
/// 256 KiB — tokens are small JSON documents; larger responses are anomalous.
/// This value is a *policy constant* owned by the credential crate; the
/// transport enforces the read bound mechanically.
pub const OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Refresh-related failures produced by [`refresh_oauth2_state`].
#[derive(Debug, thiserror::Error)]
pub enum TokenRefreshError {
    /// Stored state lacks a refresh token, so re-auth is required.
    #[error("no refresh_token available for token refresh")]
    MissingRefreshToken,
    /// HTTP request failed (transport error or SSRF pre-check).
    #[error("refresh token request failed: {0}")]
    Request(String),
    /// Token endpoint returned non-success status.
    #[error("token endpoint returned {status}: {summary}")]
    TokenEndpoint {
        /// HTTP status code string.
        status: String,
        /// Sanitized RFC 6749 error summary.
        summary: String,
    },
    /// Token endpoint response could not be parsed as JSON.
    #[error("failed to parse token response: {0}")]
    Parse(String),
    /// Response body was missing required `access_token`.
    #[error("refresh response missing required 'access_token' field")]
    MissingAccessToken,
}

/// Execute OAuth2 refresh-token grant and mutate `state` in place.
///
/// Call order (security boundary — do not reorder):
/// 1. [`validate_token_endpoint`] runs FIRST (SSRF: https-only, no
///    localhost / private / link-local). Returns `Err` before any I/O.
/// 2. Secret-scoped inner block builds a [`TokenPostRequest`] (form fields +
///    optional `basic_auth`). Secret borrows are released when the block ends;
///    the transport receives `SecretString` values that zeroize on drop.
/// 3. [`RefreshTransport::post_token`] is called — the ONLY network I/O.
/// 4. [`parse_token_response_bytes`] interprets status + bytes; SEC-02
///    redaction runs inside this crate, not in the transport.
/// 5. [`update_state_from_token_response`] mutates `state` on success.
///
/// SEC-10: the three secret values (refresh_token, client_id, client_secret)
/// are NOT extracted into `Zeroizing<String>` intermediates. Instead, secret
/// borrows live inside an inner block that returns the built
/// `TokenPostRequest`; the block ends → secret borrows drop → `state` is free
/// for `&mut` mutation in `update_state_from_token_response`. No owned
/// plaintext copy lives in our code; the unavoidable in-flight copy lives in
/// the transport's request serialization and is released after the response
/// future resolves.
pub async fn refresh_oauth2_state(
    state: &mut OAuth2State,
    transport: &dyn RefreshTransport,
) -> Result<(), TokenRefreshError> {
    // Step 1 — SSRF validation (must run before any I/O).
    validate_token_endpoint(&state.token_url).map_err(TokenRefreshError::Request)?;

    let scope_joined: Option<String> = (!state.scopes.is_empty()).then(|| state.scopes.join(" "));

    // Step 2 — Build the request inside a tight secret-borrow scope.
    // After this block the secret borrows have dropped; only `TokenPostRequest`
    // (carrying `SecretString` values) crosses the block boundary.
    let req = {
        let refresh_tok = state
            .refresh_token
            .as_ref()
            .ok_or(TokenRefreshError::MissingRefreshToken)?
            .expose_secret();
        let client_id = state.client_id.expose_secret();
        let client_secret = state.client_secret.expose_secret();

        let mut form: Vec<(String, SecretString)> = vec![
            ("grant_type".to_owned(), SecretString::new("refresh_token")),
            ("refresh_token".to_owned(), SecretString::new(refresh_tok)),
        ];
        if let Some(ref scope) = scope_joined {
            form.push(("scope".to_owned(), SecretString::new(scope.as_str())));
        }

        let basic_auth = match state.auth_style {
            AuthStyle::Header => {
                // client_id / client_secret travel as HTTP Basic Auth;
                // they do NOT go in the form body.
                Some((client_id.to_owned(), SecretString::new(client_secret)))
            },
            AuthStyle::PostBody => {
                // client_id / client_secret go in the form body.
                form.push(("client_id".to_owned(), SecretString::new(client_id)));
                form.push(("client_secret".to_owned(), SecretString::new(client_secret)));
                None
            },
        };

        TokenPostRequest {
            url: state.token_url.clone(),
            form,
            basic_auth,
            max_response_bytes: OAUTH_TOKEN_HTTP_MAX_RESPONSE_BYTES,
        }
    };

    // Step 3 — delegate I/O to the transport (dumb pipe).
    let resp = transport
        .post_token(req)
        .await
        .map_err(|e| TokenRefreshError::Request(e.to_string()))?;

    // Steps 4 + 5 — status interpretation, bounded parse, SEC-02 redaction,
    // state mutation.  All on the credential side.
    let body = parse_token_response_bytes(resp.status, &resp.body)?;
    update_state_from_token_response(state, &body)?;
    Ok(())
}

fn validate_token_endpoint(raw: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| format!("invalid OAuth token endpoint URL: {e}"))?;
    if url.scheme() != "https" {
        return Err("OAuth token endpoint must use https".to_owned());
    }

    validate_token_endpoint_host(url.host())?;
    Ok(())
}

fn validate_token_endpoint_host(host: Option<Host<&str>>) -> Result<(), String> {
    match host.ok_or_else(|| "OAuth token endpoint must include a host".to_owned())? {
        Host::Domain(host) if host.eq_ignore_ascii_case("localhost") => {
            Err("OAuth token endpoint must not target localhost".to_owned())
        },
        Host::Domain(_) => Ok(()),
        Host::Ipv4(ip) if forbidden_token_endpoint_ip(IpAddr::V4(ip)) => {
            Err("OAuth token endpoint must not target private or local addresses".to_owned())
        },
        Host::Ipv4(_) => Ok(()),
        Host::Ipv6(ip) if forbidden_token_endpoint_ip(IpAddr::V6(ip)) => {
            Err("OAuth token endpoint must not target private or local addresses".to_owned())
        },
        Host::Ipv6(_) => Ok(()),
    }
}

fn forbidden_token_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        },
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return forbidden_token_endpoint_ip(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || matches!(first & 0xfe00, 0xfc00)
                || matches!(first & 0xffc0, 0xfe80)
                || matches!(first & 0xffc0, 0xfec0)
        },
    }
}

/// Interpret a raw `(status, body_bytes)` pair from the transport.
///
/// SEC-01: `body_bytes` is already bounded to `max_response_bytes` by the
/// transport (mechanical enforcement); this function only interprets what it
/// receives.
/// SEC-02: non-2xx bodies run through `oauth_token_error_summary` before any
/// string value enters `TokenRefreshError`.
fn parse_token_response_bytes(status: u16, body: &[u8]) -> Result<Value, TokenRefreshError> {
    if status >= 400 {
        // Non-2xx: interpret as text for redaction-aware summarization.
        let body_text = String::from_utf8_lossy(body).into_owned();
        let summary = oauth_token_error_summary(&body_text);
        return Err(TokenRefreshError::TokenEndpoint {
            status: status.to_string(),
            summary,
        });
    }
    serde_json::from_slice(body).map_err(|e| TokenRefreshError::Parse(e.to_string()))
}

fn update_state_from_token_response(
    state: &mut OAuth2State,
    body: &Value,
) -> Result<(), TokenRefreshError> {
    let Some(token) = body.get("access_token").and_then(Value::as_str) else {
        return Err(TokenRefreshError::MissingAccessToken);
    };
    state.access_token = SecretString::new(token);

    if let Some(token_type) = body.get("token_type").and_then(Value::as_str) {
        state.token_type = token_type.to_owned();
    }
    if let Some(refresh_token) = body.get("refresh_token").and_then(Value::as_str) {
        state.refresh_token = Some(SecretString::new(refresh_token));
    }
    if let Some(expires_in) = body.get("expires_in").and_then(Value::as_u64) {
        let secs = i64::try_from(expires_in).map_err(|_| {
            TokenRefreshError::Parse(
                "invalid token response: 'expires_in' exceeds supported range".to_owned(),
            )
        })?;
        state.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs));
    }
    if let Some(scope) = body.get("scope").and_then(Value::as_str) {
        state.scopes = scope.split_whitespace().map(str::to_owned).collect();
    }

    Ok(())
}

/// Redacts common sensitive-field-name=value patterns in IdP error strings.
///
/// OAuth2 IdPs (per RFC 6749) sometimes echo submitted credentials inside
/// `error_description` — most commonly `refresh_token=<value>` on
/// `invalid_grant` from buggy or hostile providers. Without redaction those
/// values reach operator-facing logs / SIEM via
/// [`TokenRefreshError::TokenEndpoint`]. This helper scrubs the value while
/// preserving the structural diagnostic.
///
/// Implements redaction discipline at the source: the summary string emitted
/// by [`oauth_token_error_summary`] is the single chokepoint through which
/// non-2xx response bodies enter the error type, so applying redaction here
/// covers all downstream renderings (`Display`, `tracing`, audit events) by
/// construction.
///
/// # Pattern stability + non-panicking init
///
/// [`REDACTION_PATTERN`] is the source-of-truth pattern. We attempt
/// `Regex::new` in a `OnceLock<Option<Regex>>` so a malformed pattern
/// (theoretically possible if a future PR edits the literal incorrectly)
/// never panics in library code — instead the helper returns a conservative
/// full-redaction sentinel `[REDACTED]`, which is over-redaction (not
/// under-redaction; never leaks the input). The CI safety net
/// (`tests::redaction_regex_compiles` lib test) catches the pattern
/// regression before merge so the fallback path is never reached in
/// production.
const REDACTION_PATTERN: &str = r"(?i)\b(refresh[_-]?token|access[_-]?token|client[_-]?secret|bearer|api[_-]?key|password|secret)\s*[=:]\s*\S+";

fn redact_sensitive_fields(input: &str) -> std::borrow::Cow<'_, str> {
    use std::sync::OnceLock;

    static REDACTION_RE: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let re = REDACTION_RE.get_or_init(|| regex::Regex::new(REDACTION_PATTERN).ok());

    match re {
        Some(re) => re.replace_all(input, "$1=[REDACTED]"),
        None => std::borrow::Cow::Borrowed("[REDACTED]"),
    }
}

/// Sanitizes a raw `error_uri` value from a non-2xx OAuth2 token response
/// before it lands in operator-facing summaries.
///
/// SEC-02 (security hardening 2026-04-27 Stage 3) — without sanitization a
/// compromised / MITM IdP could inject:
/// - Phishing URLs disguised as legitimate help pages (different scheme, host
///   obfuscation),
/// - Control characters (`\x00-\x1F`, `\x7F`) that mangle SIEM rows, inject
///   ANSI escapes into terminal renderings, or break log parsers,
/// - Arbitrarily long values that bloat audit storage.
///
/// Sanitization applies in this order:
/// 1. `Url::parse` — reject anything that is not a valid absolute URL.
/// 2. Scheme allowlist — only `https` survives.
/// 3. Control-char strip — any byte `< 0x20` or `== 0x7F` is rejected.
/// 4. Length cap — `> 256` chars truncates with a `…[truncated]` suffix.
fn sanitize_error_uri(raw: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let parsed = match Url::parse(raw) {
        Ok(u) if u.scheme() == "https" => u,
        _ => return Cow::Borrowed("[invalid_error_uri_redacted]"),
    };
    let s = parsed.to_string();
    if s.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Cow::Borrowed("[control_chars_in_error_uri_redacted]");
    }
    if s.len() > 256 {
        return Cow::Owned(format!("{}…[truncated]", &s[..256]));
    }
    Cow::Owned(s)
}

fn oauth_token_error_summary(body_text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body_text) else {
        return "<non-json body>".to_owned();
    };
    let Some(error) = value.get("error").and_then(Value::as_str) else {
        return "<no error code>".to_owned();
    };

    let mut out = error.to_owned();
    if let Some(desc) = value.get("error_description").and_then(Value::as_str) {
        out.push_str(": ");
        let redacted = redact_sensitive_fields(desc);
        let prefix: Vec<char> = redacted.chars().take(257).collect();
        out.extend(prefix.iter().take(256).copied());
        if prefix.len() > 256 {
            out.push('…');
        }
    }
    if let Some(uri) = value.get("error_uri").and_then(Value::as_str) {
        out.push_str(" (error_uri=");
        out.push_str(&sanitize_error_uri(uri));
        out.push(')');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin: the static [`REDACTION_PATTERN`] compiles AND the helper
    /// uses the regex path (not the over-redaction fallback). With the
    /// non-panicking `OnceLock<Option<Regex>>` shape, a broken pattern
    /// would silently route through the `[REDACTED]` fallback, which is
    /// safer than a panic but hides the regression. This lib-level test
    /// catches the pattern compile + the round-trip at `cargo test --lib`
    /// — fails fast before the `--features rotation` integration matrix.
    #[test]
    fn redaction_regex_compiles() {
        // Pattern parses standalone — fails fast on a malformed edit.
        let _ = regex::Regex::new(REDACTION_PATTERN).expect("REDACTION_PATTERN must compile");

        // Round-trip: helper redacts a known sensitive substring AND
        // preserves surrounding text. If the pattern compiles but the
        // helper falls back to `[REDACTED]` everywhere (Option::None
        // branch), the surrounding-text assertion below catches it.
        let out = redact_sensitive_fields("refresh_token=abc12345 expired");
        assert!(out.contains("[REDACTED]"), "expected redaction: {out}");
        assert!(!out.contains("abc12345"), "secret leaked: {out}");
        assert!(
            out.contains("expired"),
            "surrounding text preserved (regex path, not fallback): {out}"
        );
    }

    fn sample_state() -> OAuth2State {
        OAuth2State {
            access_token: SecretString::new("old-token"),
            token_type: "Bearer".to_owned(),
            refresh_token: Some(SecretString::new("refresh-1")),
            expires_at: None,
            scopes: vec!["read".to_owned()],
            client_id: SecretString::new("client"),
            client_secret: SecretString::new("secret"),
            token_url: "https://example.com/token".to_owned(),
            auth_style: AuthStyle::Header,
        }
    }

    #[test]
    fn update_state_requires_access_token() {
        let mut state = sample_state();
        let body = serde_json::json!({ "token_type": "Bearer" });
        let err = update_state_from_token_response(&mut state, &body).unwrap_err();
        assert!(matches!(err, TokenRefreshError::MissingAccessToken));
    }

    #[test]
    fn update_state_applies_refresh_response_fields() {
        let mut state = sample_state();
        let body = serde_json::json!({
            "access_token": "new-token",
            "token_type": "Bearer",
            "refresh_token": "refresh-2",
            "expires_in": 3600,
            "scope": "read write",
        });
        update_state_from_token_response(&mut state, &body).expect("response should apply");

        assert_eq!(state.access_token.expose_secret(), "new-token");
        assert_eq!(state.token_type, "Bearer");
        assert_eq!(state.scopes, vec!["read".to_owned(), "write".to_owned()]);
        assert_eq!(
            state
                .refresh_token
                .as_ref()
                .expect("refresh token")
                .expose_secret(),
            "refresh-2"
        );
        assert!(state.expires_at.is_some());
    }

    #[test]
    fn token_endpoint_rejects_ipv4_mapped_ipv6_private_addresses() {
        for raw in [
            "https://[::ffff:7f00:1]/token",
            "https://[::ffff:a00:1]/token",
            "https://[::ffff:a9fe:1]/token",
            "https://[ff02::1]/token",
            "https://[fec0::1]/token",
        ] {
            let err = validate_token_endpoint(raw)
                .expect_err("private IPv4-mapped and local IPv6 addresses must be rejected");
            assert!(
                err.to_lowercase().contains("token endpoint"),
                "expected endpoint validation error for {raw}, got: {err}"
            );
        }
    }

    // Redaction / parse tests operate purely on bytes — no network, no reqwest.

    #[test]
    fn parse_token_response_bytes_maps_401_to_token_endpoint_error() {
        let body = b"{\"error\":\"invalid_client\"}";
        let err = parse_token_response_bytes(401, body).expect_err("401 should fail");
        assert!(
            matches!(err, TokenRefreshError::TokenEndpoint { .. }),
            "expected TokenEndpoint, got: {err:?}"
        );
    }

    #[test]
    fn parse_token_response_bytes_maps_invalid_json_to_parse_error() {
        let body = b"not json {";
        let err = parse_token_response_bytes(200, body).expect_err("invalid json should fail");
        assert!(
            matches!(err, TokenRefreshError::Parse(_)),
            "expected Parse, got: {err:?}"
        );
    }

    #[test]
    fn sec13_refresh_token_in_error_description_is_redacted() {
        let body = b"{\"error\":\"invalid_grant\",\"error_description\":\"refresh_token=abc123-leaked expired\"}";
        let err = parse_token_response_bytes(400, body).expect_err("400 should fail");
        let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
            panic!("expected TokenEndpoint, got: {err:?}");
        };
        assert!(
            !summary.contains("abc123-leaked"),
            "secret must not appear in summary: {summary}"
        );
        assert!(summary.contains("[REDACTED]"), "redaction sentinel present");
        assert!(summary.contains("invalid_grant"), "error code preserved");
    }

    #[test]
    fn sec02_error_uri_http_scheme_rejected() {
        let body =
            b"{\"error\":\"invalid_request\",\"error_uri\":\"http://attacker.example/page\"}";
        let err = parse_token_response_bytes(400, body).expect_err("400 should fail");
        let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
            panic!("expected TokenEndpoint");
        };
        assert!(
            summary.contains("[invalid_error_uri_redacted]"),
            "non-https URI must be rejected: {summary}"
        );
        assert!(!summary.contains("attacker.example"));
    }

    #[test]
    fn sec02_error_uri_javascript_scheme_rejected() {
        let body = b"{\"error\":\"invalid_grant\",\"error_uri\":\"javascript:alert(1)\"}";
        let err = parse_token_response_bytes(400, body).expect_err("400 should fail");
        let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
            panic!("expected TokenEndpoint");
        };
        assert!(summary.contains("[invalid_error_uri_redacted]"));
        assert!(!summary.contains("alert"));
    }

    #[test]
    fn sec02_valid_https_uri_passes_through() {
        let body = b"{\"error\":\"invalid_grant\",\"error_uri\":\"https://valid.example/help\"}";
        let err = parse_token_response_bytes(400, body).expect_err("400 should fail");
        let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
            panic!("expected TokenEndpoint");
        };
        assert!(
            summary.contains("https://valid.example/help"),
            "valid https URI preserved: {summary}"
        );
    }

    #[test]
    fn oversized_body_response_interpreted_as_non_json() {
        // Simulate transport delivering a body already capped at max_response_bytes
        // (transport enforcement). A cap-sized non-JSON blob → Parse path.
        let body = vec![b'a'; 1024];
        let err = parse_token_response_bytes(400, &body).expect_err("400 should fail");
        // non-JSON body → "<non-json body>" summary (not a secret leak)
        let TokenRefreshError::TokenEndpoint { summary, .. } = &err else {
            panic!("expected TokenEndpoint");
        };
        assert!(
            summary.contains("<non-json body>"),
            "non-json body: {summary}"
        );
    }
}
