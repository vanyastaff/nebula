//! Daemon runtime — background run loop with restart policy.
//!
//! The daemon runtime spawns a background task that runs the resource's
//! [`Daemon::run`] method in a loop, respecting the configured restart
//! policy, max restarts, and backoff duration.
//!
//! # Cancellation model (issues #318 + #323)
//!
//! A `DaemonRuntime` owns a *parent* cancel token (`parent_cancel`) that is
//! only cancelled by the enclosing `Manager` at shutdown. It is never touched
//! by `stop()`. Each call to `start()` builds a fresh *per-run* child token
//! (`DaemonRun.cancel`) from the parent and hands it to the spawned task.
//! `stop()` cancels that per-run token only — the parent (and therefore any
//! future `start()`) is unaffected.
//!
//! The restart-backoff sleep inside `daemon_loop` races against the per-run
//! cancel via a `biased` `tokio::select!`, so `stop()` returns promptly even
//! when called mid-backoff (#323). Together with the per-run token, a
//! `start → stop → start` sequence (#318) correctly spawns a new daemon loop
//! on a fresh cancel source each time.

use std::{marker::PhantomData, sync::Arc};

use nebula_core::context::Context;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    context::ResourceContext,
    error::Error,
    resource::Resource,
    topology::daemon::{Daemon, RestartPolicy, config::Config},
};

/// Runtime state for a daemon topology.
///
/// Manages a background task that runs the resource's daemon loop,
/// automatically restarting according to the configured [`RestartPolicy`].
///
/// See the module docs for the cancellation model.
pub struct DaemonRuntime<R: Resource> {
    config: Config,
    /// Parent cancel token shared with the enclosing `Manager`. Cancelling
    /// this cascades to every per-run child token, so shutdown of the
    /// manager also shuts down any running daemon. **Never cancelled by
    /// `stop()`** — that would permanently brick the runtime.
    parent_cancel: CancellationToken,
    /// Per-run state. `None` when no daemon is currently running.
    inner: Mutex<Option<DaemonRun>>,
    _phantom: PhantomData<R>,
}

/// State for a single active daemon run.
struct DaemonRun {
    /// Per-run cancel token; a child of `parent_cancel` so manager shutdown
    /// still propagates. Cancelled by `stop()` to tear down just this run.
    cancel: CancellationToken,
    /// Join handle for the spawned daemon loop task.
    handle: tokio::task::JoinHandle<()>,
}

impl<R: Resource> DaemonRuntime<R> {
    /// Creates a new daemon runtime with the given configuration and
    /// *parent* cancellation token.
    ///
    /// The parent token is used only to cascade global shutdown. Per-run
    /// cancellation is managed internally via child tokens built at each
    /// `start()`.
    pub fn new(config: Config, parent_cancel: CancellationToken) -> Self {
        Self {
            config,
            parent_cancel,
            inner: Mutex::new(None),
            _phantom: PhantomData,
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the parent cancellation token (shared with the manager).
    ///
    /// This token is not cancelled by `stop()`; it represents the outer
    /// lifetime of the runtime, not any individual run. External code may
    /// clone it to observe global shutdown, but must not cancel it to
    /// request daemon stop — use [`stop`](Self::stop) instead.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.parent_cancel
    }

    /// Returns `true` if a daemon task is currently running.
    ///
    /// Returns `false` if no task was ever started, if `stop()` has been
    /// called, or if the task exited naturally (e.g. under
    /// [`RestartPolicy::Never`]).
    pub async fn is_running(&self) -> bool {
        let guard = self.inner.lock().await;
        guard.as_ref().is_some_and(|run| !run.handle.is_finished())
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
    /// per-run cancellation token is triggered (by `stop()` or by parent
    /// shutdown).
    ///
    /// # Restart-safety (#318)
    ///
    /// A stale `DaemonRun` whose task has already finished (natural exit)
    /// is silently dropped here so a fresh `start()` succeeds. Only an
    /// actually-live run returns `Err("daemon is already running")`.
    ///
    /// # Errors
    ///
    /// Returns an error if a daemon is currently running (its join handle
    /// is not finished).
    pub async fn start(
        &self,
        resource: R,
        runtime: Arc<R::Runtime>,
        ctx: &ResourceContext,
    ) -> Result<(), Error> {
        let mut guard = self.inner.lock().await;

        // #318: if a prior run has already finished (e.g. RestartPolicy::Never
        // and natural exit), its handle sticks around in `inner` forever and
        // blocks future starts. Drop it here so a clean restart succeeds.
        if let Some(run) = guard.as_ref()
            && !run.handle.is_finished()
        {
            return Err(Error::permanent("daemon is already running"));
        }
        // Either guard was None, or the prior run is finished — drop it.
        *guard = None;

        // Fresh per-run cancel token as a child of the parent. External
        // shutdown of `parent_cancel` still propagates here, and `stop()`
        // can cancel this child without touching the parent.
        let cancel = self.parent_cancel.child_token();
        let loop_cancel = cancel.clone();

        let config = self.config.clone();
        let scope = ctx.scope().clone();

        let handle = tokio::spawn(async move {
            daemon_loop(resource, runtime, config, loop_cancel, scope).await;
        });

        *guard = Some(DaemonRun { cancel, handle });
        Ok(())
    }

    /// Stops the daemon, cancelling the background task and awaiting its
    /// completion.
    ///
    /// Idempotent: calling `stop()` when no daemon is running is a no-op.
    /// Only the per-run token is cancelled, so subsequent `start()` calls
    /// work against a fresh cancel source (#318).
    ///
    /// Combined with the `biased` select in `daemon_loop`, this returns
    /// within the per-run join time even if the task was mid-backoff
    /// (#323).
    pub async fn stop(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(run) = guard.as_mut() {
            run.cancel.cancel();
            // Keep the run visible until join completes so concurrent
            // start()/is_running() calls cannot observe a false "stopped"
            // state while shutdown is still in progress.
            if let Err(e) = (&mut run.handle).await {
                tracing::warn!(error = %e, "daemon join error on stop");
            }
            guard.take();
        }
    }
}

/// Core daemon loop extracted to avoid excessive nesting.
///
/// Runs `resource.run()` in a loop with restart logic based on the
/// configured policy. The loop and its restart-backoff sleep both observe
/// `cancel` via `biased` selects so shutdown wins deterministically (#323).
async fn daemon_loop<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    config: Config,
    cancel: CancellationToken,
    scope: nebula_core::scope::Scope,
) where
    R: Daemon + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let mut restarts = 0u32;
    let ctx = ResourceContext::minimal(scope, cancel.clone());

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let result = resource.run(&runtime, &ctx, cancel.clone()).await;

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

