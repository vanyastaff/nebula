# nebula-action v2 — Canonical Examples

> Authoritative patterns for action authors. Every example compiles against the target API.
> Companion to `2026-04-08-action-v2-spec.md`.

> **Important:** These examples describe the **v2 target API** as specified in the companion spec.
> The current implementation does not yet match — see the spec's "Current State & Target Changes" section for what exists today.
> Implementation phases are tracked in the roadmap.

**Date:** 2026-04-08
**Status:** Draft

---

## 1. Quick Start — Minimal StatelessAction

The simplest possible action: echo input to output.

```rust
use nebula_action::prelude::*;
use serde_json::Value;

#[derive(Action)]
#[action(key = "echo", name = "Echo")]
struct Echo;

impl StatelessAction for Echo {
    type Input = Value;
    type Output = Value;

    async fn execute(
        &self,
        input: Value,
        _ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}
```

**What happens under the hood:**
1. `#[derive(Action)]` generates `Action` impl returning `ActionMetadata { key: "echo", name: "Echo", .. }`
2. `#[derive(Action)]` generates `ActionDependencies` impl with empty credentials/resources
3. Developer writes the `execute` method — the only thing the macro can't generate

---

## 1.5. How `type Input = Self` Works

Many examples below use `type Input = Self;` — this means the action struct itself IS the deserialized input.

**The mechanism:**
1. The engine receives parameter JSON from the workflow definition
2. The adapter layer calls `serde_json::from_value::<Input>(params)` to deserialize into the action's `Input` type
3. When `Input = Self`, this deserializes the JSON directly into the action struct
4. The struct fields ARE the parameters — `self` and `input` carry the same data

**Why this works:** The struct derives `Deserialize`, so serde can construct it from the parameter JSON. This is the most common pattern for actions whose parameters fully define their behavior.

**When to use something else:** Use `type Input = Value` (like the Echo example) when you want raw JSON, or a separate input struct when the action has internal state distinct from its parameters.

---

## 2. HTTP Request — StatelessAction with Credentials

Real-world stateless action: HTTP call with typed credentials, parameters, and error handling.

```rust
use nebula_action::prelude::*;  // includes ActionResultExt (.retryable()?, .fatal()?)
use nebula_parameter::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Action, Parameters, Deserialize, Clone)]
#[action(key = "request", name = "HTTP Request")]
#[action(credential = BearerSecret)]
struct HttpRequest {
    #[param(label = "URL", hint = "url")]
    #[validate(required, url)]
    url: String,

    #[param(label = "Method", default = "GET")]
    method: String,

    #[param(label = "Headers")]
    headers: Option<Vec<Header>>,

    #[param(label = "Request Body")]
    body: Option<Value>,

    #[param(label = "Timeout (seconds)", default = 30)]
    #[validate(range(1..=300))]
    timeout: u32,
}

#[derive(Deserialize, Serialize, Clone)]
struct Header {
    key: String,
    value: String,
}

// Plugin registration binds this to PluginKey("http")
// Fully qualified at runtime: "http.request"

impl StatelessAction for HttpRequest {
    type Input = Self;      // Struct IS the input — fields populated from parameters
    type Output = Value;

    async fn execute(
        &self,
        _input: Self,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        // 1. Get typed credential
        let cred = ctx.credential::<BearerSecret>()?;
        // cred is CredentialGuard<BearerSecret> — Deref to BearerSecret, auto-zeroize on drop

        // 2. Build request
        let client = reqwest::Client::new();
        let mut req = client
            .request(
                self.method.parse().map_err(|e| ActionError::validation(
                    format!("Invalid HTTP method: {}", e),
                ))?,
                &self.url,
            )
            .bearer_auth(cred.token.expose_secret())
            .timeout(std::time::Duration::from_secs(self.timeout as u64));

        // 3. Add headers
        if let Some(headers) = &self.headers {
            for h in headers {
                req = req.header(&h.key, &h.value);
            }
        }

        // 4. Add body
        if let Some(body) = &self.body {
            req = req.json(body);
        }

        // 5. Execute — ActionResultExt provides .retryable()? and .fatal()?
        //    for simple cases. Here we use explicit classification for
        //    fine-grained control over timeout vs connect errors.
        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ActionError::retryable_with_backoff(e, Duration::from_secs(5))
            } else if e.is_connect() {
                ActionError::retryable(e)
            } else {
                ActionError::fatal(e)
            }
        })?;

        // 6. Handle response status
        let status = response.status().as_u16();
        let response_headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // ActionResultExt: .fatal()? is shorthand for .map_err(ActionError::fatal)?
        let body: Value = response.json().await.fatal()?;

        match status {
            200..=299 => Ok(ActionResult::success(json!({
                "status": status,
                "headers": response_headers,
                "body": body,
            }))),
            429 => {
                let retry_after = response_headers
                    .get("retry-after")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30);
                Err(ActionError::retryable_with_backoff(
                    anyhow::anyhow!("Rate limited (429)"),
                    Duration::from_secs(retry_after),
                ))
            }
            401 | 403 => Err(ActionError::fatal_with_details(
                anyhow::anyhow!("Authentication failed ({})", status),
                json!({ "status": status, "body": body }),
            )),
            _ => Err(ActionError::retryable(
                anyhow::anyhow!("HTTP {} error", status),
            )),
        }
    }
}
```

