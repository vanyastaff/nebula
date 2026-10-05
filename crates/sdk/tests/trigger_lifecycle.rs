//! Lifecycle regressions through the public SDK harness and real trigger adapters.

use std::{
    future::pending,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_sdk::{TestRuntime, prelude::*};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct PollObservation {
    entered: Notify,
    cancelled: Notify,
    dropped: Notify,
    cancelled_on_drop: AtomicBool,
    did_drop: AtomicBool,
}

struct ActivationFuture {
    observation: Arc<PollObservation>,
    cancellation: CancellationToken,
}

impl Drop for ActivationFuture {
    fn drop(&mut self) {
        self.observation
            .cancelled_on_drop
            .store(self.cancellation.is_cancelled(), Ordering::SeqCst);
        self.observation.did_drop.store(true, Ordering::SeqCst);
        self.observation.dropped.notify_one();
    }
}

struct PendingActivation(Arc<PollObservation>);

impl Action for PendingActivation {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.pending_activation"),
            metadata_name!("Pending activation"),
            "A cooperative activation that ignores cancellation",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl PollAction for PendingActivation {
    type Cursor = u32;
    type Event = Value;

    fn poll_config(&self) -> PollConfig {
        PollConfig::fixed(Duration::from_secs(1))
    }

    async fn validate(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        let _activation = ActivationFuture {
            observation: self.0.clone(),
            cancellation: ctx.cancellation().clone(),
        };
        self.0.entered.notify_one();
        ctx.cancellation().cancelled().await;
        self.0.cancelled.notify_one();
        pending().await
    }

    async fn poll(
        &self,
        _cursor: &mut PollCursor<u32>,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<PollResult<Value>, ActionError> {
        unreachable!("the pending activation must not reach polling")
    }
}

#[tokio::test]
async fn poll_grace_timeout_joins_the_owned_start_task_before_returning() {
    let observation = Arc::new(PollObservation::default());
    let report = tokio::time::timeout(
        Duration::from_secs(8),
        TestRuntime::new(TestContextBuilder::minimal())
            .with_trigger_window(Duration::from_millis(10))
            .run_poll(PendingActivation(observation.clone())),
    )
    .await
    .expect("poll harness must finish its bounded shutdown")
    .expect("poll failures belong in the report");

    assert_eq!(
        report.note.as_deref(),
        Some("trigger did not exit within grace period")
    );
    assert!(observation.did_drop.load(Ordering::SeqCst));
    assert!(observation.cancelled_on_drop.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropping_the_poll_harness_cancels_and_aborts_its_start_task() {
    let observation = Arc::new(PollObservation::default());
    let harness = tokio::spawn(
        TestRuntime::new(TestContextBuilder::minimal())
            .with_trigger_window(Duration::from_mins(1))
            .run_poll(PendingActivation(observation.clone())),
    );
    tokio::time::timeout(Duration::from_secs(1), observation.entered.notified())
        .await
        .expect("activation must enter before the harness is cancelled");
    harness.abort();
    assert!(
        harness
            .await
            .expect_err("outer harness is cancelled")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(1), observation.dropped.notified())
        .await
        .expect("the owned start future must be dropped rather than detached");
    assert!(observation.cancelled_on_drop.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropping_the_poll_harness_during_grace_aborts_its_start_task() {
    let observation = Arc::new(PollObservation::default());
    let harness = tokio::spawn(
        TestRuntime::new(TestContextBuilder::minimal())
            .with_trigger_window(Duration::ZERO)
            .run_poll(PendingActivation(observation.clone())),
    );
    tokio::time::timeout(Duration::from_secs(1), observation.cancelled.notified())
        .await
        .expect("the window must end before cancelling the grace-period wait");
    harness.abort();
    assert!(
        harness
            .await
            .expect_err("outer harness is cancelled")
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(1), observation.dropped.notified())
        .await
        .expect("the borrowed JoinHandle must stay guarded during grace");
    assert!(observation.cancelled_on_drop.load(Ordering::SeqCst));
}

struct FailingWebhook {
    fail_activation: bool,
    fail_event: bool,
    fail_cleanup: bool,
    hang_cleanup: bool,
    stops: Arc<AtomicUsize>,
}

impl Action for FailingWebhook {
    type Input = Value;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("test.failing_webhook"),
            metadata_name!("Failing webhook"),
            "Lifecycle failure fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl WebhookAction for FailingWebhook {
    type State = ();

    async fn on_activate(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        if self.fail_activation {
            return Err(ActionError::fatal("activation-primary"));
        }
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        if self.fail_event {
            return Err(ActionError::fatal("event-primary"));
        }
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }

    async fn on_deactivate(
        &self,
        _state: (),
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if self.hang_cleanup {
            return pending().await;
        }
        if self.fail_cleanup {
            return Err(ActionError::fatal("cleanup-secondary"));
        }
        Ok(())
    }
}

fn webhook(fail_event: bool, fail_cleanup: bool, hang_cleanup: bool) -> FailingWebhook {
    FailingWebhook {
        fail_activation: false,
        fail_event,
        fail_cleanup,
        hang_cleanup,
        stops: Arc::new(AtomicUsize::new(0)),
    }
}

async fn run_webhook(action: FailingWebhook) -> Result<RunReport, ActionError> {
    let request = nebula_action::webhook::webhook_request_for_test(b"{}", &[])
        .expect("valid bounded fake webhook request");
    tokio::time::timeout(
        Duration::from_secs(8),
        TestRuntime::new(TestContextBuilder::minimal()).run_webhook(action, request),
    )
    .await
    .expect("webhook harness must bound cleanup")
}

#[tokio::test]
async fn webhook_event_failure_still_deactivates_and_preserves_the_primary_error() {
    for fail_cleanup in [false, true] {
        let action = webhook(true, fail_cleanup, false);
        let stops = action.stops.clone();
        let error = run_webhook(action).await.expect_err("event must fail");
        assert!(
            std::error::Error::source(&error)
                .expect("concrete failure cause")
                .to_string()
                .contains("event-primary")
        );
        assert!(
            !std::error::Error::source(&error)
                .expect("concrete failure cause")
                .to_string()
                .contains("cleanup-secondary")
        );
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn webhook_cleanup_error_is_returned_after_successful_event_handling() {
    let action = webhook(false, true, false);
    let stops = action.stops.clone();
    let error = run_webhook(action).await.expect_err("cleanup must fail");
    assert!(
        std::error::Error::source(&error)
            .expect("concrete failure cause")
            .to_string()
            .contains("cleanup-secondary")
    );
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn webhook_cleanup_timeout_is_bounded_and_preserves_an_event_failure() {
    for fail_event in [false, true] {
        let action = webhook(fail_event, false, true);
        let stops = action.stops.clone();
        let error = run_webhook(action)
            .await
            .expect_err("cleanup never completes");
        let expected = if fail_event {
            "event-primary"
        } else {
            "webhook stop() did not exit within grace period"
        };
        assert!(
            std::error::Error::source(&error)
                .expect("concrete failure cause")
                .to_string()
                .contains(expected)
        );
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn webhook_failed_activation_does_not_deactivate_unowned_state() {
    let mut action = webhook(false, false, false);
    action.fail_activation = true;
    let stops = action.stops.clone();
    let error = run_webhook(action).await.expect_err("activation must fail");
    assert!(
        std::error::Error::source(&error)
            .expect("concrete failure cause")
            .to_string()
            .contains("activation-primary")
    );
    assert_eq!(stops.load(Ordering::SeqCst), 0);
}
