# Архитектура: одна точка входа (Docker / local)

Один процесс = **одна точка входа**. Внутри него через `tokio::spawn` поднимаются:

1. **HTTP‑сервер** (один порт) — API + webhook.
2. **N воркеров** — задачи, которые тянут задания из общей очереди и выполняют их (движок workflow).

```
                    ┌─────────────────────────────────────────┐
                    │           Один процесс (binary)         │
                    │                                         │
  Client/Webhook ──►│  ┌─────────────────────────────────┐   │
                    │  │  HTTP Server (axum)             │   │
                    │  │  • GET /health                  │   │
                    │  │  • GET /api/v1/status           │   │
                    │  │  • POST /webhooks/*             │   │
                    │  └──────────────┬──────────────────┘   │
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
                    │  wrk-1       wrk-2       wrk-3  ...   │  tokio::spawn
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

## Итог

Да: **одна точка входа**, в ней через `tokio::spawn` создаются воркеры и поднимается webhook/API сервер (или сервер в main, воркеры в spawn — как выше).