---

## 3. If/Switch — Branching with ActionResult::Branch

Conditional routing using `StatelessAction` with `ActionResult::Branch`.

```rust
use nebula_action::prelude::*;
use nebula_parameter::prelude::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Action, Parameters, Deserialize, Clone)]
#[action(key = "if", name = "If / Switch")]
struct IfSwitch {
    #[param(label = "Condition Expression")]
    #[validate(required)]
    condition: String,

    #[param(label = "Mode", default = "expression")]
    mode: IfMode,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum IfMode {
    Expression,
    Equals,
    Contains,
    Regex,
}

// Plugin: PluginKey("core") + ActionKey("if") → "core.if"

impl StatelessAction for IfSwitch {
    type Input = Self;
    type Output = Value;

    async fn execute(
        &self,
        _input: Self,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let input_data = ctx.input_data();

        let result = match self.mode {
            IfMode::Expression => {
                evaluate_expression(&self.condition, input_data)
                    .map_err(|e| ActionError::validation(
                        format!("Invalid expression: {}", e),
                    ))?
            }
            IfMode::Equals => {
                let field_value = resolve_field(input_data, &self.condition);
                field_value.is_some()
            }
            IfMode::Contains => {
                input_data.to_string().contains(&self.condition)
            }
            IfMode::Regex => {
                let re = regex::Regex::new(&self.condition)
                    .map_err(|e| ActionError::validation(
                        format!("Invalid regex: {}", e),
                    ))?;
                re.is_match(&input_data.to_string())
            }
        };

        // ActionResult::branch() is a convenience constructor that internally creates
        // ActionResult::Success { routing: Routing::Branch { branch: "true"|"false", .. } }
        if result {
            Ok(ActionResult::branch("true", input_data.clone()))
        } else {
            Ok(ActionResult::branch("false", input_data.clone()))
        }
    }
}
```

> **Note:** `ActionResult::branch("true", data)` is syntactic sugar for
> `ActionResult::Success { data, routing: Routing::Branch { branch: "true".into() } }`.
> The engine uses the branch name to select which downstream connection to follow.

---

## 4. Paginated Fetch — PaginatedAction (DX Type)

Cursor-based pagination without manual state management.

```rust
use nebula_action::prelude::*;
use nebula_action::dx::PaginatedAction;  // DX layer
use nebula_parameter::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Action, Parameters, Deserialize, Clone)]
#[action(key = "paginated_fetch", name = "Paginated API Fetch")]
#[action(credential = ApiToken)]
struct PaginatedFetch {
    #[param(label = "API URL")]
    #[validate(required, url)]
    url: String,

    #[param(label = "Max Pages", default = 10)]
    #[validate(range(1..=100))]
    max_pages: u32,

    #[param(label = "Page Size", default = 50)]
    #[validate(range(1..=500))]
    page_size: u32,
}

// Plugin: PluginKey("api") + ActionKey("paginated_fetch")

/// Cursor type — whatever the API uses for pagination
#[derive(Serialize, Deserialize, Clone)]
struct ApiCursor {
    next_url: String,
}

impl PaginatedAction for PaginatedFetch {
    type Input = Self;
    type Output = Value;
    type Cursor = ApiCursor;

    fn max_pages(&self) -> u32 {
        self.max_pages
    }

    async fn fetch_page(
        &self,
        _input: &Self::Input,
        cursor: Option<&ApiCursor>,
        ctx: &ActionContext,
    ) -> Result<PageResult<Value, ApiCursor>, ActionError> {
        let cred = ctx.credential::<ApiToken>()?;

        let url = match cursor {
            Some(c) => c.next_url.clone(),
            None => format!("{}?limit={}", self.url, self.page_size),
        };

        let response: Value = reqwest::Client::new()
            .get(&url)
            .bearer_auth(cred.token.expose_secret())
            .send()
            .await
            .map_err(ActionError::retryable)?
            .json()
            .await
            .map_err(ActionError::fatal)?;

        let next_cursor = response
            .get("next_url")
            .and_then(|v| v.as_str())
            .map(|url| ApiCursor { next_url: url.to_string() });

        Ok(PageResult {
            data: response["data"].clone(),
            next_cursor,
        })
    }
}
```

