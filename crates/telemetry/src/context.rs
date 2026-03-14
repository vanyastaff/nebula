//! W3C Trace Context types for distributed tracing.
//!
//! Implements the [W3C Trace Context](https://www.w3.org/TR/trace-context/)
//! `traceparent` header format for propagating trace identity across service
//! boundaries.
//!
//! ## Format
//!
//! ```text
//! traceparent: 00-{trace_id:032x}-{span_id:016x}-{flags:02x}
//! ```

use std::fmt;
use std::str::FromStr;

use rand::RngExt;
use serde::{Deserialize, Serialize};

// ── TraceId ──────────────────────────────────────────────────────────────────

/// W3C trace identifier (16 bytes / 128 bits), displayed as 32-char lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub u128);

impl TraceId {
    /// Generate a random trace ID.
    #[must_use]
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        // Ensure non-zero (W3C spec: all-zero is invalid)
        loop {
            let id: u128 = rng.random();
            if id != 0 {
                return Self(id);
            }
        }
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl FromStr for TraceId {
    type Err = TraceContextError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 {
            return Err(TraceContextError::InvalidFormat(
                "trace-id must be 32 hex characters".into(),
            ));
        }
        let val = u128::from_str_radix(s, 16)
            .map_err(|_| TraceContextError::InvalidFormat("invalid hex in trace-id".into()))?;
        if val == 0 {
            return Err(TraceContextError::InvalidFormat(
                "trace-id must not be all zeros".into(),
            ));
        }
        Ok(Self(val))
    }
}

// ── SpanId ───────────────────────────────────────────────────────────────────

/// W3C span identifier (8 bytes / 64 bits), displayed as 16-char lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanId(pub u64);

impl SpanId {
    /// Generate a random span ID.
    #[must_use]
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        loop {
            let id: u64 = rng.random();
            if id != 0 {
                return Self(id);
            }
        }
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl FromStr for SpanId {
    type Err = TraceContextError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 16 {
            return Err(TraceContextError::InvalidFormat(
                "span-id must be 16 hex characters".into(),
            ));
        }
        let val = u64::from_str_radix(s, 16)
            .map_err(|_| TraceContextError::InvalidFormat("invalid hex in span-id".into()))?;
        if val == 0 {
            return Err(TraceContextError::InvalidFormat(
                "span-id must not be all zeros".into(),
            ));
        }
        Ok(Self(val))
    }
}

// ── TraceContext ─────────────────────────────────────────────────────────────

/// W3C Trace Context — carries trace identity for distributed tracing.
///
/// Supports round-tripping through the `traceparent` header format:
/// `{version:02x}-{trace_id:032x}-{span_id:016x}-{flags:02x}`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    /// The trace identifier (shared across all spans in a trace).
    pub trace_id: TraceId,
    /// The span identifier (unique per span).
    pub span_id: SpanId,
    /// The parent span identifier (if this span has a parent).
    pub parent_span_id: Option<SpanId>,
    /// Whether this trace is sampled.
    pub sampled: bool,
}

impl TraceContext {
    /// Generate a new trace context with random IDs and sampling enabled.
    #[must_use]
    pub fn generate() -> Self {
        Self {
            trace_id: TraceId::generate(),
            span_id: SpanId::generate(),
            parent_span_id: None,
            sampled: true,
        }
    }

    /// Parse a W3C `traceparent` header value.
    ///
    /// Format: `{version:02x}-{trace_id:032x}-{span_id:016x}-{flags:02x}`
    ///
    /// # Errors
    ///
    /// Returns [`TraceContextError`] if the header is malformed.
    pub fn from_traceparent(header: &str) -> Result<Self, TraceContextError> {
        let parts: Vec<&str> = header.split('-').collect();
        if parts.len() != 4 {
            return Err(TraceContextError::InvalidFormat(
                "traceparent must have 4 dash-separated fields".into(),
            ));
        }

        let version = u8::from_str_radix(parts[0], 16)
            .map_err(|_| TraceContextError::InvalidFormat("invalid version hex".into()))?;

        // Version 255 (0xff) is forbidden by the spec
        if version == 0xff {
            return Err(TraceContextError::InvalidFormat(
                "version ff is invalid".into(),
            ));
        }

        let trace_id: TraceId = parts[1].parse()?;
        let span_id: SpanId = parts[2].parse()?;

        let flags = u8::from_str_radix(parts[3], 16)
            .map_err(|_| TraceContextError::InvalidFormat("invalid flags hex".into()))?;
        let sampled = (flags & 0x01) != 0;

        Ok(Self {
            trace_id,
            span_id,
            parent_span_id: None,
            sampled,
        })
    }

