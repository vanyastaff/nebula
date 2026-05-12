//! Observability identity types (spec 18).
//!
//! Includes [W3C Trace Context](https://www.w3.org/TR/trace-context/) `traceparent` /
//! `tracestate` parsing and a serde-stable carrier for persistence and async handoff.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// W3C Trace Context trace-id (128-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub u128);

/// W3C Trace Context parent-id (64-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanId(pub u64);

/// Lowercase W3C `traceparent` HTTP header field-name.
pub const W3C_TRACEPARENT: &str = "traceparent";

/// Lowercase W3C `tracestate` HTTP header field-name.
pub const W3C_TRACESTATE: &str = "tracestate";

/// Maximum length for a `tracestate` header value (W3C §4.3 — list length budget).
pub const TRACESTATE_MAX_BYTES: usize = 512;

/// Errors from parsing or constructing W3C trace context values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum W3cTraceContextError {
    /// `traceparent` string failed structural or semantic validation.
    #[error("invalid traceparent: {reason}")]
    InvalidTraceparent { reason: &'static str },
    /// `tracestate` exceeded the allowed length or contained disallowed bytes.
    #[error("invalid tracestate: {reason}")]
    InvalidTracestate { reason: &'static str },
}

/// Serializable carrier for W3C Trace Context (e.g. control queue, JSON metadata).
///
/// Holds the **exact** validated `traceparent` header value (lowercase hex) and an
/// optional `tracestate`. Both strings are suitable for re-injection as HTTP headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct W3cTraceContext {
    traceparent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracestate: Option<String>,
}

impl W3cTraceContext {
    /// Returns the validated `traceparent` header value.
    #[must_use]
    pub fn traceparent(&self) -> &str {
        self.traceparent.as_str()
    }

    /// Returns the optional `tracestate` header value.
    #[must_use]
    pub fn tracestate(&self) -> Option<&str> {
        self.tracestate.as_deref()
    }

    /// Parse from optional header values. Missing `traceparent` yields `Ok(None)`.
    /// Present but invalid `traceparent` yields `Err`. `tracestate` without `traceparent`
    /// is ignored (returns `Ok(None)`).
    ///
    /// **W3C compliance (RFC §3.3.2):** a malformed `tracestate` must **not** drop a valid
    /// `traceparent`. Therefore validation order is `traceparent` first, then `tracestate`; if
    /// `tracestate` is present but invalid, it is silently dropped while the validated
    /// `traceparent` is preserved. `nebula-core` has no `tracing` dependency (cross-cutting
    /// layer), so the drop is not logged here — the HTTP-edge middleware and queue-stamp paths
    /// validate `tracestate` separately and surface their own `WARN` events.
    pub fn from_optional_headers(
        traceparent: Option<&str>,
        tracestate: Option<&str>,
    ) -> Result<Option<Self>, W3cTraceContextError> {
        let Some(tp) = traceparent.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let traceparent = parse_and_canonicalize_traceparent(tp)?;
        let ts = tracestate
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| Self::validate_tracestate(s).ok().map(str::to_owned));
        Ok(Some(Self {
            traceparent,
            tracestate: ts,
        }))
    }

    /// Parse a single `traceparent` value (no `tracestate`).
    pub fn from_traceparent_str(traceparent: &str) -> Result<Self, W3cTraceContextError> {
        let traceparent = parse_and_canonicalize_traceparent(traceparent.trim())?;
        Ok(Self {
            traceparent,
            tracestate: None,
        })
    }

    /// Build from decoded identifiers (version `00` only). Rejects all-zero trace or parent id.
    pub fn from_trace_ids(
        trace_id: TraceId,
        parent_span_id: SpanId,
        trace_flags: u8,
    ) -> Result<Self, W3cTraceContextError> {
        if trace_id.0 == 0 {
            return Err(W3cTraceContextError::InvalidTraceparent {
                reason: "trace_id must not be all zeros",
            });
        }
        if parent_span_id.0 == 0 {
            return Err(W3cTraceContextError::InvalidTraceparent {
                reason: "parent_id must not be all zeros",
            });
        }
        let traceparent = format!(
            "00-{:032x}-{:016x}-{:02x}",
            trace_id.0, parent_span_id.0, trace_flags
        );
        Ok(Self {
            traceparent,
            tracestate: None,
        })
    }

    /// Attach a validated `tracestate` (must be non-empty after trim).
    #[must_use = "builder methods must be chained or used"]
    pub fn with_tracestate(mut self, tracestate: &str) -> Result<Self, W3cTraceContextError> {
        let ts = Self::validate_tracestate(tracestate.trim())?;
        self.tracestate = Some(ts.into());
        Ok(self)
    }

    fn validate_tracestate(s: &str) -> Result<&str, W3cTraceContextError> {
        if s.is_empty() {
            return Err(W3cTraceContextError::InvalidTracestate {
                reason: "tracestate must not be empty when set",
            });
        }
        if s.len() > TRACESTATE_MAX_BYTES {
            return Err(W3cTraceContextError::InvalidTracestate {
                reason: "tracestate exceeds 512 bytes",
            });
        }
        if !s.is_ascii() {
            return Err(W3cTraceContextError::InvalidTracestate {
                reason: "tracestate must be ASCII",
            });
        }
        // W3C Trace Context §3.3.2: tracestate uses VCHAR + horizontal tab + delimiters; reject
        // ASCII control bytes (CR/LF/NUL/DEL etc.) so header-splitting / log-injection attempts
        // cannot survive round-trip via `Display` or HTTP re-injection.
        if s.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(W3cTraceContextError::InvalidTracestate {
                reason: "tracestate must not contain ASCII control characters",
            });
        }
        Ok(s)
    }
}