**What `PaginatedAction` does for you:**
- Manages `PaginationState { cursor, pages_fetched }` automatically
- Reports progress: `pages_fetched / max_pages`
- Returns `Continue` when `next_cursor.is_some() && pages_fetched < max_pages`
- Returns `Break { Completed }` when done
- Engine checkpoints state between iterations — survives restarts

---

## 5. Webhook Trigger — WebhookAction (DX Type)

Full webhook lifecycle: register, verify, handle, unregister.

```rust
use nebula_action::prelude::*;
use nebula_action::dx::{WebhookAction, WebhookRequest, WebhookResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Action)]
#[action(key = "incoming_webhook", name = "GitHub Webhook")]
#[action(credential = WebhookSecret)]
struct GitHubWebhook;

// Plugin: PluginKey("github") + ActionKey("incoming_webhook")

#[derive(Serialize, Deserialize, Default)]
struct WebhookState {
    hook_id: Option<String>,
}

impl WebhookAction for GitHubWebhook {
    type State = WebhookState;

    async fn on_activate(
        &self,
        ctx: &TriggerContext,
    ) -> Result<WebhookState, ActionError> {
        let cred = ctx.credential::<WebhookSecret>()?;

        // Register webhook with GitHub API
        let response = reqwest::Client::new()
            .post("https://api.github.com/repos/owner/repo/hooks")
            .bearer_auth(cred.token.expose_secret())
            .json(&serde_json::json!({
                "name": "web",
                "active": true,
                "events": ["push", "pull_request"],
                "config": {
                    "url": ctx.webhook_url(),
                    "content_type": "json",
                    "secret": cred.signing_secret.expose_secret(),
                }
            }))
            .send()
            .await
            .map_err(ActionError::retryable)?
            .json::<Value>()
            .await
            .map_err(ActionError::fatal)?;

        Ok(WebhookState {
            hook_id: response["id"].as_str().map(String::from),
        })
    }

    async fn verify_signature(
        &self,
        request: &WebhookRequest,
        _state: &WebhookState,
        ctx: &TriggerContext,
    ) -> Result<bool, ActionError> {
        let cred = ctx.credential::<WebhookSecret>()?;

        let signature = request.headers
            .get("x-hub-signature-256")
            .ok_or_else(|| ActionError::validation("Missing signature header"))?;

        let expected = hmac_sha256(
            cred.signing_secret.expose_secret().as_bytes(),
            &request.body,
        );

        // Constant-time comparison (prevent timing attacks)
        Ok(constant_time_eq(signature.as_bytes(), expected.as_bytes()))
    }

    async fn handle_request(
        &self,
        request: WebhookRequest,
        _state: &WebhookState,
        ctx: &TriggerContext,
    ) -> Result<WebhookResponse, ActionError> {
        let event_type = request.headers
            .get("x-github-event")
            .cloned()
            .unwrap_or_default();

        let payload: Value = serde_json::from_slice(&request.body)
            .map_err(|e| ActionError::validation(e.to_string()))?;

        // Emit workflow execution with the webhook payload
        ctx.emit_execution(serde_json::json!({
            "event": event_type,
            "payload": payload,
            "delivery_id": request.headers.get("x-github-delivery"),
        })).await?;

        Ok(WebhookResponse {
            status: 200,
            headers: HashMap::new(),
            body: Bytes::from_static(b"OK"),
        })
    }

    async fn on_deactivate(
        &self,
        state: WebhookState,
        ctx: &TriggerContext,
    ) -> Result<(), ActionError> {
        if let Some(hook_id) = &state.hook_id {
            let cred = ctx.credential::<WebhookSecret>()?;
            reqwest::Client::new()
                .delete(format!(
                    "https://api.github.com/repos/owner/repo/hooks/{}",
                    hook_id,
                ))
                .bearer_auth(cred.token.expose_secret())
                .send()
                .await
                .map_err(ActionError::retryable)?;
        }
        Ok(())
    }
}
```

