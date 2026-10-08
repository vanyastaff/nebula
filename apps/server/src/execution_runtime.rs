//! App-owned execution topology and supervision of the in-process worker.

use std::{future::Future, time::Duration};

use axum::Router;
use nebula_worker::{WorkerRuntime, WorkerRuntimeError};
use tokio::net::TcpListener;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use crate::compose::{ServerRunError, serve_until_shutdown};

/// Placement of execution workers, independent of transport and endpoint location.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum ExecutionTopology {
    InProcess,
    SeparateWorkers,
}

const WORKER_DRAIN_BUDGET: Duration = Duration::from_secs(20);

/// Starts the admitted worker before accepting HTTP and observes both owners.
#[tracing::instrument(skip_all)]
pub(crate) async fn serve(
    app: Router,
    listener: TcpListener,
    runtime: Option<WorkerRuntime>,
    shutdown: CancellationToken,
    signal: impl Future<Output = ()>,
    http_drain_budget: Duration,
    transport: &'static str,
) -> Result<(), ServerRunError> {
    let _cancel_on_drop = shutdown.clone().drop_guard();
    let local_address = listener.local_addr()?;
    let worker = match runtime {
        Some(runtime) => Some(runtime.start(shutdown.clone()).await?),
        None => None,
    };
    tracing::info!(transport, %local_address, "starting transport");
    let serving = serve_until_shutdown(app, listener, shutdown.clone(), signal, http_drain_budget);
    tokio::pin!(serving);
    let Some(mut worker) = worker else {
        return serving.await;
    };
    tokio::select! {
        http_result = &mut serving => {
            shutdown.cancel();
            let worker_result = drain_worker(&mut worker).await;
            http_result?;
            worker_result
        },
        worker_result = &mut worker => {
            shutdown.cancel();
            let http_result = serving.await;
            // WorkerRuntime reports unsolicited component completion as an
            // error. Its successful completion already requires cancellation.
            worker_result.map_err(ServerRunError::WorkerTask)??;
            http_result
        },
    }
}

async fn drain_worker(
    worker: &mut AbortOnDropHandle<Result<(), WorkerRuntimeError>>,
) -> Result<(), ServerRunError> {
    if let Ok(result) = tokio::time::timeout(WORKER_DRAIN_BUDGET, &mut *worker).await {
        result
            .map_err(ServerRunError::WorkerTask)?
            .map_err(Into::into)
    } else {
        tracing::error!("in-process worker shutdown timed out; aborting and joining its task");
        worker.abort();
        let _ = worker.await;
        Err(ServerRunError::WorkerDrainTimedOut)
    }
}
