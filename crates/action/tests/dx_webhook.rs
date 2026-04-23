//! Integration tests for WebhookAction DX trait + WebhookTriggerAdapter.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use nebula_action::{
    Action, ActionError, ActionMetadata, HasTriggerScheduling, TestContextBuilder, TriggerEvent,
    TriggerEventOutcome, TriggerHandler, WebhookAction, WebhookRequest, WebhookResponse,
    WebhookTriggerAdapter, webhook::webhook_request_for_test,
};
use nebula_core::{DeclaresDependencies, context::Context};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebhookReg {
    hook_id: String,
}

struct TestWebhook {
    meta: ActionMetadata,
    secret: String,
    activated: Arc<AtomicBool>,
    deactivated: Arc<AtomicBool>,
}

impl DeclaresDependencies for TestWebhook {}
impl Action for TestWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for TestWebhook {
    type State = WebhookReg;

    async fn on_activate(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookReg, ActionError> {
        self.activated.store(true, Ordering::Relaxed);
        Ok(WebhookReg {
            hook_id: "hook_123".into(),
        })
    }

    async fn handle_request(
        &self,
        request: &WebhookRequest,
        _state: &WebhookReg,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        let sig = request.header_str("X-Secret").unwrap_or_default();
        if sig != self.secret {
            return Ok(WebhookResponse::accept(TriggerEventOutcome::skip()));
        }
        let payload = request.body_json::<serde_json::Value>().map_err(|e| {
            ActionError::validation(
                "body",
                nebula_action::ValidationReason::MalformedJson,
                Some(e.to_string()),
            )
        })?;
        Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
    }

    async fn on_deactivate(
        &self,
        _state: WebhookReg,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        self.deactivated.store(true, Ordering::Relaxed);
        Ok(())
    }
}

fn make_webhook() -> (TestWebhook, Arc<AtomicBool>, Arc<AtomicBool>) {
    let activated = Arc::new(AtomicBool::new(false));
    let deactivated = Arc::new(AtomicBool::new(false));
    (
        TestWebhook {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.webhook"),
                "Test Webhook",
                "Test webhook action",
            ),
            secret: "mysecret".into(),
            activated: activated.clone(),
            deactivated: deactivated.clone(),
        },
        activated,
        deactivated,
    )
}

fn wrap_event(req: WebhookRequest) -> TriggerEvent {
    TriggerEvent::new(None, req)
}