> **Note:** `WebhookAction` provides default no-op implementations for `on_activate` and `on_deactivate`,
> and `verify_signature` defaults to `Ok(true)`. For simple webhooks that just listen, you only need to
> implement `handle_request`.

---

## 6. Event Trigger — EventTrigger (DX Type)

SSE/WebSocket event subscription with auto-reconnect.

```rust
use nebula_action::prelude::*;
use nebula_action::dx::{EventTrigger, EventErrorPolicy};
use serde::Serialize;
use serde_json::Value;

#[derive(Action)]
#[action(key = "sse_listener", name = "SSE Event Listener")]
#[action(credential = ApiToken)]
struct SseListener;

// Plugin: PluginKey("events") + ActionKey("sse_listener")

struct SseConnection {
    stream: reqwest_eventsource::EventSource,
}

impl EventTrigger for SseListener {
    type Connection = SseConnection;
    type Event = Value;

    async fn connect(&self, ctx: &TriggerContext) -> Result<SseConnection, ActionError> {
        let cred = ctx.credential::<ApiToken>()?;

        let stream = reqwest_eventsource::EventSource::get("https://api.example.com/events")
            .header("Authorization", format!("Bearer {}", cred.token.expose_secret()))
            .build()
            .map_err(ActionError::retryable)?;

        Ok(SseConnection { stream })
    }

    async fn next_event(
        &self,
        conn: &mut SseConnection,
        ctx: &TriggerContext,
    ) -> Result<Option<Value>, ActionError> {
        match conn.stream.next().await {
            Some(Ok(event)) => {
                let data: Value = serde_json::from_str(&event.data)
                    .map_err(|e| ActionError::validation(e.to_string()))?;
                Ok(Some(data))
            }
            Some(Err(e)) => Err(ActionError::retryable(e)),
            None => Ok(None), // Stream ended
        }
    }

    fn on_error(&self, error: &ActionError) -> EventErrorPolicy {
        // Reconnect on transient errors, stop on fatal
        if error.is_retryable() {
            EventErrorPolicy::Reconnect {
                delay: Duration::from_secs(5),
            }
        } else {
            EventErrorPolicy::Stop
        }
    }
}
```

**What `EventTrigger` does for you:**
- `start()` → spawns loop: `connect()` → `next_event()` → `ctx.emit_execution(event)` → repeat
- Auto-reconnect on `Reconnect` error policy
- `stop()` → cancels via `CancellationToken`
- Health check: reports connection state

---

## 7. Database Pool — ResourceAction

Scoped dependency injection: provide a connection pool to downstream nodes.

```rust
use nebula_action::prelude::*;
use serde::Deserialize;

#[derive(Action)]
#[action(key = "postgres_pool", name = "PostgreSQL Connection Pool")]
#[action(credential = PostgresCredential)]
struct PostgresPool;

// Plugin: PluginKey("postgres") + ActionKey("postgres_pool")

#[derive(Deserialize)]
struct PoolConfig {
    max_connections: u32,
    idle_timeout_secs: u64,
}

struct Pool {
    inner: sqlx::PgPool,
}

impl ResourceAction for PostgresPool {
    type Resource = Pool;

    async fn configure(
        &self,
        ctx: &ActionContext,
    ) -> Result<Pool, ActionError> {
        let cred = ctx.credential::<PostgresCredential>()?;

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(self.max_connections)
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs))
            .connect(&cred.connection_string())
            .await
            .map_err(ActionError::retryable)?;

        Ok(Pool { inner: pool })
    }

    async fn cleanup(
        &self,
        resource: Pool,
        _ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        resource.inner.close().await;
        Ok(())
    }
}
```

**How downstream nodes use it:**

