# Example: Log Event Streams

Use this pattern when you need human-readable operational traces from event streams.

```rust
use nebula_eventbus::EventBus;

#[derive(Debug, Clone)]
enum LoggableEvent {
    Started { execution_id: String },
    Completed { execution_id: String },
}

#[tokio::main]
async fn main() {
    let bus = EventBus::<LoggableEvent>::new(256);
    let mut sub = bus.subscribe();

    tokio::spawn(async move {
        while let Some(event) = sub.recv().await {
            tracing::info!(target: "nebula.eventbus", ?event, "event received");
        }
    });

    let _ = bus.emit(LoggableEvent::Started {
        execution_id: "exec-1".into(),
    });
    let _ = bus.emit(LoggableEvent::Completed {
        execution_id: "exec-1".into(),
    });
}
```

## Why this works

- Producer path stays non-blocking by default.
- Subscriber can lag independently without stopping producers.
- `PublishOutcome` can be captured if you need drop-aware logging.