/// Parsed view of a validated `traceparent` (version `00` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedTraceparent {
    /// 128-bit trace id.
    pub trace_id: TraceId,
    /// 64-bit parent span id from the header.
    pub parent_span_id: SpanId,
    /// Trace-flags byte (lower 8 bits; header carries two hex digits).
    pub trace_flags: u8,
}

/// Parse `traceparent` into structured fields after the same validation as [`W3cTraceContext`].
pub fn parse_traceparent(traceparent: &str) -> Result<ParsedTraceparent, W3cTraceContextError> {
    let canonical = parse_and_canonicalize_traceparent(traceparent.trim())?;
    let parsed = parse_traceparent_parts(&canonical)?;
    Ok(parsed)
}

fn parse_and_canonicalize_traceparent(s: &str) -> Result<String, W3cTraceContextError> {
    let parsed = parse_traceparent_parts(s)?;
    Ok(format!(
        "00-{:032x}-{:016x}-{:02x}",
        parsed.trace_id.0, parsed.parent_span_id.0, parsed.trace_flags
    ))
}

fn is_lower_hex_digit(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='f')
}

fn parse_traceparent_parts(s: &str) -> Result<ParsedTraceparent, W3cTraceContextError> {
    if s.len() != 55 {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "traceparent must be 55 characters",
        });
    }
    // Reject non-ASCII bytes BEFORE slicing `&str` at fixed offsets — otherwise a malicious
    // but length-55 multibyte input would land slice boundaries inside a UTF-8 codepoint and
    // panic. The lowercase-hex checks below catch all *valid* ASCII shapes that survive here.
    if !s.is_ascii() {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "traceparent must be ASCII",
        });
    }
    let bytes = s.as_bytes();
    if bytes[2] != b'-' || bytes[35] != b'-' || bytes[52] != b'-' {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "traceparent must use '-' separators at fixed positions",
        });
    }
    let version = &s[..2];
    if version != "00" {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "only traceparent version 00 is supported",
        });
    }
    if !version.chars().all(is_lower_hex_digit) {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "version must be lowercase hex",
        });
    }

    let trace_id_str = &s[3..35];
    let parent_id_str = &s[36..52];
    let flags_str = &s[53..55];

    if !trace_id_str.chars().all(is_lower_hex_digit) {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "trace_id must be 32 lowercase hex digits",
        });
    }
    if !parent_id_str.chars().all(is_lower_hex_digit) {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "parent_id must be 16 lowercase hex digits",
        });
    }
    if !flags_str.chars().all(is_lower_hex_digit) {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "flags must be 2 lowercase hex digits",
        });
    }

    let trace_id = u128::from_str_radix(trace_id_str, 16).map_err(|_| {
        W3cTraceContextError::InvalidTraceparent {
            reason: "trace_id parse failed",
        }
    })?;
    if trace_id == 0 {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "trace_id must not be all zeros",
        });
    }

    let parent = u64::from_str_radix(parent_id_str, 16).map_err(|_| {
        W3cTraceContextError::InvalidTraceparent {
            reason: "parent_id parse failed",
        }
    })?;
    if parent == 0 {
        return Err(W3cTraceContextError::InvalidTraceparent {
            reason: "parent_id must not be all zeros",
        });
    }

    let trace_flags = u8::from_str_radix(flags_str, 16).map_err(|_| {
        W3cTraceContextError::InvalidTraceparent {
            reason: "flags parse failed",
        }
    })?;

    Ok(ParsedTraceparent {
        trace_id: TraceId(trace_id),
        parent_span_id: SpanId(parent),
        trace_flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_traceparent_round_trip() {
        let tp = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let ctx = W3cTraceContext::from_traceparent_str(tp).expect("valid");
        assert_eq!(ctx.traceparent(), tp);
        let parsed = parse_traceparent(tp).expect("parse");
        assert_eq!(parsed.trace_id.0, 0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c);
        assert_eq!(parsed.parent_span_id.0, 0xb7ad_6b71_6920_3331_u64);
        assert_eq!(parsed.trace_flags, 0x01);
    }

    #[test]
    fn uppercase_hex_rejected() {
        let tp = "00-0AF7651916CD43DD8448EB211C80319C-B7AD6B7169203331-01";
        let err = W3cTraceContext::from_traceparent_str(tp).expect_err("uppercase rejected");
        assert!(
            matches!(err, W3cTraceContextError::InvalidTraceparent { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn reject_all_zero_trace_id() {
        let tp = "00-00000000000000000000000000000000-b7ad6b7169203331-01";
        let err = W3cTraceContext::from_traceparent_str(tp).expect_err("zeros");
        assert!(
            matches!(err, W3cTraceContextError::InvalidTraceparent { reason } if reason.contains("trace_id"))
        );
    }

    #[test]
    fn reject_all_zero_parent_id() {
        let tp = "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01";
        let err = W3cTraceContext::from_traceparent_str(tp).expect_err("zeros");
        assert!(
            matches!(err, W3cTraceContextError::InvalidTraceparent { reason } if reason.contains("parent"))
        );
    }

    #[test]
    fn missing_traceparent_yields_none() {
        let out = W3cTraceContext::from_optional_headers(None, Some("k=v")).expect("ok");
        assert!(out.is_none());
    }

    #[test]
    fn from_trace_ids_builds_expected_string() {
        let ctx = W3cTraceContext::from_trace_ids(
            TraceId(0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c),
            SpanId(0xb7ad_6b71_6920_3331),
            1,
        )
        .expect("build");
        assert_eq!(
            ctx.traceparent(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );
    }

    #[test]
    fn tracestate_too_long_drops_silently_when_paired_with_valid_traceparent() {
        // W3C §3.3.2 — malformed tracestate must NOT drop a valid traceparent. Direct
        // `validate_tracestate` still surfaces the typed error for callers that opt in.
        let too_long = "a".repeat(TRACESTATE_MAX_BYTES + 1);
        let out = W3cTraceContext::from_optional_headers(
            Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
            Some(&too_long),
        )
        .expect("traceparent must be preserved")
        .expect("Some");
        assert_eq!(
            out.traceparent(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
        );
        assert!(
            out.tracestate().is_none(),
            "invalid tracestate must be dropped"
        );
    }

    #[test]
    fn tracestate_with_control_chars_rejected() {
        // Reject `\r`, `\n`, NUL, DEL embedded *inside* the value (trim would strip them off the
        // ends). Defends against header-splitting / log-injection that survives `Display`
        // round-trip into HTTP headers or structured logs.
        for poison in ["a=b\r\nc=d", "a=\0b", "a=b\x7fc", "a\tb=c"] {
            let direct = W3cTraceContext::from_traceparent_str(
                "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            )
            .expect("ok")
            .with_tracestate(poison);
            assert!(
                matches!(
                    direct,
                    Err(W3cTraceContextError::InvalidTracestate { reason })
                        if reason.contains("control")
                ),
                "expected control-char rejection for {poison:?}, got {direct:?}"
            );
        }
    }

    #[test]
    fn non_ascii_traceparent_rejected_without_panic() {
        // Length-55 string of NON-ASCII multibyte chars would normally panic on
        // `&s[..2]` / `&s[3..35]` slice boundaries. The early `is_ascii()` guard returns the
        // typed error instead.
        let s: String = "ä".repeat(55 / 2) + "ö"; // length-55 bytes mismatch — but enforces non-ascii
        // Build something length-exactly-55 in bytes by repeating a 2-byte char carefully.
        let s55 = "x".repeat(54) + "ä"; // 54 + 2 = 56; trim to 55? Use exact-55 ascii + 1 nonascii?
        let _ = s;
        let _ = s55;
        // Simpler: take 53 ASCII + one 2-byte char => len = 55 bytes, non-ascii.
        let mixed = format!("{}{}", "x".repeat(53), "ä");
        assert_eq!(mixed.len(), 55, "must be length-55 bytes");
        let err = parse_traceparent(&mixed).expect_err("must reject non-ASCII");
        assert!(
            matches!(err, W3cTraceContextError::InvalidTraceparent { reason } if reason.contains("ASCII")),
            "got {err:?}"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let ctx = W3cTraceContext::from_traceparent_str(
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        )
        .expect("valid")
        .with_tracestate("vendor1=value1")
        .expect("ts");
        let json = serde_json::to_string(&ctx).expect("ser");
        let back: W3cTraceContext = serde_json::from_str(&json).expect("de");
        assert_eq!(back, ctx);
    }
}