```rust
impl StatelessAction for QueryUsers {
    type Input = QueryParams;
    type Output = Value;

    async fn execute(
        &self,
        input: QueryParams,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        // Get the pool provided by upstream PostgresPool ResourceAction
        let pool: &Pool = ctx.resource("database")
            .map_err(ActionError::fatal)?;

        let rows = sqlx::query_as!(User, "SELECT * FROM users LIMIT $1", input.limit)
            .fetch_all(&pool.inner)
            .await
            .map_err(ActionError::retryable)?;

        Ok(ActionResult::success(serde_json::to_value(rows).unwrap()))
    }
}
```

---

## 8. Polling Trigger — PollAction (DX Type)

Periodic polling with cursor persistence.

```rust
use nebula_action::prelude::*;
use nebula_action::dx::PollAction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Action)]
#[action(key = "poll_new_items", name = "Poll for New Items")]
#[action(credential = ApiToken)]
struct PollNewItems;

// Plugin: PluginKey("myservice") + ActionKey("poll_new_items")

#[derive(Serialize, Deserialize, Default)]
struct PollCursor {
    last_seen_id: Option<String>,
    last_poll_at: Option<String>,
}

impl PollAction for PollNewItems {
    type Cursor = PollCursor;
    type Event = Value;

    fn poll_interval(&self) -> Duration {
        Duration::from_secs(60) // Poll every minute
    }

    async fn poll(
        &self,
        cursor: &mut PollCursor,
        ctx: &TriggerContext,
    ) -> Result<Vec<Value>, ActionError> {
        let cred = ctx.credential::<ApiToken>()?;

        let mut url = "https://api.example.com/items?order=created_at".to_string();
        if let Some(ref last_id) = cursor.last_seen_id {
            url.push_str(&format!("&after={}", last_id));
        }

        let items: Vec<Value> = reqwest::Client::new()
            .get(&url)
            .bearer_auth(cred.token.expose_secret())
            .send()
            .await
            .map_err(ActionError::retryable)?
            .json()
            .await
            .map_err(ActionError::fatal)?;

        // Update cursor with latest item ID
        if let Some(last) = items.last() {
            cursor.last_seen_id = last.get("id").and_then(|v| v.as_str()).map(String::from);
            cursor.last_poll_at = Some(chrono::Utc::now().to_rfc3339());
        }

        Ok(items)
    }
}
```

**What `PollAction` does for you:**
- `start()` → schedules first poll via `ctx.schedule_after(poll_interval())`
- Each poll: call `poll(cursor)` → for each event, `ctx.emit_execution(event)`
- Cursor checkpointed after each poll — survives restarts
- `stop()` → unschedules, saves final cursor state

---

## 9. Closure-Based Action — `stateless_fn()`

For one-off actions, testing, or rapid prototyping — no struct needed.

```rust
use nebula_action::prelude::*;
use serde_json::{json, Value};

// Minimal: no struct, no derive, no trait impl
let echo = stateless_fn(
    ActionMetadata::new("echo", "Echo Action"),
    |input: Value, _ctx: &ActionContext| async move {
        Ok(ActionResult::success(input))
    },
);

// With transformation
let uppercase = stateless_fn(
    ActionMetadata::new("uppercase", "Uppercase Text"),
    |input: Value, _ctx: &ActionContext| async move {
        let text = input.as_str().unwrap_or_default().to_uppercase();
        Ok(ActionResult::success(json!(text)))
    },
);

// Register in registry
registry.register(echo)?;
registry.register(uppercase)?;
```

---

## 10. Manual Registration (No Proc Macros)

For users who prefer full control (Bevy-style). This is the low-level API — it uses string-based `ActionMetadata` and bypasses the type-safe credential declarations that `#[action(credential = Type)]` provides.

