//! Integration tests for WebhookAction DX trait + WebhookTriggerAdapter.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use nebula_action::{
    Action, ActionDependencies, ActionError, ActionMetadata, TestContextBuilder, TriggerContext,
    TriggerEvent, TriggerEventOutcome, TriggerHandler, WebhookAction, WebhookRequest,
    WebhookResponse, WebhookTriggerAdapter, webhook::webhook_request_for_test,
};
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

impl ActionDependencies for TestWebhook {}
impl Action for TestWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for TestWebhook {
    type State = WebhookReg;

    async fn on_activate(&self, _ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
        self.activated.store(true, Ordering::Relaxed);
        Ok(WebhookReg {
            hook_id: "hook_123".into(),
        })
    }

    async fn handle_request(
        &self,
        request: &WebhookRequest,
        _state: &WebhookReg,
        _ctx: &TriggerContext,
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
        _ctx: &TriggerContext,
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
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    assert!(activated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_stop_passes_stored_state() {
    let (webhook, _, deactivated) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    adapter.stop(&ctx).await.unwrap();
    assert!(deactivated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn webhook_adapter_handle_event_emits_on_valid_secret() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let req =
        webhook_request_for_test(br#"{"action":"push"}"#, &[("X-Secret", "mysecret")]).unwrap();
    let outcome = adapter.handle_event(wrap_event(req), &ctx).await.unwrap();
    assert!(outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_handle_event_skips_on_bad_secret() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();

    let req = webhook_request_for_test(br#"{"action":"push"}"#, &[("X-Secret", "wrong")]).unwrap();
    let outcome = adapter.handle_event(wrap_event(req), &ctx).await.unwrap();
    assert!(!outcome.will_emit());
}

#[tokio::test]
async fn webhook_adapter_accepts_events() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    assert!(adapter.accepts_events());
}

#[tokio::test]
async fn webhook_adapter_handle_event_before_start_fails() {
    let (webhook, _, _) = make_webhook();
    let adapter = WebhookTriggerAdapter::new(webhook);
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

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

impl ActionDependencies for CountingWebhook {}
impl Action for CountingWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for CountingWebhook {
    type State = WebhookReg;

    async fn on_activate(&self, _ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
        self.activate_count.fetch_add(1, Ordering::Relaxed);
        Ok(WebhookReg {
            hook_id: "hook".into(),
        })
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &WebhookReg,
        _ctx: &TriggerContext,
    ) -> Result<WebhookResponse, ActionError> {
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }

    async fn on_deactivate(
        &self,
        _state: WebhookReg,
        _ctx: &TriggerContext,
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
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

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
    let (ctx, _, _) = TestContextBuilder::minimal().build_trigger();

    adapter.start(&ctx).await.unwrap();
    adapter.stop(&ctx).await.unwrap();
    adapter.start(&ctx).await.unwrap();

    assert_eq!(activate.load(Ordering::Relaxed), 2);
    assert_eq!(deactivate.load(Ordering::Relaxed), 1);

    adapter.stop(&ctx).await.unwrap();
    assert_eq!(deactivate.load(Ordering::Relaxed), 2);
}