    /// Serialize to a W3C `traceparent` header value.
    #[must_use]
    pub fn to_traceparent(&self) -> String {
        let flags: u8 = if self.sampled { 0x01 } else { 0x00 };
        format!("00-{}-{}-{:02x}", self.trace_id, self.span_id, flags)
    }

    /// Create a child span context inheriting the trace ID.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id,
            span_id: SpanId::generate(),
            parent_span_id: Some(self.span_id),
            sampled: self.sampled,
        }
    }
}

impl fmt::Display for TraceContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_traceparent())
    }
}

impl FromStr for TraceContext {
    type Err = TraceContextError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_traceparent(s)
    }
}

// ── TraceContextError ────────────────────────────────────────────────────────

/// Error parsing a trace context header.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum TraceContextError {
    /// The traceparent header has an invalid format.
    #[error("invalid traceparent format: {0}")]
    InvalidFormat(String),
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_valid_context() {
        let ctx = TraceContext::generate();
        assert_ne!(ctx.trace_id.0, 0);
        assert_ne!(ctx.span_id.0, 0);
        assert!(ctx.sampled);
        assert!(ctx.parent_span_id.is_none());
    }

    #[test]
    fn traceparent_roundtrip() {
        let ctx = TraceContext::generate();
        let header = ctx.to_traceparent();
        let parsed = TraceContext::from_traceparent(&header).unwrap();
        assert_eq!(ctx.trace_id, parsed.trace_id);
        assert_eq!(ctx.span_id, parsed.span_id);
        assert_eq!(ctx.sampled, parsed.sampled);
    }

    #[test]
    fn parse_valid_traceparent() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-01";
        let ctx = TraceContext::from_traceparent(header).unwrap();
        assert_eq!(
            ctx.trace_id,
            TraceId(0x4bf9_2f35_77b3_4da6_a3ce_929d_0e0e_4736)
        );
        assert_eq!(ctx.span_id, SpanId(0x00f0_67aa_0bb9_02b7));
        assert!(ctx.sampled);
    }

    #[test]
    fn parse_unsampled_traceparent() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-00";
        let ctx = TraceContext::from_traceparent(header).unwrap();
        assert!(!ctx.sampled);
    }

    #[test]
    fn parse_invalid_traceparent_too_few_parts() {
        let result = TraceContext::from_traceparent("00-abcd-01");
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_traceparent_bad_trace_id() {
        let header = "00-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ-00f067aa0bb902b7-01";
        assert!(TraceContext::from_traceparent(header).is_err());
    }

    #[test]
    fn parse_invalid_traceparent_zero_trace_id() {
        let header = "00-00000000000000000000000000000000-00f067aa0bb902b7-01";
        assert!(TraceContext::from_traceparent(header).is_err());
    }

    #[test]
    fn parse_invalid_traceparent_zero_span_id() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01";
        assert!(TraceContext::from_traceparent(header).is_err());
    }

    #[test]
    fn display_shows_traceparent_format() {
        let ctx = TraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-01",
        )
        .unwrap();
        assert_eq!(
            ctx.to_string(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-01"
        );
    }

    #[test]
    fn serialization_roundtrip() {
        let ctx = TraceContext::generate();
        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: TraceContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, parsed);
    }

    #[test]
    fn child_inherits_trace_id() {
        let parent = TraceContext::generate();
        let child = parent.child();
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.parent_span_id, Some(parent.span_id));
        assert_ne!(child.span_id, parent.span_id);
        assert_eq!(child.sampled, parent.sampled);
    }

    #[test]
    fn version_ff_is_rejected() {
        let header = "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-01";
        assert!(TraceContext::from_traceparent(header).is_err());
    }

    #[test]
    fn from_str_works_same_as_from_traceparent() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0bb902b7-01";
        let a = TraceContext::from_traceparent(header).unwrap();
        let b: TraceContext = header.parse().unwrap();
        assert_eq!(a, b);
    }
}