```rust
use nebula_action::prelude::*;
use serde_json::Value;
use std::sync::Arc;

// 1. Build metadata manually
let metadata = ActionMetadata::builder("transform", "JSON Transform")
    .with_description("Transform JSON data using a mapping expression")
    .with_version(InterfaceVersion::new(1, 0))
    .with_inputs(vec![InputPort::flow("input")])
    .with_outputs(vec![
        OutputPort::flow("output", FlowKind::Main),
        OutputPort::flow("error", FlowKind::Error),
    ])
    .with_parameters(
        ParameterCollection::builder()
            .string("expression", |p| p.label("Transform Expression").required())
            .select("mode", |p| {
                p.label("Mode")
                    .default("jmespath")
                    .option("jmespath", "JMESPath")
                    .option("jsonata", "JSONata")
            })
            .build(),
    )
    .build();

// 2. Register with closure handler
registry.register_handler(
    metadata.key.clone(),
    metadata.version.clone(),
    Arc::new(FnHandler::new(metadata, |input: Value, ctx: &ActionContext| async move {
        let expression = ctx.parameter("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActionError::validation("expression is required"))?;

        let result = evaluate_jmespath(expression, &input)
            .map_err(|e| ActionError::fatal(e))?;

        Ok(ActionResult::success(result))
    })),
);
```

---

## 11. Testing Patterns

### 11.1 Basic StatelessAction Test

```rust
#[tokio::test]
async fn test_echo_action() {
    let action = Echo;
    let ctx = TestContextBuilder::minimal().build();
    let input = json!({"message": "hello"});

    let result = action.execute(input.clone(), &ctx).await;
    assert_success!(result, input);
}
```

### 11.2 Action with Credentials

```rust
#[tokio::test]
async fn test_http_request_with_auth() {
    let action = HttpRequest {
        url: "https://httpbin.org/get".into(),
        method: "GET".into(),
        headers: None,
        body: None,
        timeout: 30,
    };

    let ctx = TestContextBuilder::new()
        .with_credential::<BearerSecret>(BearerSecret {
            token: SecretString::new("test-token"),
        })
        .build();

    let result = action.execute(action.clone(), &ctx).await;
    assert_success!(result);

    // Verify logs
    let logger = ctx.spy_logger();
    assert!(!logger.contains("error"));
}
```

### 11.3 Testing Branch Logic

```rust
#[tokio::test]
async fn test_if_switch_true_branch() {
    let action = IfSwitch {
        condition: "data.value > 10".into(),
        mode: IfMode::Expression,
    };

    let ctx = TestContextBuilder::new()
        .with_input(json!({"data": {"value": 42}}))
        .build();

    let result = action.execute(action.clone(), &ctx).await;
    assert_branch!(result, "true");
}

#[tokio::test]
async fn test_if_switch_false_branch() {
    let action = IfSwitch {
        condition: "data.value > 10".into(),
        mode: IfMode::Expression,
    };

    let ctx = TestContextBuilder::new()
        .with_input(json!({"data": {"value": 3}}))
        .build();

    let result = action.execute(action.clone(), &ctx).await;
    assert_branch!(result, "false");
}
```

### 11.4 StatefulAction Test with Harness

```rust
#[tokio::test]
async fn test_paginated_fetch() {
    let action = PaginatedFetch {
        url: "https://api.example.com/items".into(),
        max_pages: 3,
        page_size: 10,
    };

    let ctx = TestContextBuilder::new()
        .with_credential::<ApiToken>(ApiToken {
            token: SecretString::new("test"),
        })
        .build();

    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    // Step 1: first page
    let result = harness.step(action.clone()).await.unwrap();
    assert_continue!(result);
    assert_eq!(harness.state::<PaginationState>().unwrap().pages_fetched, 1);

    // Step 2: second page
    let result = harness.step(action.clone()).await.unwrap();
    assert_continue!(result);

    // Step 3: third page (max_pages reached)
    let result = harness.step(action.clone()).await.unwrap();
    assert_break!(result);
    assert_eq!(harness.iterations(), 3);
}
```

### 11.5 Trigger Test with Harness

```rust
#[tokio::test]
async fn test_poll_trigger() {
    let action = PollNewItems;

    let ctx = TestContextBuilder::trigger()
        .with_credential::<ApiToken>(ApiToken {
            token: SecretString::new("test"),
        })
        .build_trigger();

    let mut harness = TriggerTestHarness::new(action, ctx);

    // Start trigger
    harness.start().await.unwrap();

    // Verify it scheduled a poll
    assert_eq!(harness.scheduled_delays().len(), 1);
    assert_eq!(harness.scheduled_delays()[0], Duration::from_secs(60));

    // Simulate poll with mock data
    harness.simulate_poll(vec![
        json!({"id": "1", "name": "Item 1"}),
        json!({"id": "2", "name": "Item 2"}),
    ]).await;

    // Verify executions emitted
    assert_eq!(harness.emitted_executions().len(), 2);

    // Stop
    harness.stop().await.unwrap();
}
```

