//! Observability identity types (spec 18).

use serde::{Deserialize, Serialize};

/// W3C Trace Context trace-id (128-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub u128);

/// W3C Trace Context parent-id (64-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpanId(pub u64);