        // #323: race the restart-backoff against cancel. `biased` so
        // cancellation wins deterministically — a tight stop() budget must
        // not be paid down by a long backoff.
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(config.restart_backoff) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU32, Ordering},
        time::Duration,
    };

    use nebula_core::{ExecutionId, ResourceKey};

    use super::*;
    use crate::{
        context::ResourceContext,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
        topology::daemon::{Daemon, RestartPolicy, config::Config as DaemonCfg},
    };

    #[derive(Clone, Debug, Default)]
    struct EmptyCfg;

    nebula_schema::impl_empty_has_schema!(EmptyCfg);

    impl ResourceConfig for EmptyCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("daemon-test: {0}")]
    struct TestError(&'static str);

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.to_string())
        }
    }

    #[derive(Clone)]
    struct FlakyDaemon {
        attempts: Arc<AtomicU32>,
    }

    impl Resource for FlakyDaemon {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Auth = ();

        fn key() -> ResourceKey {
            ResourceKey::new("daemon-flaky").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _auth: &(),
            _ctx: &ResourceContext,
        ) -> Result<(), TestError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for FlakyDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
            _cancel: CancellationToken,
        ) -> Result<(), TestError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(TestError("intentional"))
        }
    }

    #[derive(Clone)]
    struct OneShotDaemon;

    impl Resource for OneShotDaemon {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Auth = ();

        fn key() -> ResourceKey {
            ResourceKey::new("daemon-oneshot").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _auth: &(),
            _ctx: &ResourceContext,
        ) -> Result<(), TestError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for OneShotDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
            _cancel: CancellationToken,
        ) -> Result<(), TestError> {
            Ok(())
        }
    }

    /// #323: `stop()` called while the daemon is sleeping in `restart_backoff`
    /// must return promptly. Without the `biased select` at the bottom of
    /// `daemon_loop`, stop would be blocked for the full backoff.
    #[tokio::test]
    async fn stop_during_restart_backoff_returns_promptly() {
        let parent = CancellationToken::new();
        let cfg = DaemonCfg {
            restart_policy: RestartPolicy::Always,
            restart_backoff: Duration::from_secs(10),
            max_restarts: 100,
        };
        let rt = DaemonRuntime::<FlakyDaemon>::new(cfg, parent);
        let resource = FlakyDaemon {
            attempts: Arc::new(AtomicU32::new(0)),
        };
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        );

        rt.start(resource, Arc::new(()), &ctx).await.unwrap();

        // Give the daemon time for the first run() to fail and enter the
        // 10s backoff sleep.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        rt.stop().await;
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "stop() during restart_backoff must return promptly, took {elapsed:?}",
        );
        assert!(!rt.is_running().await);
    }

    /// #318: `start → stop → start` must succeed. The per-run child token
    /// + inner handle cleanup in `start()` are what makes this work.
    #[tokio::test]
    async fn start_stop_start_lifecycle() {
        let parent = CancellationToken::new();
        let cfg = DaemonCfg {
            restart_policy: RestartPolicy::Always,
            restart_backoff: Duration::from_millis(20),
            max_restarts: 100,
        };
        let rt = DaemonRuntime::<FlakyDaemon>::new(cfg, parent);
        let resource = FlakyDaemon {
            attempts: Arc::new(AtomicU32::new(0)),
        };
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        );

        rt.start(resource.clone(), Arc::new(()), &ctx)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        rt.stop().await;
        assert!(!rt.is_running().await);

        rt.start(resource, Arc::new(()), &ctx)
            .await
            .expect("start after stop must succeed");
        assert!(rt.is_running().await);
        rt.stop().await;
    }

    /// #318: `start → natural-exit → start` must succeed too. Under
    /// `RestartPolicy::Never` the task exits after one run; the stale
    /// finished handle in `inner` must not block the next `start()`.
    #[tokio::test]
    async fn start_natural_exit_start_lifecycle() {
        let parent = CancellationToken::new();
        let cfg = DaemonCfg {
            restart_policy: RestartPolicy::Never,
            restart_backoff: Duration::from_millis(10),
            max_restarts: 0,
        };
        let rt = DaemonRuntime::<OneShotDaemon>::new(cfg, parent);
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        );

        rt.start(OneShotDaemon, Arc::new(()), &ctx).await.unwrap();

        // Wait until the run future has resolved and the join handle is
        // finished. 250 ms with 5 ms polls is generous.
        for _ in 0..50 {
            if !rt.is_running().await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!rt.is_running().await);

        rt.start(OneShotDaemon, Arc::new(()), &ctx)
            .await
            .expect("start after natural exit must succeed");
    }
}
