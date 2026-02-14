---
title: Built-in Actions
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# Built-in Actions

Document canonical bundled actions here. Each entry follows the template.

## HTTP Request
**ID:** `http.request`  
**Purpose:** Make HTTP calls with typed input/output, timeouts, and headers.

### Parameters
- `url: string` — target URL (required)
- `method: enum(GET|POST|PUT|DELETE|PATCH)` — default `GET`
- `headers: map<string,string>` — optional
- `body: any` — optional
- `timeout_ms: u32` — default `10000`

### Output
```json
{ "status": 200, "headers": { "content-type": "application/json" }, "body": { ... }, "elapsed_ms": 12 }
```

### Errors
- `InvalidInput.method`, `Timeout`, `ExternalServiceError{service:"http"}`

---

## PostgreSQL Supplier
**ID:** `pg.supply`  
**Purpose:** Provide a pooled Postgres client for downstream actions.

### Parameters
- `dsn: string` — connection string
- `max_connections: u32` — default 10
- `idle_timeout_ms: u32` — default 30000

### Output
```
PoolHandle
```

### Health
- `health_check` executes `SELECT 1` within 500ms.

---

## Kafka Consumer Trigger
**ID:** `kafka.trigger`  
**Purpose:** Consume messages and emit events with offsets.

### Parameters
- `brokers: string[]`
- `topic: string`
- `group_id: string`
- `offset_reset: enum(earliest|latest)`

### Event
```json
{ "key": "abc", "payload": { ... }, "offset": 12345, "partition": 0 }
```

### Notes
- Stores consumer offsets in durable storage; ensures at-least-once with idempotency keys.
