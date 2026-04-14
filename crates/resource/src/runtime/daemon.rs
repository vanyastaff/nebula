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

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    ctx::Ctx,
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
        guard
            .as_ref()
            .is_some_and(|run| !run.handle.is_finished())
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
        ctx: &dyn Ctx,
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
        let execution_id = *ctx.execution_id();

        let handle = tokio::spawn(async move {
            daemon_loop(resource, runtime, config, loop_cancel, execution_id).await;
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
        let run = {
            let mut guard = self.inner.lock().await;
            guard.take()
        };
        if let Some(run) = run {
            run.cancel.cancel();
            // User rule: no `let _ = ` on Result. Treat a join error as a
            // diagnostic warning rather than silent swallowing.
            if let Err(e) = run.handle.await {
                tracing::warn!(error = %e, "daemon join error on stop");
            }
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
    execution_id: nebula_core::ExecutionId,
) where
    R: Daemon + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let mut restarts = 0u32;
    let ctx = crate::ctx::BasicCtx::new(execution_id).with_cancel_token(cancel.clone());

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

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        ctx::BasicCtx,
        resource::{Resource, ResourceMetadata},
        topology::daemon::config::Config as DaemonConfig,
    };

    #[derive(Debug, Clone)]
    struct TestError(String);

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for TestError {}

    impl From<TestError> for Error {
        fn from(e: TestError) -> Self {
            Error::transient(e.0)
        }
    }

    // `ResourceConfig for ()` is already provided elsewhere in the crate
    // (see `handle.rs`), so no blanket impl is needed here.

    /// A test Daemon whose behaviour can be configured per-test.
    #[derive(Clone)]
    struct TestDaemon {
        run_count: Arc<AtomicU32>,
        mode: Mode,
    }

    #[derive(Clone, Copy)]
    enum Mode {
        /// `run` returns `Ok(())` immediately.
        ExitOk,
        /// `run` returns `Err(...)` immediately — forces backoff under
        /// `RestartPolicy::OnFailure`.
        ExitErr,
    }

    impl Resource for TestDaemon {
        type Config = ();
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("test-daemon")
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _auth: &Self::Auth,
            _ctx: &dyn Ctx,
        ) -> Result<Self::Runtime, Self::Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for TestDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &dyn Ctx,
            _cancel: CancellationToken,
        ) -> Result<(), Self::Error> {
            self.run_count.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                Mode::ExitOk => Ok(()),
                Mode::ExitErr => Err(TestError("daemon test failure".into())),
            }
        }
    }

    /// A daemon whose `run()` blocks on the cancel token — used to force a
    /// "currently running" state for tests that need one.
    #[derive(Clone)]
    struct WaitDaemon;

    impl Resource for WaitDaemon {
        type Config = ();
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("wait-daemon")
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _auth: &Self::Auth,
            _ctx: &dyn Ctx,
        ) -> Result<Self::Runtime, Self::Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for WaitDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &dyn Ctx,
            cancel: CancellationToken,
        ) -> Result<(), Self::Error> {
            cancel.cancelled().await;
            Ok(())
        }
    }

    fn test_ctx() -> BasicCtx {
        BasicCtx::new(ExecutionId::new())
    }

    /// #318: start → stop → start must spawn a fresh run.
    ///
    /// Uses `WaitDaemon` so the first run is definitely live when `stop()`
    /// is called (rather than racing a TestDaemon that returns immediately).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_stop_start_runs_daemon_twice() {
        let config = DaemonConfig {
            restart_policy: RestartPolicy::Never,
            restart_backoff: Duration::from_millis(10),
            max_restarts: 3,
        };
        let runtime: DaemonRuntime<WaitDaemon> =
            DaemonRuntime::new(config, CancellationToken::new());
        let ctx = test_ctx();

        runtime
            .start(WaitDaemon, Arc::new(()), &ctx)
            .await
            .expect("first start must succeed");

        // Wait until the loop actually enters `run` (blocks on cancel).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !runtime.is_running().await && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            runtime.is_running().await,
            "first daemon run did not become live"
        );

        runtime.stop().await;
        assert!(!runtime.is_running().await, "first stop did not clear");

        // Critical assertion: second start must succeed after stop. Before
        // #318 the stored cancel token was already-cancelled and the guard
        // refused the second start because the handle was never cleared.
        runtime
            .start(WaitDaemon, Arc::new(()), &ctx)
            .await
            .expect("second start after stop must succeed (#318)");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !runtime.is_running().await && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            runtime.is_running().await,
            "second daemon run did not become live"
        );

        runtime.stop().await;
        assert!(!runtime.is_running().await);
    }

    /// #318: natural-exit run leaves a finished handle in place; next start
    /// must reuse that slot rather than return "already running".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_after_natural_exit_succeeds() {
        let run_count = Arc::new(AtomicU32::new(0));
        let daemon = TestDaemon {
            run_count: Arc::clone(&run_count),
            mode: Mode::ExitOk,
        };

        let config = DaemonConfig {
            restart_policy: RestartPolicy::Never,
            restart_backoff: Duration::from_millis(10),
            max_restarts: 3,
        };
        let runtime: DaemonRuntime<TestDaemon> =
            DaemonRuntime::new(config, CancellationToken::new());
        let ctx = test_ctx();

        runtime
            .start(daemon.clone(), Arc::new(()), &ctx)
            .await
            .expect("first start");

        // Wait for the daemon to naturally exit.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while runtime.is_running().await && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !runtime.is_running().await,
            "daemon should have exited naturally under RestartPolicy::Never"
        );

        // Second start must succeed — the stale finished handle is cleared.
        runtime
            .start(daemon, Arc::new(()), &ctx)
            .await
            .expect("start after natural exit must succeed");
        runtime.stop().await;
    }

    /// Concurrent `start` calls on a live daemon: exactly one must succeed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_start_rejects_second() {
        let config = DaemonConfig {
            restart_policy: RestartPolicy::Never,
            restart_backoff: Duration::from_millis(10),
            max_restarts: 3,
        };
        let runtime: DaemonRuntime<WaitDaemon> =
            DaemonRuntime::new(config, CancellationToken::new());
        let ctx = test_ctx();

        runtime
            .start(WaitDaemon, Arc::new(()), &ctx)
            .await
            .expect("first start");

        let err = runtime
            .start(WaitDaemon, Arc::new(()), &ctx)
            .await
            .expect_err("second concurrent start must fail");
        assert!(
            err.to_string().contains("already running"),
            "unexpected error: {err}"
        );

        runtime.stop().await;
    }

    /// Parent-cancel propagation: cancelling the parent token externally
    /// (e.g. from manager shutdown) must exit the running daemon.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parent_cancel_propagates_to_running_daemon() {
        let parent = CancellationToken::new();
        let config = DaemonConfig {
            restart_policy: RestartPolicy::Never,
            restart_backoff: Duration::from_millis(10),
            max_restarts: 3,
        };
        let runtime: DaemonRuntime<WaitDaemon> = DaemonRuntime::new(config, parent.clone());
        let ctx = test_ctx();

        runtime
            .start(WaitDaemon, Arc::new(()), &ctx)
            .await
            .expect("start");
        assert!(runtime.is_running().await);

        parent.cancel();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while runtime.is_running().await && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !runtime.is_running().await,
            "parent cancel must cascade to per-run child token"
        );

        runtime.stop().await;
    }

    /// #323: `stop()` during a long restart-backoff must return promptly.
    /// Before the fix, the `tokio::time::sleep(restart_backoff)` did not
    /// race against cancel, so `stop()` blocked for up to `restart_backoff`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_during_backoff_returns_promptly() {
        let run_count = Arc::new(AtomicU32::new(0));
        let daemon = TestDaemon {
            run_count: Arc::clone(&run_count),
            mode: Mode::ExitErr,
        };

        // Very long backoff — if the select! raced right, stop should
        // still return within a fraction of this.
        let config = DaemonConfig {
            restart_policy: RestartPolicy::OnFailure,
            restart_backoff: Duration::from_secs(10),
            max_restarts: 10,
        };
        let runtime: DaemonRuntime<TestDaemon> =
            DaemonRuntime::new(config, CancellationToken::new());
        let ctx = test_ctx();

        runtime
            .start(daemon, Arc::new(()), &ctx)
            .await
            .expect("start");

        // Wait for the loop to enter backoff (run returned Err, now sleeping).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while run_count.load(Ordering::SeqCst) == 0
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(run_count.load(Ordering::SeqCst) >= 1);

        // Give it a tiny grace so the loop definitely reached the sleep.
        tokio::time::sleep(Duration::from_millis(30)).await;

        let start = tokio::time::Instant::now();
        runtime.stop().await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(500),
            "stop() took {elapsed:?} — should have been cancel-responsive (#323)"
        );
    }
}