### 11.6 Error Testing

```rust
#[tokio::test]
async fn test_missing_credential() {
    let action = HttpRequest {
        url: "https://example.com".into(),
        method: "GET".into(),
        headers: None,
        body: None,
        timeout: 30,
    };

    // Context WITHOUT the required credential
    let ctx = TestContextBuilder::minimal().build();

    let result = action.execute(action.clone(), &ctx).await;
    assert_fatal!(result);
}

#[tokio::test]
async fn test_validation_error() {
    let action = IfSwitch {
        condition: "".into(),  // Empty — should fail validation
        mode: IfMode::Expression,
    };

    let ctx = TestContextBuilder::minimal().build();
    let result = action.execute(action.clone(), &ctx).await;
    assert_validation_error!(result);
}
```

### 11.7 Spy Logger Assertions

```rust
#[tokio::test]
async fn test_action_logging() {
    let spy = SpyLogger::new();
    let ctx = TestContextBuilder::new()
        .with_logger(spy.clone())
        .build();

    // ... execute action ...

    // Assert specific log messages
    assert!(spy.contains("Processing request"));
    assert!(!spy.contains("error"));
    assert_eq!(spy.count(), 3);

    // Check log levels
    let entries = spy.entries();
    assert!(entries.iter().all(|(level, _)| *level != ActionLogLevel::Error));
}
```

---

## 12. Agent Action — ReActAgent (DX Type, Planned)

Autonomous LLM agent with tool use.

```rust
use nebula_action::prelude::*;
use nebula_action::dx::ReActAgent;
use serde_json::Value;

#[derive(Action)]
#[action(key = "research_agent", name = "Research Agent")]
#[action(credential = OpenAIKey)]
struct ResearchAgent;

// Plugin: PluginKey("ai") + ActionKey("research_agent")

impl ReActAgent for ResearchAgent {
    type Input = Value;
    type Output = Value;

    fn system_prompt(&self) -> &str {
        "You are a research assistant. Use the available tools to find information \
         and synthesize a comprehensive answer."
    }

    fn available_tools(&self) -> Vec<ToolSpec> {
        // Tools can also come from connected SupportPort nodes
        vec![
            ToolSpec {
                name: "web_search".into(),
                description: "Search the web for information".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
                hints: ToolHints {
                    idempotent: true,
                    read_only: true,
                    estimated_latency: Some(Duration::from_secs(2)),
                },
            },
        ]
    }

    fn max_iterations(&self) -> u32 { 10 }

    async fn think(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
        ctx: &AgentContext,
    ) -> Result<AgentStep, ActionError> {
        let cred = ctx.credential::<OpenAIKey>()?;

        let response = call_openai(
            &cred,
            messages,
            tools,
        ).await.map_err(ActionError::retryable)?;

        // Track token usage
        ctx.usage().add_tokens(
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
        );

        if let Some(tool_call) = response.tool_calls.first() {
            Ok(AgentStep::ToolCall(tool_call.clone()))
        } else {
            Ok(AgentStep::Response(response.content))
        }
    }

    async fn execute_tool(
        &self,
        tool_call: ToolCall,
        ctx: &AgentContext,
    ) -> Result<Value, ActionError> {
        match tool_call.name.as_str() {
            "web_search" => {
                let query = tool_call.arguments["query"].as_str()
                    .ok_or_else(|| ActionError::validation("query is required"))?;
                // ... perform search ...
                Ok(json!({"results": [...]}))
            }
            _ => Err(ActionError::validation(
                format!("Unknown tool: {}", tool_call.name),
            )),
        }
    }

    fn is_complete(&self, step: &AgentStep) -> Option<Value> {
        match step {
            AgentStep::Response(value) => Some(value.clone()),
            AgentStep::ToolCall(_) => None,
        }
    }
}
```

**What `ReActAgent` does for you:**
- Drives the think → tool → think loop internally
- Budget enforcement via `AgentContext` (max iterations, tokens, cost)
- Automatic `AgentOutcome::Park { BudgetExhausted }` when limits hit
- Tools from connected SupportPort nodes merged with `available_tools()`
- Token/cost usage tracked and reported via `CostMetrics`
