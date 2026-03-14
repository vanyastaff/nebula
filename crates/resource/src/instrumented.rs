//! Instrumented guard that records Tier 1 usage on drop.
//!
//! The kernel wraps every acquired resource in [`InstrumentedGuard`]. When the
//! guard is dropped, it records [`ResourceUsageRecord`] (resource_key, timing,
//! drop_reason) via the context's [`Recorder`].

use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use nebula_core::ResourceKey;
use nebula_telemetry::{DropReason, Recorder, ResourceUsageRecord};

use crate::manager_guard::AnyGuardTrait;

/// Wraps an [`AnyGuard`](crate::manager_guard::AnyGuard) and records Tier 1
/// usage when dropped or when [`into_inner`](InstrumentedGuard::into_inner) is used.
///
/// Drop reason is set automatically:
/// - [`DropReason::Released`] — normal drop
/// - [`DropReason::Panic`] — dropped while unwinding (`std::thread::panicking()`)
/// - [`DropReason::Detached`] — guard consumed via `into_inner()`
pub struct InstrumentedGuard {
    inner: Option<Box<dyn AnyGuardTrait>>,
    resource_key: ResourceKey,
    acquired_at: Instant,
    wait_duration: std::time::Duration,
    recorder: Arc<dyn Recorder>,
}

impl InstrumentedGuard {
    /// Create a new instrumented guard. The caller must pass the guard returned
    /// by the pool (or wrapped with release hooks), plus timing and recorder.
    #[must_use]
    pub fn new(
        inner: Box<dyn AnyGuardTrait>,
        resource_key: ResourceKey,
        acquired_at: Instant,
        wait_duration: std::time::Duration,
        recorder: Arc<dyn Recorder>,
    ) -> Self {
        Self {
            inner: Some(inner),
            resource_key,
            acquired_at,
            wait_duration,
            recorder,
        }
    }

    /// Consume this guard and return the inner guard. Records usage with
    /// [`DropReason::Detached`] (caller is responsible for dropping the inner guard).
    #[allow(dead_code)] // public API for Detached recording
    pub fn into_inner(mut self) -> Box<dyn AnyGuardTrait> {
        let inner = self
            .inner
            .take()
            .expect("InstrumentedGuard::into_inner called twice");
        let hold_duration = self.acquired_at.elapsed();
        self.recorder.record_usage(ResourceUsageRecord {
            resource_key: self.resource_key.clone(),
            acquired_at: self.acquired_at,
            wait_duration: self.wait_duration,
            hold_duration,
            drop_reason: DropReason::Detached,
        });
        inner
    }
}

impl AnyGuardTrait for InstrumentedGuard {
    fn as_any(&self) -> &dyn Any {
        self.inner
            .as_ref()
            .expect("InstrumentedGuard used after into_inner")
            .as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner
            .as_mut()
            .expect("InstrumentedGuard used after into_inner")
            .as_any_mut()
    }
}

impl Drop for InstrumentedGuard {
    fn drop(&mut self) {
        let inner = match self.inner.take() {
            Some(guard) => guard,
            None => return,
        };
        let hold_duration = self.acquired_at.elapsed();
        let drop_reason = if std::thread::panicking() {
            DropReason::Panic
        } else {
            DropReason::Released
        };
        self.recorder.record_usage(ResourceUsageRecord {
            resource_key: self.resource_key.clone(),
            acquired_at: self.acquired_at,
            wait_duration: self.wait_duration,
            hold_duration,
            drop_reason,
        });
        drop(inner);
    }
}

impl std::fmt::Debug for InstrumentedGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentedGuard")
            .field("resource_key", &self.resource_key)
            .field("acquired_at", &self.acquired_at)
            .finish_non_exhaustive()
    }
}
