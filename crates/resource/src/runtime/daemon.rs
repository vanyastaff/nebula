//! Daemon runtime — background run loop with restart policy.
//!
//! The daemon runtime spawns a background task that runs the resource's
//! [`Daemon::run`] method in a loop, respecting the configured restart
//! policy, max restarts, and backoff duration.

use std::marker::PhantomData;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::ctx::Ctx;
use crate::error::Error;
use crate::resource::Resource;
use crate::topology::daemon::config::Config;
use crate::topology::daemon::{Daemon, RestartPolicy};

/// Runtime state for a daemon topology.
///
/// Manages a background task that runs the resource's daemon loop,
/// automatically restarting according to the configured [`RestartPolicy`].
pub struct DaemonRuntime<R: Resource> {
    config: Config,
    cancel: CancellationToken,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    _phantom: PhantomData<R>,
}

impl<R: Resource> DaemonRuntime<R> {
    /// Creates a new daemon runtime with the given configuration and
    /// cancellation token.
    pub fn new(config: Config, cancel: CancellationToken) -> Self {
        Self {
            config,
            cancel,
            handle: Mutex::new(None),
            _phantom: PhantomData,
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the cancellation token used to stop this daemon.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl<R> DaemonRuntime<R>
where
    R: Daemon + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    /// Starts the daemon background task.
    ///
    /// Spawns a tokio task that runs `resource.run()` in a loop, respecting
    /// the restart policy:
    ///
    /// - [`RestartPolicy::Never`] — exits after the first run.
    /// - [`RestartPolicy::OnFailure`] — restarts only if `run()` returns `Err`.
    /// - [`RestartPolicy::Always`] — restarts regardless of exit reason.
    ///
    /// The loop stops after `max_restarts` consecutive restarts or when the
    /// cancellation token is triggered.
    ///
    /// # Errors
    ///
    /// Returns an error if a daemon is already running.
    pub async fn start(
        &self,
        resource: R,
        runtime: Arc<R::Runtime>,
        ctx: &dyn Ctx,
    ) -> Result<(), Error> {
        let mut guard = self.handle.lock().await;
        if guard.is_some() {
            return Err(Error::permanent("daemon is already running"));
        }

        let cancel = self.cancel.clone();
        let config = self.config.clone();
        let cancel_token_for_run = self.cancel.child_token();
        let execution_id = *ctx.execution_id();

        let join_handle = tokio::spawn(async move {
            daemon_loop(
                resource,
                runtime,
                config,
                cancel,
                cancel_token_for_run,
                execution_id,
            )
            .await;
        });

        *guard = Some(join_handle);
        Ok(())
    }

    /// Stops the daemon, cancelling the background task and awaiting its
    /// completion.
    ///
    /// This is idempotent — calling `stop()` when no daemon is running is
    /// a no-op.
    pub async fn stop(&self) {
        self.cancel.cancel();
        let handle = self.handle.lock().await.take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }
}

/// Core daemon loop extracted to avoid excessive nesting.
///
/// Runs `resource.run()` in a loop with restart logic based on the
/// configured policy. Uses a child cancellation token for each run
/// iteration so individual runs can be cancelled independently.
async fn daemon_loop<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    config: Config,
    cancel: CancellationToken,
    run_cancel: CancellationToken,
    execution_id: nebula_core::ExecutionId,
) where
    R: Daemon + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let mut restarts = 0u32;
    let ctx = crate::ctx::BasicCtx::new(execution_id).with_cancel_token(run_cancel.clone());

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let result = resource.run(&runtime, &ctx, run_cancel.clone()).await;

        if cancel.is_cancelled() {
            break;
        }

        let should_restart = match (&config.restart_policy, &result) {
            (RestartPolicy::Never, _) => false,
            (RestartPolicy::OnFailure, Ok(())) => false,
            (RestartPolicy::OnFailure, Err(_)) => true,
            (RestartPolicy::Always, _) => true,
        };

        if !should_restart {
            break;
        }

        restarts += 1;
        if restarts > config.max_restarts {
            tracing::warn!(
                restarts,
                max = config.max_restarts,
                "daemon exceeded max restarts, stopping"
            );
            break;
        }

        if let Err(ref e) = result {
            tracing::warn!(
                restart = restarts,
                error = %e,
                "daemon restarting after error"
            );
        }

        tokio::time::sleep(config.restart_backoff).await;
    }
}