#[tokio::test]
async fn webhook_adapter_start_stores_state() {
    let (webhook, activated, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    assert!(activated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_stop_passes_stored_state() {
    let (webhook, _, deactivated) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    adapter.stop(&ctx).await.unwrap();
    assert!(deactivated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_handle_event_emits_on_valid_secret() {
    let (webhook, ..) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let req =
        webhook_request_for_test(br#"{"action":"push"}"#, &[("X-Secret", "mysecret")]).unwrap();
    let outcome = adapter.handle_event(wrap_event(req), &ctx).await.unwrap();
    assert!(outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_handle_event_skips_on_bad_secret() {
    let (webhook, ..) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(br#"{"action":"push"}"#, &[("X-Secret", "wrong")]).unwrap();
    let outcome = adapter.handle_event(wrap_event(req), &ctx).await.unwrap();
    assert!(!outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_accepts_events() {
    let (webhook, ..) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    assert!(adapter.accepts_events());
}

#[tokio::test]
async fn webhook_adapter_handle_event_before_start_fails() {
    let (webhook, ..) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    let req =
        webhook_request_for_test(br#"{"action":"push"}"#, &[("X-Secret", "mysecret")]).unwrap();
    let result = adapter.handle_event(wrap_event(req), &ctx).await;
    assert!(result.is_err());
    nebula_action::assert_fatal!(result);
}

// ── Double-start rejection (A2) ───────────────────────────────────────────

struct CountingWebhook {
    meta: ActionMetadata,
    activate_count: Arc<AtomicUsize>,
    deactivate_count: Arc<AtomicUsize>,
}

impl DeclaresDependencies for CountingWebhook {}
impl Action for CountingWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for CountingWebhook {
    type State = WebhookReg;

    async fn on_activate(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookReg, ActionError> {
        self.activate_count.fetch_add(1, Ordering::Relaxed);
        Ok(WebhookReg {
            hook_id: "hook".into(),
        })
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &WebhookReg,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }

    async fn on_deactivate(
        &self,
        _state: WebhookReg,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        self.deactivate_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn make_counting() -> (CountingWebhook, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let activate = Arc::new(AtomicUsize::new(0));
    let deactivate = Arc::new(AtomicUsize::new(0));
    (
        CountingWebhook {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.webhook.count"),
                "Counting Webhook",
                "Counts activate/deactivate",
            ),
            activate_count: activate.clone(),
            deactivate_count: deactivate.clone(),
        },
        activate,
        deactivate,
    )
}

#[tokio::test]
async fn webhook_adapter_rejects_double_start() {
    let (webhook, activate, deactivate) = make_counting();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    assert_eq!(activate.load(Ordering::Relaxed), 1);
    assert_eq!(deactivate.load(Ordering::Relaxed), 0);

    let err = adapter
        .start(&ctx)
        .await
        .expect_err("second start must fail");
    assert!(err.is_fatal(), "double-start must be Fatal");
    assert_eq!(activate.load(Ordering::Relaxed), 1);
    assert_eq!(deactivate.load(Ordering::Relaxed), 0);

    adapter.stop(&ctx).await.unwrap();
    assert_eq!(deactivate.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn webhook_adapter_start_stop_start_succeeds() {
    let (webhook, activate, deactivate) = make_counting();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    adapter.stop(&ctx).await.unwrap();
    adapter.start(&ctx).await.unwrap();

    assert_eq!(activate.load(Ordering::Relaxed), 2);
    assert_eq!(deactivate.load(Ordering::Relaxed), 1);

    adapter.stop(&ctx).await.unwrap();
    assert_eq!(deactivate.load(Ordering::Relaxed), 2);
}

// ── H1: handle_request error → 500 via oneshot ───────────────────────────

struct ErroringWebhook {
    meta: ActionMetadata,
}

impl DeclaresDependencies for ErroringWebhook {}
impl Action for ErroringWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for ErroringWebhook {
    type State = ();

    async fn on_activate(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        Err(ActionError::retryable("handler blew up"))
    }
}

#[tokio::test]
async fn handle_request_error_sends_500_via_oneshot() {
    use http::StatusCode;
    let adapter = WebhookTriggerAdapter::new(ErroringWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.error"),
            "Erroring Webhook",
            "handle_request always returns Err",
        ),
    });
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();
    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(b"{}", &[]).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let req = req.with_response_channel(tx);
    let event = wrap_event(req);

    let result = adapter.handle_event(event, &ctx).await;
    assert!(
        result.is_err(),
        "handle_event must propagate the handler's Err"
    );

    // The oneshot MUST have received a 500 before the Err was
    // propagated. Without H1, this would be Err(RecvError) and the
    // transport would hang or return a wrong status.
    let response = rx
        .await
        .expect("oneshot sender must have sent 500 before returning Err");
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
}

// ── H6: cancellation mid-request ─────────────────────────────────────────

struct HangingWebhook {
    meta: ActionMetadata,
    entered: Arc<AtomicBool>,
}

impl DeclaresDependencies for HangingWebhook {}
impl Action for HangingWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for HangingWebhook {
    type State = ();

    async fn on_activate(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        self.entered.store(true, Ordering::Relaxed);
        // Hang forever — only cancellation can save us.
        std::future::pending::<()>().await;
        unreachable!()
    }
}

#[tokio::test]
async fn handle_request_cancelled_mid_flight_returns_cleanly() {
    use http::StatusCode;
    let entered = Arc::new(AtomicBool::new(false));
    let adapter = Arc::new(WebhookTriggerAdapter::new(HangingWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.hang"),
            "Hanging Webhook",
            "handle_request hangs forever",
        ),
        entered: entered.clone(),
    }));
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();
    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(b"{}", &[]).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let req = req.with_response_channel(tx);
    let event = wrap_event(req);

    let cancel = ctx.cancellation().clone();
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let handle = tokio::spawn(async move { adapter1.handle_event(event, &ctx1).await });

    // Let handle_request enter pending state.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        entered.load(Ordering::Relaxed),
        "handler should have started before cancel"
    );

    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle)
        .await
        .expect("handle_event must exit within 1s of cancellation")
        .unwrap();
    assert!(
        result.is_err(),
        "cancelled handle_event must return retryable Err"
    );

    // Oneshot must have received 503.
    let response = rx.await.expect("503 must be sent via oneshot on cancel");
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
}

// ── H7: TriggerHealth wired ──────────────────────────────────────────────

#[tokio::test]
async fn webhook_adapter_records_health_success_on_emit() {
    let (webhook, ..) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();
    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(br#"{"ok":true}"#, &[("X-Secret", "mysecret")]).unwrap();
    let event = wrap_event(req);
    let outcome = adapter.handle_event(event, &ctx).await.unwrap();
    assert!(matches!(outcome, TriggerEventOutcome::Emit(_)));

    let snap = ctx.health().snapshot();
    assert_eq!(
        snap.total_emitted, 1,
        "health must record 1 emission from handle_event"
    );
    assert_eq!(snap.error_streak, 0);
}

#[tokio::test]
async fn webhook_adapter_records_health_error_on_handler_failure() {
    let adapter = WebhookTriggerAdapter::new(ErroringWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.error_health"),
            "Erroring",
            "error path health check",
        ),
    });
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();
    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(b"{}", &[]).unwrap();
    let event = wrap_event(req);
    let _ = adapter.handle_event(event, &ctx).await;

    let snap = ctx.health().snapshot();
    assert!(
        snap.error_streak >= 1,
        "health error_streak must grow on handler Err, got {}",
        snap.error_streak
    );
}

// ── H10: Notify wakes stop() instead of yield_now spin ───────────────────

struct SlowWebhook {
    meta: ActionMetadata,
    finish: Arc<AtomicBool>,
}

impl DeclaresDependencies for SlowWebhook {}
impl Action for SlowWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for SlowWebhook {
    type State = ();

    async fn on_activate(
        &self,
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl nebula_action::TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        // Await until a signal flag flips — simulates a slow handler.
        while !self.finish.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }
}

#[tokio::test]
async fn in_flight_notify_wakes_stop() {
    let finish = Arc::new(AtomicBool::new(false));
    let adapter = Arc::new(WebhookTriggerAdapter::new(SlowWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.slow"),
            "Slow Webhook",
            "handle_request awaits a flag",
        ),
        finish: finish.clone(),
    }));
    let (ctx, ..) = TestContextBuilder::minimal().build_trigger();
    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(b"{}", &[]).unwrap();
    let event = wrap_event(req);

    // Spawn the in-flight request.
    let adapter1 = Arc::clone(&adapter);
    let ctx1 = ctx.clone();
    let in_flight_task = tokio::spawn(async move { adapter1.handle_event(event, &ctx1).await });

    // Let the handler enter the slow loop.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    // Start stop() — it should block on in_flight > 0.
    let adapter2 = Arc::clone(&adapter);
    let ctx2 = ctx.clone();
    let stop_task = tokio::spawn(async move { adapter2.stop(&ctx2).await });

    // Give stop() a chance to park on the notify.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !stop_task.is_finished(),
        "stop() should be waiting for in-flight"
    );

    // Release the handler. Notify wakes stop() promptly.
    finish.store(true, Ordering::Release);

    let stop_result = tokio::time::timeout(std::time::Duration::from_secs(2), stop_task)
        .await
        .expect("stop() must return within 2s once in_flight drops to 0")
        .unwrap();
    assert!(stop_result.is_ok());

    // And the in-flight request also completes.
    let in_flight_result = tokio::time::timeout(std::time::Duration::from_secs(2), in_flight_task)
        .await
        .expect("in-flight handle_event must complete after finish flag")
        .unwrap();
    assert!(in_flight_result.is_ok());
}
