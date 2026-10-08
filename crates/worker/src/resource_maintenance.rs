//! Worker-owned retirement of deleted stored resources, independent of diagnostics.

use std::{sync::Arc, time::Duration};

use nebula_engine::WorkflowEngine;
use tokio_util::sync::CancellationToken;

/// The engine bounds each sweep and preserves rows whose storage state is unknown.
/// The worker owns scheduling and cancellation, including a pending storage read.
#[tracing::instrument(name = "worker.resource_maintenance", skip_all)]
pub(super) async fn run(engine: Arc<WorkflowEngine>, shutdown: CancellationToken) {
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            () = engine.retire_deleted_resources() => {},
        }

        // Delay after work (no catch-up burst); jitter avoids fleet-wide scans
        // repeatedly landing on the same deployment database at the same time.
        let delay = Duration::from_millis(rand::random_range(8_000..=12_000));
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(delay) => {},
        }
    }
}
