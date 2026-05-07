# Архитектура: одна точка входа (Docker / local)

Один процесс = **одна точка входа**. Внутри него через `tokio::spawn` поднимаются:

1. **HTTP‑сервер** (один порт) — API + webhook.
2. **N воркеров** — задачи, которые тянут задания из общей очереди и выполняют их (движок workflow).

```
                    ┌─────────────────────────────────────────┐
                    │           Один процесс (binary)         │
                    │                                         │
  Client/Webhook ──►│  ┌─────────────────────────────────────┐ │
                    │  │  HTTP Server (axum)                 │ │
                    │  │  • GET  /health                     │ │
                    │  │  • GET  /api/v1/status              │ │
                    │  │  • POST /webhooks/{uuid}/{nonce}    │ │
                    │  │  • POST /api/v1/hooks/{org}/{ws}/   │ │
                    │  │         {trigger_slug}              │ │
                    │  │  • POST /internal/v1/webhooks/      │ │
                    │  │         reload  (X-Internal-Token)  │ │
                    │  └──────────────┬──────────────────────┘ │
                    │                 │                      │
                    │                 │ enqueue              │
                    │                 ▼                      │
                    │  ┌─────────────────────────────────┐   │
                    │  │  Queue (TaskQueue)              │   │
                    │  │  in-memory / Redis              │   │
                    │  └──────────────┬──────────────────┘   │
                    │                 │ dequeue              │
                    │     ┌───────────┼───────────┐          │
                    │     ▼           ▼           ▼          │
                    │  worker-1    worker-2    worker-3 ... │  tokio::spawn
                    │  (loop:      (loop:      (loop:       │
                    │   dequeue →   dequeue →   dequeue →   │
                    │   execute →   execute →   execute →    │
                    │   ack)        ack)        ack)        │
                    └─────────────────────────────────────────┘
```

## Точка входа (псевдокод)

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let queue: Arc<dyn TaskQueue> = Arc::new(MemoryQueue::default());
    let webhook = WebhookServer::new_embedded(webhook_config)?;
    let worker_count = 4;

    // Воркеры — в фоне, пока крутится сервер
    for i in 0..worker_count {
        let q = queue.clone();
        tokio::spawn(async move { worker_loop(i, q).await });
    }

    // Один порт: API + webhook (блокирует до shutdown)
    let app = nebula_api::app(webhook.clone(), workers_snapshot());
    axum::serve(listener, app).await?;
    Ok(())
}
```

## Docker

- **Один контейнер** — тот же бинарник: внутри и HTTP, и воркеры.
- При масштабировании потом можно вынести воркеры в отдельные контейнеры (очередь в Redis), как n8n Queue Mode.

## Webhook ingress (M3.3 / ADR-0049)

После M3.3 у webhook-сюрфейса **одна труба** — `WebhookTransport`. Оба URL-shape проходят через единый `dispatch_inner`:

| Surface | Path | Источник регистрации |
|---|---|---|
| Programmatic | `POST /webhooks/{trigger_uuid}/{nonce}` | Runtime через `WebhookTransport::activate(...)` (типизированные `WebhookAction`-триггеры) |
| Slug-routed | `POST|GET /api/v1/hooks/{org}/{ws}/{trigger_slug}` | Storage bootstrap `bootstrap_webhook_activations` + lifecycle bus + admin reload |
| Admin reload | `POST /internal/v1/webhooks/reload` | Внутренний хеадер `X-Internal-Token`, атомарный `replace_slug_map` |

Pipeline в `dispatch_inner` (один источник истины для обоих shape):

```
routing-map lookup     → 404
rate-limit (per-key)   → 429 + Retry-After
signature verify       → 401 (HMAC + replay-window timestamp)
pre_handle             → optional RespondNow (Slack url_verification,
                                              Stripe pending_webhook,
                                              Generic ?challenge=)
handle_request         → engine
```

Provider-каталог (`Slack` / `Stripe` / `Generic`) живёт в `crates/action/src/webhook/providers/`; engine-`ActionRegistry` держит string-keyed factory map; bootstrap по `action_kind` в storage row выбирает фабрику, разрешает `secret_id` через `WebhookSecretResolver`, строит `BuiltWebhookHandler` и регистрирует через `transport.activate_slug(...)`.

## Итог

Да: **одна точка входа**, в ней через `tokio::spawn` создаются воркеры и поднимается webhook/API сервер (или сервер в main, воркеры в spawn — как выше).
