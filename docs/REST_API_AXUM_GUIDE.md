# 🚀 REST API на Axum: Полный гайд по High-Load, безопасности и лучшим практикам (2025–2026)

> Этот документ собирает актуальную информацию и лучшие практики по построению production-grade REST API
> на Rust/Axum, включая высоконагруженные системы, безопасность и observability.
>
> **Для масштабирования до уровня n8n/Temporal** в рамках многокрейтовой архитектуры (workspace) API должен оставаться **точкой входа**, а не местом выполнения workflow: вся логика выполнения живёт в engine, workers и очередях; API только принимает запросы, валидирует, вызывает порты (traits) и возвращает ответы.

---

## Содержание

- [1. Архитектура проекта](#1-архитектура-проекта)
  - [1.1 API как точка входа в многокрейтовой платформе (n8n/Temporal-класс)](#11-api-как-точка-входа-в-многокрейтовой-платформе-n8ntemporal-класс)
- [2. Настройка Axum Router (Production-Grade)](#2-настройка-axum-router-production-grade)
- [3. High-Load: Производительность и масштабирование](#3-high-load-производительность-и-масштабирование)
  - [3.1 Tokio Runtime — правильная настройка](#31-tokio-runtime--правильная-настройка)
  - [3.2 Tower Middleware для High-Load](#32-tower-middleware-для-high-load)
  - [3.3 Connection Pool и State Management](#33-connection-pool-и-state-management)
  - [3.4 Graceful Shutdown](#34-graceful-shutdown)
  - [3.5 Пагинация и потоковые ответы](#35-пагинация-и-потоковые-ответы)
- [4. Безопасность (Security Best Practices 2025–2026)](#4-безопасность-security-best-practices-20252026)
  - [4.1 Security Headers](#41-security-headers)
  - [4.2 CORS — правильная конфигурация](#42-cors--правильная-конфигурация)
  - [4.3 Аутентификация — JWT middleware](#43-аутентификация--jwt-middleware)
  - [4.4 RBAC (Role-Based Access Control)](#44-rbac-role-based-access-control)
  - [4.5 Rate Limiting (per-IP, per-User)](#45-rate-limiting-per-ip-per-user)
  - [4.6 Input Validation (Защита от инъекций)](#46-input-validation-защита-от-инъекций)
  - [4.7 Защита от Out-of-Order API Execution (OWASP)](#47-защита-от-out-of-order-api-execution-owasp)
  - [4.8 Sensitive Data в HTTP запросах (OWASP)](#48-sensitive-data-в-http-запросах-owasp)
- [5. Единая обработка ошибок](#5-единая-обработка-ошибок)
- [6. RFC 9457 — Problem Details for HTTP APIs (подробно)](#6-rfc-9457--problem-details-for-http-apis-подробно)
- [7. Продвинутая обработка ошибок: thiserror + anyhow](#7-продвинутая-обработка-ошибок-thiserror--anyhow)
- [8. Observability (Логирование, метрики, трейсинг)](#8-observability-логирование-метрики-трейсинг)
- [9. HTTP Status Codes — полная таблица для REST API](#9-http-status-codes--полная-таблица-для-rest-api)
- [10. Idempotency Keys и Cache-Control](#10-idempotency-keys-и-cache-control)
- [11. Версионирование API](#11-версионирование-api)
- [12. Тестирование API](#12-тестирование-api)
- [13. Экосистема Axum — полезные библиотеки](#13-экосистема-axum--полезные-библиотеки)
- [14. Чеклист Production REST API (2025–2026)](#14-чеклист-production-rest-api-20252026)
- [15. Краткое резюме ключевых принципов](#15-краткое-резюме-ключевых-принципов)

---

## 1. Архитектура проекта

### Структура папок (Clean / Hexagonal Architecture)

```text
src/
├── main.rs              # Точка входа, настройка runtime
├── app.rs               # Сборка Router, middleware
├── config.rs            # Конфигурация (env, файлы)
├── routes/
│   ├── mod.rs
│   ├── users.rs         # Группа роутов /api/v1/users
│   └── health.rs        # /health, /ready
├── handlers/            # Тонкие обработчики (только извлечение + делегация)
├── services/            # Бизнес-логика
├── repositories/        # Слой данных (DB, cache)
├── models/              # Domain models + DTOs
├── errors.rs            # Единый тип ошибок + IntoResponse
├── extractors/          # Кастомные extractors
├── middleware/           # Auth, logging, rate-limit
└── state.rs             # AppState
```

### Принцип: Handler → Service → Repository

```rust
// Handler — тонкий, только извлечение и делегация
async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    // Валидация
    payload.validate()?;

    // Делегация в сервис
    let user = state.user_service.create(payload).await?;

    Ok(Json(UserResponse::from(user)))
}

// Service — бизнес-логика
// Repository — только доступ к данным
// Это позволяет:
// ✅ Тестировать каждый слой отдельно
// ✅ Подменять реализации (mock)
// ✅ Разделять ответственности
```

### 1.1 API как точка входа в многокрейтовой платформе (n8n/Temporal-класс)

Чтобы масштабировать REST API до уровня систем вроде n8n или Temporal в **многокрейтовом workspace**, нужен чёткий фундамент: **API — тонкий слой входа**, не перегруженный логикой выполнения.

#### Ключевой принцип

| Кто | Ответственность |
|-----|------------------|
| **API (nebula-api)** | HTTP: маршрутизация, валидация входа, вызов портов (traits), маппинг ошибок в HTTP, middleware (auth, rate limit, trace). |
| **Engine / Workers / Queue** | Выполнение workflow, планирование шагов, очередь задач, состояние запусков. Живут в отдельных крейтах или процессах. |
| **Storage** | Персистенция определений workflow и состояния выполнений. Реализации за портами. |
| **App (main binary)** | Сборка: создаёт Router, подставляет в State реализации портов (WorkflowRepo, ExecutionRepo, TaskQueue), запускает workers и сервер. |

API **не должен**: запускать шаги workflow в потоке запроса, держать в себе движок выполнения, дублировать бизнес-правила из engine/credential/resource.

#### Многокрейтовая схема (Ports & Adapters)

```text
                    HTTP-клиент / Webhook
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  nebula-api (один порт: /health, /api/v1/*, /webhooks/*)        │
│  • Роуты, middleware, DTO ↔ доменные типы                       │
│  • State: Arc<dyn WorkflowRepo>, Arc<dyn ExecutionRepo>, …       │
│  • Handler: извлёк → вызвал сервис/порт → вернул ответ         │
└─────────────────────────────────────────────────────────────────┘
    │                    │                    │
    │ (traits)           │ (traits)           │ (traits)
    ▼                    ▼                    ▼
┌──────────┐    ┌──────────────┐    ┌─────────────────┐
│ Workflow │    │ Execution    │    │ TaskQueue       │
│ Repo     │    │ Repo         │    │ (enqueue run)   │
└──────────┘    └──────────────┘    └─────────────────┘
    │                    │                    │
    ▼                    ▼                    ▼
  drivers / app:  Postgres, engine, queue-memory, worker loop
```

API зависит только от **портов** (traits из `nebula-ports` или аналог). Конкретные реализации (БД, очередь, движок) подставляются при сборке приложения (main/app crate). Так API остаётся тестируемым (mock-реализации портов) и не тянет за собой тяжёлые зависимости выполнения.

#### Паттерн «Enqueue and Return»

Для запуска workflow и масштабирования:

1. **POST /api/v1/workflows/:id/run** — handler валидирует запрос и права, вызывает порт (например, `TaskQueue::enqueue` или `ExecutionRepo::start_run`), **не** ждёт завершения выполнения.
2. Ответ: **202 Accepted** + `Location: /api/v1/runs/:run_id` и тело с `run_id`.
3. Клиент узнаёт статус через **GET /api/v1/runs/:id** (polling) или подписку (WebSocket/SSE).

Выполнение происходит в **workers** (отдельный пул задач или процесс), которые забирают задачи из очереди. API не блокирует запрос на `engine.run()` до конца — иначе один долгий workflow забил бы поток и не дал бы масштабироваться.

#### Что класть в API crate, что — нет

| В API (допустимо) | Не в API (вынести в engine/storage/credential) |
|-------------------|-------------------------------------------------|
| Валидация DTO (длина, формат, права) | Правила переходов состояний workflow |
| Маппинг доменных ошибок → HTTP + RFC 9457 | Логика выполнения узлов DAG |
| Вызов методов портов (list, get, create, enqueue) | Ротация credentials, политики ресурсов |
| Auth middleware, rate limit, idempotency key | Решение «какой узел выполнить следующим» |

#### Антипаттерны

- **Выполнение в handler:** `let result = state.engine.run(workflow_id).await` в HTTP handler — выполнение не должно жить в API; только enqueue + возврат идентификатора run.
- **Прямая зависимость API от engine/storage крейтов:** API должен зависеть от **traits** (портов), а не от `nebula-engine` или конкретного драйвера БД; иначе тесты и замена бэкенда усложняются.
- **Бизнес-логика в API:** проверки вида «можно ли перейти из состояния A в B» относятся к домену (engine/workflow); API только передаёт вызов в сервис/порт и возвращает ошибку, если порт вернул конфликт.

#### Связь с документацией Nebula

- **CONSTITUTION (nebula-api):** «No business logic in API crate» — HTTP layer only; engine/storage/credential вызываются через порты.
- **INTERACTIONS:** API — downstream от webhook; engine, storage, workers — downstream от app; API получает в state уже собранные реализации портов.

Соблюдение этого разделения позволяет масштабировать API горизонтально (несколько инстансов за балансировщиком), а выполнение — отдельно (пул workers, отдельные очереди), как в n8n и Temporal.

---

## 2. Настройка Axum Router (Production-Grade)

### 2.1 Порядок Middleware (Onion Model vs ServiceBuilder)

> **Источник:** [Axum middleware docs](https://docs.rs/axum/latest/axum/middleware/index.html), [axum/src/docs/middleware.md](https://github.com/tokio-rs/axum/blob/main/axum/src/docs/middleware.md)

Понимание порядка выполнения middleware — **критически важно** для production API.

**С `Router::layer` — снизу вверх (onion model):**

```text
let app = Router::new()
    .route("/", get(handler))
    .layer(layer_one)       // ← оборачивает handler
    .layer(layer_two)       // ← оборачивает layer_one + handler
    .layer(layer_three);    // ← оборачивает всё

// Порядок выполнения:
//   Request:  layer_three → layer_two → layer_one → handler
//   Response: handler → layer_one → layer_two → layer_three
```

Визуализация (из официальной документации Axum):

```text
        requests
           |
           v
+----- layer_three -----+
| +---- layer_two ----+ |
| | +-- layer_one --+ | |
| | |               | | |
| | |    handler    | | |
| | |               | | |
| | +-- layer_one --+ | |
| +---- layer_two ----+ |
+----- layer_three -----+
           |
           v
        responses
```

**С `ServiceBuilder` — сверху вниз (рекомендуется):**

```text
let app = Router::new()
    .route("/", get(handler))
    .layer(
        ServiceBuilder::new()
            .layer(layer_one)     // ← выполняется ПЕРВЫМ
            .layer(layer_two)     // ← выполняется ВТОРЫМ
            .layer(layer_three),  // ← выполняется ТРЕТЬИМ
    );

// Порядок выполнения:
//   Request:  layer_one → layer_two → layer_three → handler
//   Response: handler → layer_three → layer_two → layer_one
```

> ⚠️ **Важно:** `ServiceBuilder` выполняет middleware **сверху вниз** — это проще для понимания и является рекомендуемым подходом. Любой middleware может прервать цепочку и вернуть ответ досрочно (например, при неудачной авторизации).

### 2.2 Production Router

```rust
use axum::{
    Router,
    routing::{get, post, put, delete},
    error_handling::HandleErrorLayer,
    http::StatusCode,
    BoxError,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer,
    compression::CompressionLayer,
    trace::TraceLayer,
    timeout::TimeoutLayer,
    limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer,
    catch_panic::CatchPanicLayer,
};
use std::time::Duration;

pub fn build_router(state: AppState) -> Router {
    let api_v1 = Router::new()
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", get(get_user).put(update_user).delete(delete_user));

    Router::new()
        // Версионирование API через prefix
        .nest("/api/v1", api_v1)

        // Health checks (без middleware аутентификации!)
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))

        // Глобальные middleware (ServiceBuilder — сверху вниз)
        .layer(
            ServiceBuilder::new()
                // 1. Catch panics — не роняем сервер
                .layer(CatchPanicLayer::new())
                // 2. Трейсинг запросов
                .layer(TraceLayer::new_for_http())
                // 3. Обработка ошибок от timeout
                .layer(HandleErrorLayer::new(|_: BoxError| async {
                    StatusCode::REQUEST_TIMEOUT
                }))
                // 4. Таймаут на запрос — 10с
                .layer(TimeoutLayer::new(Duration::from_secs(10)))
                // 5. Лимит тела запроса — 2MB
                .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
                // 6. Сжатие ответов (gzip, brotli, zstd)
                .layer(CompressionLayer::new())
                // 7. CORS
                .layer(build_cors_layer())
                // 8. Security headers
                .layer(security_headers_layer())
        )
        .with_state(state)
}
```

### 2.3 Написание кастомных middleware

> **Источник:** [Axum middleware guide](https://docs.rs/axum/latest/axum/middleware/index.html)

Axum предлагает несколько способов написания middleware:

| Способ | Когда использовать |
|---|---|
| `middleware::from_fn` | Простой async/await синтаксис, internal middleware |
| `middleware::from_extractor` | Тип используется и как extractor, и как middleware |
| `ServiceBuilder::map_request/map_response` | Мелкие ad-hoc операции (добавить header) |
| `tower::Service` + `BoxFuture` | Конфигурируемый middleware, публикация как crate |
| `tower::Service` + custom futures | Максимальная производительность, zero overhead |

**Шаблон кастомного `tower::Service` middleware:**

```rust
use axum::{response::Response, body::Body, extract::Request};
use futures_core::future::BoxFuture;
use tower::{Service, Layer};
use std::task::{Context, Poll};

#[derive(Clone)]
struct MyLayer;

impl<S> Layer<S> for MyLayer {
    type Service = MyMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MyMiddleware { inner }
    }
}

#[derive(Clone)]
struct MyMiddleware<S> {
    inner: S,
}

impl<S> Service<Request> for MyMiddleware<S>
where
    S: Service<Request, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let future = self.inner.call(request);
        Box::pin(async move {
            let response: Response = future.await?;
            Ok(response)
        })
    }
}
```

> ⚠️ **Принцип из документации Axum:** middleware должен **всегда возвращать response**, а не bail out с кастомным error. Если middleware использует `type Error = BoxError` (а не `Infallible`), **обязательно** оберните его в `HandleErrorLayer`:

```rust
ServiceBuilder::new()
    .layer(HandleErrorLayer::new(|_: BoxError| async {
        StatusCode::BAD_REQUEST
    }))
    .layer(your_fallible_middleware_layer)
```

### 2.4 Передача state в middleware и из middleware в handlers

**Доступ к state в `from_fn` middleware:**

```rust
use axum::{middleware, extract::State};

// Используйте from_fn_with_state для доступа к AppState
let app = Router::new()
    .route("/", get(handler))
    .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
    .with_state(state);
```

**Передача данных из middleware в handler через request extensions:**

```rust
use axum::{extract::{Request, Extension}, middleware::Next, http::StatusCode, response::Response};

#[derive(Clone)]
struct CurrentUser { id: String, roles: Vec<String> }

async fn auth(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req.headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let auth_header = match auth_header {
        Some(h) => h,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    if let Some(current_user) = authorize(auth_header).await {
        // ✅ Вставляем в extensions — handler может извлечь через Extension
        req.extensions_mut().insert(current_user);
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn handler(Extension(user): Extension<CurrentUser>) {
    // Используем user, установленный middleware
}
```

### 2.5 Backpressure и Load Shedding

> **Источник:** [Axum middleware docs — Routing and backpressure](https://docs.rs/axum/latest/axum/middleware/index.html)

Axum считает все сервисы **всегда готовыми** (`Poll::Ready(Ok(()))` из `poll_ready`). Это означает:

- **Не используйте** middleware, чувствительный к backpressure, напрямую в роутере
- **Обязательно** используйте `load_shed` чтобы запросы сбрасывались быстро при перегрузке
- Backpressure-чувствительный middleware можно применить **вокруг всего приложения**:

```rust
use tower::ServiceBuilder;

let app = Router::new().route("/", get(handler));

// Backpressure-чувствительный middleware оборачивает ВСЁ приложение
let app = ServiceBuilder::new()
    .layer(some_backpressure_sensitive_middleware)
    .service(app);
```

> 💡 **Примечание:** Handlers из async функций не заботятся о backpressure и всегда готовы. Если вы не используете Tower middleware — можно не беспокоиться об этом.

### 2.6 Перезапись URI в middleware

Middleware, добавленный через `Router::layer`, выполняется **после** routing. Для перезаписи URI нужно обернуть middleware вокруг всего `Router`:

```rust
use tower::Layer;
use axum::{Router, ServiceExt};

fn rewrite_request_uri(req: Request) -> Request {
    // ... перезапись URI
    req
}

let middleware = tower::util::MapRequestLayer::new(rewrite_request_uri);
let app = Router::new();

// Middleware выполнится ДО роутинга
let app_with_middleware = middleware.layer(app);

let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
axum::serve(listener, app_with_middleware.into_make_service()).await.unwrap();
```

---

## 3. High-Load: Производительность и масштабирование

### 3.1 Tokio Runtime — правильная настройка

```rust
use tokio::runtime::Builder;

fn main() {
    let runtime = Builder::new_multi_thread()
        // Кол-во worker threads = кол-во CPU cores (default)
        // Для CPU-bound: num_cpus
        // Для I/O-bound: можно 2x num_cpus
        .worker_threads(num_cpus::get())

        // Ограничить blocking threads (default = 512, слишком много!)
        .max_blocking_threads(64)

        // Имя потоков для дебага
        .thread_name("nebula-worker")

        // Размер стека (по умолчанию 2MB, увеличить для глубокой рекурсии)
        .thread_stack_size(3 * 1024 * 1024)

        // Включить все (IO, time, etc.)
        .enable_all()

        .build()
        .expect("Failed to build Tokio runtime");

    runtime.block_on(async {
        serve().await;
    });
}
```

### 3.2 Tower Middleware для High-Load

```rust
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower::load_shed::LoadShedLayer;
use tower::timeout::TimeoutLayer;

// Многослойная защита от перегрузки
let layers = ServiceBuilder::new()
    // 1. Load Shedding — отклоняет запросы когда сервис перегружен
    //    Возвращает 503 Service Unavailable вместо queuing
    .load_shed()

    // 2. Concurrency Limit — макс. одновременных запросов
    //    Подбирать под нагрузку: начать с 1024, мониторить latency
    .concurrency_limit(1024)

    // 3. Timeout — не даём запросам висеть вечно
    .timeout(Duration::from_secs(10))

    // 4. Rate Limiting (tower-governor) — per-IP/per-user
    .layer(GovernorLayer {
        config: Arc::new(
            GovernorConfigBuilder::default()
                .per_second(50)        // 50 req/sec per key
                .burst_size(100)       // burst до 100
                .key_extractor(SmartIpKeyExtractor)
                .finish()
                .unwrap(),
        ),
    });

// Применить к роутеру
let app = Router::new()
    .nest("/api/v1", api_routes)
    .layer(layers);
```

### 3.3 Connection Pool и State Management

```rust
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub cache: Arc<redis::Client>,
    pub config: Arc<AppConfig>,
    // НЕ Mutex<T> для read-heavy данных — используем RwLock или DashMap
    pub hot_cache: Arc<dashmap::DashMap<String, CachedValue>>,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        let db = PgPoolOptions::new()
            // Пул = 4× кол-во CPU cores для большинства случаев
            .max_connections((num_cpus::get() * 4) as u32)
            // Минимум живых соединений
            .min_connections(4)
            // Таймаут на получение соединения из пула
            .acquire_timeout(Duration::from_secs(5))
            // Макс. время жизни соединения (предотвращает stale connections)
            .max_lifetime(Duration::from_mins(30))
            // Idle timeout
            .idle_timeout(Duration::from_mins(10))
            // Тест соединения при выдаче
            .test_before_acquire(true)
            .connect(&config.database_url)
            .await
            .expect("Failed to create DB pool");

        Self {
            db,
            cache: Arc::new(redis::Client::open(config.redis_url.clone()).unwrap()),
            config: Arc::new(config),
            hot_cache: Arc::new(dashmap::DashMap::new()),
        }
    }
}
```

### 3.4 Graceful Shutdown

```rust
use tokio::net::TcpListener;
use tokio::signal;

async fn serve() {
    let state = AppState::new(load_config()).await;
    let app = build_router(state.clone());

    let listener = TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind");

    tracing::info!("Listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    // Cleanup после shutdown
    state.db.close().await;
    tracing::info!("Server shut down gracefully");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C"),
        _ = terminate => tracing::info!("Received SIGTERM"),
    }
}
```

### 3.5 Пагинация и потоковые ответы

```rust
use serde::{Deserialize, Serialize};

// Cursor-based пагинация (лучше offset для high-load!)
#[derive(Debug, Deserialize)]
pub struct CursorParams {
    pub cursor: Option<String>,  // opaque cursor
    pub limit: Option<u32>,      // default = 50, max = 100
}

impl CursorParams {
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(50).min(100)
    }
}

#[derive(Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// Handler
async fn list_users(
    State(state): State<AppState>,
    Query(params): Query<CursorParams>,
) -> Result<Json<PaginatedResponse<UserResponse>>, AppError> {
    let limit = params.limit();
    // Запрашиваем limit + 1 чтобы узнать есть ли ещё данные
    let fetch_limit = limit + 1;

    let users = state.user_service
        .list(params.cursor.as_deref(), fetch_limit)
        .await?;

    let has_more = users.len() > limit as usize;
    let data: Vec<UserResponse> = users
        .into_iter()
        .take(limit as usize)
        .map(UserResponse::from)
        .collect();

    let next_cursor = if has_more {
        data.last().map(|u| encode_cursor(&u.id))
    } else {
        None
    };

    Ok(Json(PaginatedResponse { data, next_cursor, has_more }))
}
```

---

## 4. Безопасность (Security Best Practices 2025–2026)

### 4.1 Security Headers

```rust
use axum::http::{HeaderName, HeaderValue};
use tower_http::set_header::SetResponseHeaderLayer;
use tower::ServiceBuilder;

pub fn security_headers_layer() -> impl tower::Layer</* ... */> + Clone {
    ServiceBuilder::new()
        // Предотвращает MIME sniffing
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        // Запрещает iframe embedding (Clickjacking protection)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        // Content Security Policy
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ))
        // Strict Transport Security (HTTPS only)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
        ))
        // Отключить Referrer для API
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
        // Permissions Policy (запрет камера, микрофон и т.д.)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
}
```

### 4.2 CORS — правильная конфигурация

```rust
use tower_http::cors::{CorsLayer, Any};
use axum::http::{Method, HeaderValue, header::HeaderName};

pub fn build_cors_layer() -> CorsLayer {
    // ⚠️ НИКОГДА не используйте .allow_origin(Any) в production!
    CorsLayer::new()
        // Только доверенные origins
        .allow_origin([
            "https://app.example.com".parse::<HeaderValue>().unwrap(),
            "https://admin.example.com".parse::<HeaderValue>().unwrap(),
        ])
        // Разрешённые методы
        .allow_methods([
            Method::GET, Method::POST, Method::PUT,
            Method::DELETE, Method::PATCH, Method::OPTIONS,
        ])
        // Разрешённые заголовки
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::ACCEPT,
            HeaderName::from_static("x-request-id"),
        ])
        // Expose заголовки клиенту
        .expose_headers([
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-ratelimit-remaining"),
        ])
        // Credentials (cookies, auth headers)
        .allow_credentials(true)
        // Cache preflight на 1 час
        .max_age(Duration::from_secs(3600))
}
```

### 4.3 Аутентификация — JWT middleware

```rust
use axum::{
    extract::Request,
    http::{StatusCode, HeaderMap},
    middleware::Next,
    response::{Response, IntoResponse, Json},
};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,         // user id
    pub exp: u64,            // expiration (UNIX timestamp)
    pub iat: u64,            // issued at
    pub roles: Vec<String>,  // RBAC roles
    pub jti: String,         // unique token id (для revocation)
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub roles: Vec<String>,
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized("Missing token".into()))?;

    // Проверка в чёрном списке (revoked tokens)
    if state.token_blacklist.contains(token) {
        return Err(AppError::Unauthorized("Token revoked".into()));
    }

    let mut validation = Validation::new(Algorithm::ES256); // ← ES256, НЕ HS256!
    validation.set_audience(&["nebula-api"]);
    validation.set_issuer(&["nebula-auth"]);
    validation.leeway = 30; // 30 секунд leeway для clock skew

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_ec_pem(&state.config.jwt_public_key)?,
        &validation,
    )
    .map_err(|e| AppError::Unauthorized(format!("Invalid token: {e}")))?;

    let auth_user = AuthUser {
        id: token_data.claims.sub,
        roles: token_data.claims.roles,
    };

    // Вставляем в extensions для доступа в handlers
    request.extensions_mut().insert(auth_user);

    Ok(next.run(request).await)
}

// Extractor для удобства
#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AuthUser>()
            .cloned()
            .ok_or(AppError::Unauthorized("Not authenticated".into()))
    }
}
```

### 4.4 RBAC (Role-Based Access Control)

```rust
use axum::{extract::Request, middleware::Next, response::Response};

/// Middleware-фабрика для проверки ролей
pub fn require_role(
    required: &'static str,
) -> impl Fn(AuthUser, Request, Next) -> impl Future<Output = Result<Response, AppError>> + Clone {
    move |user: AuthUser, request: Request, next: Next| {
        let required = required;
        async move {
            if !user.roles.iter().any(|r| r == required || r == "admin") {
                return Err(AppError::Forbidden(format!(
                    "Role '{required}' required"
                )));
            }
            Ok(next.run(request).await)
        }
    }
}

// Использование в роутере:
let admin_routes = Router::new()
    .route("/admin/users", get(admin_list_users))
    .route("/admin/settings", put(update_settings))
    .route_layer(middleware::from_fn(require_role("admin")));

let editor_routes = Router::new()
    .route("/content", post(create_content).put(update_content))
    .route_layer(middleware::from_fn(require_role("editor")));

let app = Router::new()
    .merge(admin_routes)
    .merge(editor_routes)
    .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));
```

### 4.5 Rate Limiting (per-IP, per-User)

```rust
use tower_governor::{
    GovernorLayer, GovernorConfigBuilder,
    governor::GovernorConfig,
    key_extractor::KeyExtractor,
};

// Кастомный extractor — per-user если авторизован, иначе per-IP
#[derive(Clone)]
pub struct SmartKeyExtractor;

impl KeyExtractor for SmartKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &http::Request<T>) -> Result<Self::Key, GovernorError> {
        // Сначала пробуем user_id из extensions
        if let Some(user) = req.extensions().get::<AuthUser>() {
            return Ok(format!("user:{}", user.id));
        }

        // Fallback на IP (учитываем X-Forwarded-For / CF-Connecting-IP)
        let ip = req.headers()
            .get("cf-connecting-ip")
            .or_else(|| req.headers().get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");

        Ok(format!("ip:{ip}"))
    }

    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some(key.clone())
    }
}

// Разные лимиты для разных групп эндпоинтов
pub fn strict_rate_limit() -> GovernorLayer<SmartKeyExtractor, GovernorConfig<SmartKeyExtractor>> {
    let config = GovernorConfigBuilder::default()
        .per_second(10)
        .burst_size(20)
        .key_extractor(SmartKeyExtractor)
        .finish()
        .unwrap();
    GovernorLayer { config: Arc::new(config) }
}

pub fn relaxed_rate_limit() -> GovernorLayer<SmartKeyExtractor, GovernorConfig<SmartKeyExtractor>> {
    let config = GovernorConfigBuilder::default()
        .per_second(100)
        .burst_size(200)
        .key_extractor(SmartKeyExtractor)
        .finish()
        .unwrap();
    GovernorLayer { config: Arc::new(config) }
}

// Применение разных лимитов к разным группам роутов
let auth_routes = Router::new()
    .route("/login", post(login))
    .route("/register", post(register))
    .layer(strict_rate_limit());  // 10 req/sec — защита от brute-force

let api_routes = Router::new()
    .nest("/users", user_routes)
    .nest("/data", data_routes)
    .layer(relaxed_rate_limit()); // 100 req/sec — обычные API
```

### 4.6 Input Validation (Защита от инъекций) — OWASP

> **Источник:** [OWASP REST Security Cheat Sheet — Input Validation](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)

**Ключевые правила OWASP:**
- **Не доверяй входным параметрам/объектам** — валидируй длину, диапазон, формат и тип
- Используй **сильные типы** (числа, булевы, даты) вместо строк где возможно
- Ограничивай строки **регулярными выражениями**
- Отклоняй неожиданный/запрещённый контент
- Определи **лимит размера запроса** и отклоняй запросы сверх лимита с HTTP 413
- **Логируй провалы валидации** — сотни ошибок в секунду = атака
- Используй **безопасный парсер** (защита от XXE для XML)
- **Валидируй Content-Type** — отклоняй запросы с неожиданным типом контента (415 Unsupported Media Type)

```rust
use serde::Deserialize;
use validator::Validate;
use axum::{extract::rejection::JsonRejection, Json};

// DTO с валидацией
#[derive(Debug, Deserialize, Validate)]
pub struct CreateUserRequest {
    #[validate(length(min = 2, max = 50, message = "Name must be 2-50 chars"))]
    #[validate(custom(function = "validate_no_html"))]
    pub name: String,

    #[validate(email(message = "Invalid email format"))]
    #[validate(length(max = 254))]
    pub email: String,

    #[validate(length(min = 12, message = "Password must be at least 12 chars"))]
    pub password: String,

    #[validate(range(min = 0, max = 150))]
    pub age: Option<u8>,
}

// Кастомный валидатор — запрет HTML/script injection
fn validate_no_html(value: &str) -> Result<(), validator::ValidationError> {
    if value.contains('<') || value.contains('>') || value.contains("script") {
        return Err(validator::ValidationError::new("contains_html"));
    }
    Ok(())
}

// Кастомный extractor с валидацией
pub struct ValidatedJson<T>(pub T);

#[axum::async_trait]
impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    T: for<'de> Deserialize<'de> + Validate,
    S: Send + Sync,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON: {e}")))?;

        value.validate()
            .map_err(|e| AppError::ValidationError(e))?;

        Ok(ValidatedJson(value))
    }
}

// Использование в handler — чистый и безопасный
async fn create_user(
    State(state): State<AppState>,
    ValidatedJson(payload): ValidatedJson<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    let user = state.user_service.create(payload).await?;
    Ok(Json(user.into()))
}
```

### 4.7 Защита от Out-of-Order API Execution (OWASP)

> **Источник:** [OWASP REST Security Cheat Sheet — Preventing Out-of-Order API Execution](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)

Современные REST API часто реализуют бизнес-потоки через последовательность эндпоинтов (create → validate → approve → finalize). Если бэкенд не валидирует переходы состояний workflow, атакующие могут вызывать эндпоинты не по порядку, обходя контроль.

**Проблема:** Атакующий может:
- Пропустить обязательные шаги workflow, вызвав эндпоинт поздней стадии напрямую
- Переиспользовать токены между стадиями
- Эксплуатировать предположение, что фронтенд контролирует порядок

**Пример атаки:**
```rust
// Ожидаемая последовательность:
// POST /checkout/create → POST /checkout/pay → POST /checkout/confirm
//
// Атакующий вызывает напрямую:
// POST /checkout/confirm   ← без оплаты!
```

**Защита:**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowState {
    Created,
    Validated,
    Paid,
    Confirmed,
}

impl WorkflowState {
    /// Проверяет допустимость перехода между состояниями
    pub fn can_transition_to(&self, next: &WorkflowState) -> bool {
        matches!(
            (self, next),
            (WorkflowState::Created, WorkflowState::Validated)
                | (WorkflowState::Validated, WorkflowState::Paid)
                | (WorkflowState::Paid, WorkflowState::Confirmed)
        )
    }
}

async fn confirm_checkout(
    State(state): State<AppState>,
    Path(checkout_id): Path<Uuid>,
) -> Result<Json<CheckoutResponse>, AppError> {
    let checkout = state.checkout_service.get(checkout_id).await?;

    // ✅ Валидация состояния workflow на сервере
    if !checkout.state.can_transition_to(&WorkflowState::Confirmed) {
        return Err(AppError::BadRequest(format!(
            "Cannot confirm checkout in state {:?}. Payment required first.",
            checkout.state
        )));
    }

    let result = state.checkout_service.confirm(checkout_id).await?;
    Ok(Json(result))
}
```

**Чеклист тестирования (OWASP):**
- [ ] Можно ли вызвать эндпоинты не по порядку?
- [ ] Каждый эндпоинт валидирует текущее состояние workflow?
- [ ] Токены переиспользуемы между шагами?
- [ ] Невалидные переходы состояний отклоняются?

---

### 4.8 Sensitive Data в HTTP запросах (OWASP)

> **Источник:** [OWASP REST Security Cheat Sheet — Sensitive Information](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)

**Пароли, токены безопасности и API-ключи НЕ ДОЛЖНЫ появляться в URL**, так как это может быть захвачено в логах web-сервера.

| Метод | Где передавать sensitive data |
|---|---|
| POST/PUT | В теле запроса или заголовках |
| GET | В HTTP заголовках (НЕ в query params) |

```text
✅ OK:  https://api.example.com/users/123/profile
✅ OK:  Authorization: Bearer <token>

❌ BAD: https://api.example.com/users?apiKey=a53f435643de32
❌ BAD: https://api.example.com/auth?token=secret123
```

**Дополнительно:**
- Не выставляйте management эндпоинты в интернет
- Если management эндпоинты доступны через интернет — требуйте MFA
- Используйте отдельные порты/хосты для management endpoints
- Ограничивайте доступ файрволом или ACL

---

## 5. Единая обработка ошибок

```rust
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response, Json},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Validation error")]
    ValidationError(validator::ValidationErrors),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Service unavailable")]
    ServiceUnavailable,
}

// Стандартизированный JSON ответ ошибки (RFC 9457 — Problem Details)
#[derive(Serialize)]
struct ErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    title: String,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<FieldError>>,
}

#[derive(Serialize)]
struct FieldError {
    field: String,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, detail, field_errors) = match &self {
            AppError::NotFound(msg) => {
                (StatusCode::NOT_FOUND, "not_found", msg.clone(), None)
            }
            AppError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, "bad_request", msg.clone(), None)
            }
            AppError::Unauthorized(msg) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone(), None)
            }
            AppError::Forbidden(msg) => {
                (StatusCode::FORBIDDEN, "forbidden", msg.clone(), None)
            }
            AppError::Conflict(msg) => {
                (StatusCode::CONFLICT, "conflict", msg.clone(), None)
            }
            AppError::RateLimited => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Too many requests".into(), None)
            }
            AppError::ValidationError(errs) => {
                let fields: Vec<FieldError> = errs.field_errors()
                    .into_iter()
                    .flat_map(|(field, errors)| {
                        errors.iter().map(move |e| FieldError {
                            field: field.to_string(),
                            message: e.message.as_ref()
                                .map(|m| m.to_string())
                                .unwrap_or_default(),
                        })
                    })
                    .collect();
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "validation_error",
                    "Validation failed".into(),
                    Some(fields),
                )
            }
            AppError::Internal(msg) => {
                // ⚠️ НИКОГДА не показывать внутренние ошибки клиенту
                tracing::error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".into(),
                    None,
                )
            }
            AppError::ServiceUnavailable => {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "Service temporarily unavailable".into(),
                    None,
                )
            }
        };

        let body = ErrorResponse {
            error_type: error_type.into(),
            title: status.canonical_reason().unwrap_or("Error").into(),
            status: status.as_u16(),
            detail,
            instance: None,
            errors: field_errors,
        };

        (status, Json(body)).into_response()
    }
}
```

---

## 6. RFC 9457 — Problem Details for HTTP APIs (подробно)

> **Источник:** [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) (July 2023, заменяет RFC 7807)

RFC 9457 определяет стандартный формат для передачи машиночитаемых деталей ошибок в HTTP ответах.
Это **текущий стандарт** (2025–2026) для оформления ошибок в REST API.

### Обязательные поля Problem Details объекта

| Поле | Тип | Описание |
|---|---|---|
| `type` | URI | Идентификатор типа ошибки. По умолчанию `"about:blank"`. Рекомендуется абсолютный URI |
| `status` | integer | HTTP status code (100–599). Дублирует статус ответа для удобства |
| `title` | string | Краткое human-readable описание типа ошибки. **Не должно меняться** от случая к случаю |
| `detail` | string | Human-readable объяснение конкретного случая ошибки. Фокус на **помощи клиенту исправить проблему** |
| `instance` | URI | Идентификатор конкретного случая ошибки (для поддержки/forensics) |

### Расширения (Extension Members)

Типы проблем **МОГУТ** расширять объект дополнительными полями. Клиенты **ОБЯЗАНЫ** игнорировать неизвестные расширения.

### Полный пример (из RFC 9457)

```json
{
    "type": "https://example.com/probs/out-of-credit",
    "title": "You do not have enough credit.",
    "status": 403,
    "detail": "Your current balance is 30, but that costs 50.",
    "instance": "/account/12345/msgs/abc",
    "balance": 30,
    "accounts": ["/account/12345", "/account/67890"]
}
```

### Пример с Validation Errors (из RFC 9457)

```json
{
    "type": "https://example.net/validation-error",
    "title": "Your request is not valid.",
    "status": 422,
    "errors": [
        {
            "detail": "must be a positive integer",
            "pointer": "#/age"
        },
        {
            "detail": "must be 'green', 'red' or 'blue'",
            "pointer": "#/profile/color"
        }
    ]
}
```

### Реализация в Axum (обновлённая, по RFC 9457)

```rust
use axum::{http::StatusCode, response::{IntoResponse, Response, Json}};
use serde::Serialize;

/// RFC 9457 Problem Details — Content-Type: application/problem+json
#[derive(Serialize)]
pub struct ProblemDetails {
    /// URI идентификатор типа проблемы
    #[serde(rename = "type")]
    pub problem_type: String,

    /// Краткое описание типа проблемы (не меняется)
    pub title: String,

    /// HTTP status code
    pub status: u16,

    /// Описание конкретного случая (помощь клиенту)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// URI конкретного случая ошибки
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,

    /// Расширение: ошибки валидации полей
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ValidationFieldError>>,
}

#[derive(Serialize)]
pub struct ValidationFieldError {
    pub detail: String,
    pub pointer: String, // JSON Pointer (RFC 6901), e.g. "#/age"
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, problem) = match &self {
            AppError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                ProblemDetails {
                    problem_type: "https://api.example.com/problems/not-found".into(),
                    title: "Resource not found".into(),
                    status: 404,
                    detail: Some(msg.clone()),
                    instance: None,
                    errors: None,
                },
            ),
            AppError::ValidationError(errs) => {
                let field_errors: Vec<ValidationFieldError> = errs
                    .field_errors()
                    .into_iter()
                    .flat_map(|(field, errors)| {
                        errors.iter().map(move |e| ValidationFieldError {
                            pointer: format!("#/{field}"),
                            detail: e.message.as_ref()
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| format!("Invalid value for {field}")),
                        })
                    })
                    .collect();
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ProblemDetails {
                        problem_type: "https://api.example.com/problems/validation-error".into(),
                        title: "Your request is not valid.".into(),
                        status: 422,
                        detail: Some("One or more fields failed validation.".into()),
                        instance: None,
                        errors: Some(field_errors),
                    },
                )
            }
            AppError::Internal(msg) => {
                // ⚠️ Security: НИКОГДА не раскрывать внутренние детали клиенту
                tracing::error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails {
                        problem_type: "about:blank".into(),
                        title: "Internal Server Error".into(),
                        status: 500,
                        detail: None, // Намеренно пустое
                        instance: None,
                        errors: None,
                    },
                )
            }
            // ... другие варианты
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ProblemDetails {
                    problem_type: "about:blank".into(),
                    title: "Internal Server Error".into(),
                    status: 500,
                    detail: None,
                    instance: None,
                    errors: None,
                },
            ),
        };

        // ✅ RFC 9457: Content-Type MUST be application/problem+json
        let mut response = (status, Json(problem)).into_response();
        response.headers_mut().insert(
            "content-type",
            "application/problem+json".parse().unwrap(),
        );
        response
    }
}
```

### Рекомендации RFC 9457 по безопасности

> ⚠️ **Из RFC 9457, Section 5 — Security Considerations:**
>
> - Тщательно проверяйте информацию, включаемую в problem details
> - **Не раскрывайте** stack traces, детали реализации, внутренние данные через HTTP интерфейс
> - `status` в теле может расходиться с HTTP status code (например, из-за прокси/кэша)
> - Генераторы **ОБЯЗАНЫ** использовать одинаковый status code в HTTP ответе и в поле `status`

---

## 7. Продвинутая обработка ошибок: thiserror + anyhow

> **Источник:** [Luca Palmieri — Error Handling In Rust: A Deep Dive](https://www.lpalmieri.com/posts/error-handling-rust/) (из книги "Zero To Production In Rust")

### Ментальная модель ошибок

Ошибки служат **двум целям**:
1. **Control flow** — определить, что делать дальше (для машины)
2. **Reporting** — расследовать, что пошло не так (для человека)

И имеют **две локации**:
- **Internal** — функция вызывает другую функцию внутри приложения
- **At the edge** — API запрос, который мы не смогли выполнить

|  | Internal | At the edge |
|---|---|---|
| **Control Flow** | Типы, методы, поля (enum variants) | Status codes |
| **Reporting** | Logs / traces | Response body |

### Правило: кто должен логировать ошибки?

> **Ошибки должны логироваться когда они обрабатываются, а не когда пробрасываются.**

- Если функция пробрасывает ошибку через `?` — она **НЕ должна** её логировать
- Она может добавить контекст (`.context("...")`)
- Логирование делегируется middleware (`TraceLayer`, `TracingLogger`)

```rust
// ❌ НЕПРАВИЛЬНО — логируем И пробрасываем
pub async fn store_token(/* */) -> Result<(), StoreTokenError> {
    sqlx::query!(/* */)
        .execute(transaction)
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute query: {:?}", e);  // ← лишний лог!
            StoreTokenError(e)
        })?;
    Ok(())
}

// ✅ ПРАВИЛЬНО — только пробрасываем с контекстом
pub async fn store_token(/* */) -> Result<(), StoreTokenError> {
    sqlx::query!(/* */)
        .execute(transaction)
        .await
        .map_err(StoreTokenError)?;
    Ok(())
}
```

### thiserror — генерация boilerplate для error types

`thiserror` — процедурный макрос для генерации `Display`, `Error::source` и `From` реализаций.

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SubscribeError {
    #[error("{0}")]
    ValidationError(String),

    #[error("Failed to acquire a Postgres connection from the pool")]
    PoolError(#[source] sqlx::Error),

    #[error("Failed to insert new subscriber in the database.")]
    InsertSubscriberError(#[source] sqlx::Error),

    #[error("Failed to store the confirmation token for a new subscriber.")]
    StoreTokenError(#[from] StoreTokenError),

    #[error("Failed to commit SQL transaction to store a new subscriber.")]
    TransactionCommitError(#[source] sqlx::Error),

    #[error("Failed to send a confirmation email.")]
    SendEmailError(#[from] reqwest::Error),
}
```

**Атрибуты thiserror:**
- `#[error("...")]` — задаёт `Display` представление
- `#[source]` — задаёт `Error::source()` (root cause)
- `#[from]` — автоматически генерирует `From<T>` + помечает как source

### Проблема "Ball of Mud" enum

Использовать один enum-вариант на каждый fallible вызов **не масштабируется**. Решение: разделять ошибки по уровням абстракции.

```rust
// ✅ Чистый error type на уровне API handler
#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    #[error("{0}")]
    ValidationError(String),

    // Непрозрачная ошибка — детали скрыты от caller'а
    #[error(transparent)]
    UnexpectedError(#[from] anyhow::Error),
}
```

### anyhow — непрозрачные ошибки с контекстом

`anyhow::Error` — обёртка над `Box<dyn std::error::Error>` с дополнительными возможностями:
- Требует `Send + Sync + 'static`
- Гарантирует backtrace
- Компактное представление (один pointer)
- Метод `.context()` для добавления контекста

```rust
use anyhow::Context;

pub async fn subscribe(/* */) -> Result<HttpResponse, SubscribeError> {
    let new_subscriber = form.0.try_into()
        .map_err(SubscribeError::ValidationError)?;

    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;

    let subscriber_id = insert_subscriber(&mut transaction, &new_subscriber)
        .await
        .context("Failed to insert new subscriber in the database.")?;

    store_token(&mut transaction, subscriber_id, &token)
        .await
        .context("Failed to store the confirmation token for a new subscriber.")?;

    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to store a new subscriber.")?;

    send_confirmation_email(&email_client, new_subscriber, &base_url, &token)
        .await
        .context("Failed to send a confirmation email.")?;

    Ok(HttpResponse::Ok().finish())
}
```

### Итоговая цепочка ошибок в логах

```text
exception.details=
    "Failed to store the confirmation token for a new subscriber.

    Caused by:
        A database failure was encountered while trying to store
        a subscription token.
    Caused by:
        error returned from database: column 'subscription_token'
        of relation 'subscription_tokens' does not exist"
```

### Утилита для отображения цепочки ошибок

```rust
fn error_chain_fmt(
    e: &impl std::error::Error,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    writeln!(f, "{}\n", e)?;
    let mut current = e.source();
    while let Some(cause) = current {
        writeln!(f, "Caused by:\n\t{}", cause)?;
        current = cause.source();
    }
    Ok(())
}

// Использование для Debug реализации
impl std::fmt::Debug for SubscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        error_chain_fmt(self, f)
    }
}
```

### anyhow или thiserror?

> **Миф:** "anyhow для приложений, thiserror для библиотек"
>
> **Правильный подход:** рассуждайте об **intent** (намерении):

| Ситуация | Инструмент |
|---|---|
| Caller должен реагировать по-разному на разные ошибки | **thiserror** (enum с вариантами) |
| Caller просто отказывается при ошибке, важен только отчёт | **anyhow** (непрозрачная ошибка) |
| Библиотека — пользователям нужен контроль | **thiserror** |
| Внутренний код — ошибка прокидывается до handler'а | **anyhow** + `.context()` |

---

## 8. Observability (Логирование, метрики, трейсинг)

```rust
use axum::{extract::Request, middleware::Next, response::Response};
use tracing::{info_span, Instrument};
use uuid::Uuid;
use std::time::Instant;

#[derive(Clone)]
struct RequestId(String);

// Request ID middleware — каждому запросу уникальный ID
pub async fn request_id_middleware(
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| Uuid::now_v7().to_string()); // UUIDv7 — сортируемый по времени

    request.extensions_mut().insert(RequestId(request_id.clone()));

    let mut response = next.run(request).await;

    response.headers_mut().insert(
        "x-request-id",
        request_id.parse().unwrap(),
    );

    response
}

// Structured logging middleware
pub async fn logging_middleware(
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let request_id = request.extensions()
        .get::<RequestId>()
        .map(|r| r.0.clone())
        .unwrap_or_default();

    let span = info_span!(
        "http_request",
        method = %method,
        uri = %uri,
        request_id = %request_id,
        status = tracing::field::Empty,
        latency_ms = tracing::field::Empty,
    );

    let start = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let latency = start.elapsed();

    span.record("status", response.status().as_u16());
    span.record("latency_ms", latency.as_millis());

    // Structured log
    if response.status().is_server_error() {
        tracing::error!(parent: &span, "Request failed");
    } else if latency > Duration::from_secs(1) {
        tracing::warn!(parent: &span, "Slow request");
    } else {
        tracing::info!(parent: &span, "Request completed");
    }

    response
}
```

### Ключевые метрики для High-Load (Prometheus)

| Метрика | Тип | Описание |
|---|---|---|
| `http_requests_total{method, path, status}` | counter | Общее кол-во запросов |
| `http_request_duration_seconds{method, path}` | histogram | Время обработки запроса |
| `http_requests_in_flight` | gauge | Запросы в обработке прямо сейчас |
| `db_pool_connections{state}` | gauge | Соединения в пуле (active/idle) |
| `db_query_duration_seconds{query}` | histogram | Время выполнения SQL запросов |

### Audit Logs (OWASP)

> **Источник:** [OWASP REST Security Cheat Sheet — Audit Logs](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)

- Пишите audit logs **до и после** security-related событий
- Логируйте ошибки валидации токенов для обнаружения атак
- **Санитизируйте данные логов** перед записью (защита от log injection)

```rust
/// Аудит-лог для security-значимых действий
pub async fn audit_log(
    action: &str,
    user_id: Option<&str>,
    resource: &str,
    result: &str,
    details: &str,
) {
    tracing::info!(
        audit = true,
        action = action,
        user_id = user_id.unwrap_or("anonymous"),
        resource = resource,
        result = result,           // "success" | "failure" | "denied"
        details = %sanitize_log_input(details),  // ← защита от log injection
        "AUDIT"
    );
}

fn sanitize_log_input(input: &str) -> String {
    input
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
```

---

## 9. HTTP Status Codes — полная таблица для REST API

> **Источник:** [OWASP REST Security Cheat Sheet — HTTP Return Code](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)

Всегда используйте семантически правильный status code. Не используйте только `200` для успеха и `404` для ошибок.

### Success (2xx)

| Code | Сообщение | Описание | Когда использовать |
|---|---|---|---|
| 200 | OK | Успешный ответ | GET, PUT, PATCH, DELETE |
| 201 | Created | Ресурс создан. URI в заголовке `Location` | POST создание ресурса |
| 202 | Accepted | Запрос принят, обработка не завершена | Async операции |
| 204 | No Content | Успех без тела ответа | DELETE, PUT без возврата данных |

### Redirection (3xx)

| Code | Сообщение | Описание |
|---|---|---|
| 301 | Moved Permanently | Постоянное перенаправление |
| 304 | Not Modified | Кэш актуален (ETag/If-None-Match) |
| 307 | Temporary Redirect | Временное перенаправление (сохраняет метод) |

### Client Errors (4xx)

| Code | Сообщение | Описание |
|---|---|---|
| 400 | Bad Request | Малформированный запрос (неверный формат тела, JSON syntax error) |
| 401 | Unauthorized | Неверные/отсутствующие credentials |
| 403 | Forbidden | Аутентификация успешна, но нет permissions |
| 404 | Not Found | Ресурс не существует |
| 405 | Method Not Allowed | HTTP метод не поддерживается для ресурса |
| 406 | Not Acceptable | Content type из Accept не поддерживается |
| 409 | Conflict | Конфликт (duplicate, optimistic lock) |
| 413 | Payload Too Large | Превышен лимит размера запроса |
| 415 | Unsupported Media Type | Content-Type запроса не поддерживается |
| 422 | Unprocessable Entity | Валидация провалена (бизнес-правила) |
| 429 | Too Many Requests | Rate limit / DOS защита |

### Server Errors (5xx)

| Code | Сообщение | Описание |
|---|---|---|
| 500 | Internal Server Error | Неожиданная ошибка. **Не раскрывайте детали!** |
| 501 | Not Implemented | Операция ещё не реализована |
| 503 | Service Unavailable | Временная недоступность (перегрузка, maintenance). Клиент должен retry |

### Реализация в Axum — restrict HTTP methods

> **OWASP:** Применяйте allowlist разрешённых HTTP методов. Отклоняйте все запросы, не входящие в allowlist, с 405 Method Not Allowed.

```rust
use axum::routing::{get, post, put, delete, MethodRouter};

// Axum автоматически возвращает 405 для незарегистрированных методов на роуте
let app = Router::new()
    // GET и POST разрешены, остальные → 405
    .route("/users", get(list_users).post(create_user))
    // GET, PUT, DELETE разрешены, остальные → 405
    .route("/users/{id}", get(get_user).put(update_user).delete(delete_user));
```

---

## 10. Idempotency Keys и Cache-Control

### Idempotency Keys

Для **POST** и **PUT** запросов, которые создают ресурсы или инициируют действия, используйте idempotency keys чтобы предотвратить дублирование при retry.

```rust
use axum::{extract::Request, http::HeaderMap, middleware::Next, response::Response};
use uuid::Uuid;

/// Middleware для обработки idempotency keys
pub async fn idempotency_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // Только для мутирующих методов
    if !matches!(
        request.method(),
        &http::Method::POST | &http::Method::PUT | &http::Method::PATCH
    ) {
        return Ok(next.run(request).await);
    }

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    if let Some(key) = &idempotency_key {
        // Проверяем, был ли запрос уже обработан
        if let Some(cached_response) = state.idempotency_cache.get(key) {
            tracing::info!(idempotency_key = %key, "Returning cached response");
            return Ok(cached_response);
        }
    }

    let response = next.run(request).await;

    // Кэшируем результат для idempotency key
    if let Some(key) = idempotency_key {
        if response.status().is_success() {
            state.idempotency_cache.insert(key, response.clone());
        }
    }

    Ok(response)
}
```

### Cache-Control для API ответов

> **OWASP:** Заголовок `Cache-Control: no-store` предотвращает кэширование чувствительных данных браузерами.

```rust
use tower_http::set_header::SetResponseHeaderLayer;
use axum::http::{HeaderName, HeaderValue};

// Для API с чувствительными данными — запрет кэширования
let no_cache_layer = SetResponseHeaderLayer::overriding(
    HeaderName::from_static("cache-control"),
    HeaderValue::from_static("no-store"),
);

// Для публичных неизменяемых данных — разрешить кэширование
let cache_layer = SetResponseHeaderLayer::overriding(
    HeaderName::from_static("cache-control"),
    HeaderValue::from_static("public, max-age=3600, stale-while-revalidate=60"),
);
```

### ETag + Conditional Requests

```rust
use axum::{extract::Path, http::{HeaderMap, StatusCode}, response::IntoResponse, Json};
use sha2::{Sha256, Digest};

async fn get_user(
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let user = state.user_service.get(id).await?;
    let body = serde_json::to_vec(&user)?;

    // Генерируем ETag из содержимого
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let etag = format!("\"{}\"", hex::encode(hasher.finalize()));

    // Conditional GET — если клиент уже имеет актуальную версию
    if let Some(if_none_match) = headers.get("if-none-match") {
        if if_none_match.to_str().ok() == Some(etag.as_str()) {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    Ok((
        StatusCode::OK,
        [
            ("etag", etag.as_str()),
            ("cache-control", "private, max-age=0, must-revalidate"),
        ],
        Json(user),
    ).into_response())
}
```

---

## 11. Версионирование API

```rust
// ✅ Рекомендация 2025–2026: URL path versioning — самый простой и явный

// Подход 1: Nest (рекомендуемый)
let app = Router::new()
    .nest("/api/v1", v1_routes())
    .nest("/api/v2", v2_routes());

fn v1_routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(v1::list_users).post(v1::create_user))
        .route("/users/{id}", get(v1::get_user))
}

fn v2_routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(v2::list_users).post(v2::create_user))
        .route("/users/{id}", get(v2::get_user))
        // V2 добавляет новые endpoint'ы
        .route("/users/{id}/preferences", get(v2::get_preferences))
}

// Подход 2: Header-based (Accept-Version) — для advanced сценариев
pub async fn version_router(
    headers: HeaderMap,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let version = headers
        .get("accept-version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("v1");

    match version {
        "v2" => { /* route to v2 handler */ }
        _ =>    { /* route to v1 handler */ }
    }
}
```

### Deprecation Headers

При закате старой версии API добавляйте заголовки:

```text
Sunset: Sat, 01 Mar 2026 00:00:00 GMT
Deprecation: true
Link: <https://api.example.com/api/v2/docs>; rel="successor-version"
```

---

## 12. Тестирование API

```rust
use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::json;

// Интеграционные тесты с axum-test
#[tokio::test]
async fn test_create_user_success() {
    let app = build_test_app().await;
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/users")
        .json(&json!({
            "name": "Ivan Petrov",
            "email": "ivan@example.com",
            "password": "super_secure_password_123"
        }))
        .await;

    response.assert_status(StatusCode::CREATED);
    response.assert_json_contains(&json!({
        "name": "Ivan Petrov",
        "email": "ivan@example.com"
    }));
    // Пароль НЕ должен возвращаться
    assert!(!response.text().contains("password"));
}

#[tokio::test]
async fn test_create_user_validation_error() {
    let app = build_test_app().await;
    let server = TestServer::new(app).unwrap();

    let response = server
        .post("/api/v1/users")
        .json(&json!({
            "name": "X",  // too short
            "email": "not-an-email",
            "password": "short"
        }))
        .await;

    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    response.assert_json_contains(&json!({
        "error_type": "validation_error"
    }));
}

#[tokio::test]
async fn test_unauthorized_without_token() {
    let app = build_test_app().await;
    let server = TestServer::new(app).unwrap();

    let response = server.get("/api/v1/users").await;
    response.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_rate_limiting() {
    let app = build_test_app().await;
    let server = TestServer::new(app).unwrap();

    // Flood the endpoint
    for _ in 0..25 {
        server.post("/api/v1/auth/login")
            .json(&json!({"email": "a@b.com", "password": "x"}))
            .await;
    }

    // Should be rate limited
    let response = server.post("/api/v1/auth/login")
        .json(&json!({"email": "a@b.com", "password": "x"}))
        .await;

    response.assert_status(StatusCode::TOO_MANY_REQUESTS);
}
```

---

## 13. Экосистема Axum — полезные библиотеки

> **Источник:** [Axum ECOSYSTEM.md](https://github.com/tokio-rs/axum/blob/main/ECOSYSTEM.md) — официальный список community проектов

### 🔒 Аутентификация и авторизация

| Библиотека | Описание |
|---|---|
| [axum-login](https://github.com/maxcountryman/axum-login) | Session-based аутентификация |
| [axum-gate](https://github.com/) | JWT auth с Cookie и Bearer (монолит + микросервисы) |
| [axum-keycloak-auth](https://github.com/) | Защита роутов JWT от Keycloak |
| [jwt-authorizer](https://github.com/) | JWT авторизация (OIDC discovery, валидация, claims) |
| [aliri_axum](https://github.com/) | JWT валидация + OAuth2 scopes |
| [axum-csrf-sync-pattern](https://github.com/) | CSRF защита для AJAX/API бэкендов |
| [axum-casbin-auth](https://github.com/) | Casbin access control middleware |

### 📊 Observability

| Библиотека | Описание |
|---|---|
| [axum-tracing-opentelemetry](https://github.com/) | Middleware для axum + tracing + OpenTelemetry |
| [tower-otel](https://github.com/) | OpenTelemetry layer для HTTP/gRPC с axum интеграцией |
| [axum-otel-metrics](https://github.com/) | OpenTelemetry Metrics с Prometheus exporter |
| [axum-prometheus](https://github.com/) | HTTP метрики, совместимо с metrics.rs exporters |

### 🛡️ Rate Limiting и Resilience

| Библиотека | Описание |
|---|---|
| [tower-governor](https://github.com/benwis/tower-governor) | Rate limiting на основе governor |
| [tower-resilience](https://github.com/) | Circuit breaker, bulkhead, retry, rate limiter |
| [tower_allowed_hosts](https://github.com/) | Middleware для ограничения по allowed hosts |

### 📦 Утилиты и расширения

| Библиотека | Описание |
|---|---|
| [axum-test](https://github.com/) | High-level библиотека для интеграционных тестов |
| [axum-valid](https://github.com/) | Extractors для валидации (validator, garde, validify) |
| [axum-typed-multipart](https://github.com/) | Type-safe multipart upload |
| [axum-typed-websockets](https://github.com/) | WebSocket с типизированными сообщениями |
| [aide](https://github.com/) | Code-first OpenAPI генерация с axum интеграцией |
| [axum-typed-routing](https://github.com/) | Статически типизированные routing макросы + OpenAPI |
| [tower-sessions](https://github.com/) | Sessions как tower/axum middleware |
| [socketioxide](https://github.com/) | Socket.IO сервер как tower layer |
| [axum-streams](https://github.com/) | Streaming HTTP body (JSON, CSV, Protobuf) |
| [axum-conditional-requests](https://github.com/) | Client-side caching HTTP headers (ETag, If-None-Match) |
| [axum-sqlx-tx](https://github.com/) | Request-bound SQLx транзакции с auto commit/rollback |
| [tower-cookies](https://github.com/) | Cookie manager middleware |
| [sigterm](https://github.com/) | Signal-aware async control для graceful shutdown |

### 🚀 Production-ready шаблоны и примеры

| Проект | Описание |
|---|---|
| [loco.rs](https://loco.rs) | Full stack framework в стиле Rails, на базе axum |
| [axum-postgres-template](https://github.com/) | Production-ready axum + PostgreSQL template |
| [clean_axum_demo](https://github.com/) | Clean architecture + DDD + JWT + OpenTelemetry |
| [axum-rest-api-example](https://github.com/) | REST API с JWT, SQLx, PostgreSQL, Redis, Docker |
| [realworld-axum-sqlx](https://github.com/) | Realworld spec implementation (axum + SQLx) |
| [spring-rs](https://github.com/) | Microservice framework inspired by Spring Boot |
| [zino](https://github.com/) | Next-gen framework для composable applications |

---

## 14. Чеклист Production REST API (2025–2026)

### 🏗️ Архитектура

| Требование | Детали |
|---|---|
| ✅ Handler → Service → Repository | Разделение ответственности |
| ✅ Cursor-based pagination | Не offset! Стабильнее при больших объёмах |
| ✅ URL path versioning (`/api/v1/`) | Явный, простой, кэшируемый |
| ✅ RFC 9457 Problem Details | `application/problem+json` формат ошибок |
| ✅ Idempotency keys | Для POST/PUT — `Idempotency-Key` header |
| ✅ ETag + If-None-Match | Conditional GET для кэширования |
| ✅ Workflow state validation | Защита от out-of-order вызовов (OWASP) |
| ✅ Proper HTTP status codes | Семантически правильные (OWASP таблица) |
| ✅ API как точка входа (multi-crate) | Зависимость от портов (traits); выполнение в engine/workers, не в handler; enqueue-and-return для run (202/Location) |

### ⚡ High-Load

| Требование | Детали |
|---|---|
| ✅ Tokio multi-thread runtime | worker_threads = num_cpus |
| ✅ Connection pooling | sqlx PgPool, redis pool |
| ✅ Load shedding | `tower::load_shed` — 503 вместо queueing |
| ✅ Concurrency limits | `tower::limit::ConcurrencyLimit` |
| ✅ Request timeouts | 10s API, 5s DB, 30s background |
| ✅ Compression | gzip/brotli/zstd через `tower-http` |
| ✅ Graceful shutdown | SIGTERM → drain connections → exit |
| ✅ Body size limits | `RequestBodyLimitLayer` — 2MB default |

### 🔒 Безопасность

| Требование | Детали |
|---|---|
| ✅ JWT с ES256 (не HS256!) | Асимметричная подпись, verify signature not MAC |
| ✅ JWT claims validation | `iss`, `aud`, `exp`, `nbf` — обязательно (OWASP) |
| ✅ Token revocation | Чёрный список + `jti` claim |
| ✅ No `{"alg":"none"}` JWT | Отклонять unsigned JWT (OWASP) |
| ✅ Rate limiting per-IP + per-User | tower-governor |
| ✅ Input validation | validator crate + custom extractors |
| ✅ Content-Type validation | Отклонять неожиданные типы (415) |
| ✅ Security headers | HSTS, CSP, X-Content-Type-Options, X-Frame-Options |
| ✅ Cache-Control: no-store | Для чувствительных данных (OWASP) |
| ✅ CORS — whitelist origins | НЕ `allow_origin(Any)` |
| ✅ No internal errors to client | Log internally, return generic message |
| ✅ SQL injection prevention | Prepared statements (sqlx) |
| ✅ Password hashing | argon2id (NOT bcrypt, NOT sha256) |
| ✅ HTTPS only | TLS termination at LB/reverse proxy |
| ✅ Request ID tracing | UUIDv7 для сортируемости |
| ✅ No secrets in URL | API keys, tokens только в headers/body (OWASP) |
| ✅ Audit logs | До и после security событий, sanitize input |
| ✅ Restrict HTTP methods | Allowlist, 405 для остальных (OWASP) |

### 📊 Observability

| Требование | Детали |
|---|---|
| ✅ Structured logging | `tracing` с JSON output |
| ✅ Request/response logging | method, path, status, latency_ms |
| ✅ Prometheus metrics | counters, histograms, gauges |
| ✅ Health + Readiness endpoints | `/health`, `/ready` |
| ✅ Distributed tracing | OpenTelemetry + trace_id propagation |
| ✅ Error chain logging | `Error::source()` chain в логах |
| ✅ Log at handle, not propagate | Логируем только при обработке ошибки |
| ✅ Audit logging | Security-значимые действия (OWASP) |

### 🧰 Error Handling

| Требование | Детали |
|---|---|
| ✅ thiserror для typed errors | Enum с вариантами для control flow |
| ✅ anyhow для opaque errors | `.context()` для enrichment |
| ✅ RFC 9457 Problem Details | `application/problem+json` content type |
| ✅ Разделение error layers | Handler errors ≠ service errors |
| ✅ Error::source chain | Полная цепочка причин для оператора |
| ✅ Generic errors для клиентов | Не раскрывать внутренности (OWASP) |

### 📦 Рекомендуемые зависимости (Cargo.toml)

```toml
[dependencies]
# Web framework
axum = "0.8"
tokio = { version = "1", features = ["full"] }
tower = { version = "0.5", features = ["full"] }
tower-http = { version = "0.6", features = [
    "cors", "compression-full", "trace", "timeout",
    "limit", "set-header", "catch-panic"
] }
tower-governor = "0.5"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }

# Auth
jsonwebtoken = "9"
argon2 = "0.5"

# Validation
validator = { version = "0.18", features = ["derive"] }

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
uuid = { version = "1", features = ["v7"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Hashing (ETag, checksums)
sha2 = "0.10"
hex = "0.4"

# OpenAPI documentation (optional)
# aide = "0.14"
# axum-typed-routing = "0.2"
```

---

## 15. Краткое резюме ключевых принципов

1. **Тонкие handlers** — только extraction + delegation, бизнес-логика в сервисах
2. **Слоистая защита** — load_shed → concurrency_limit → timeout → rate_limit → auth
3. **Fail fast** — валидация на входе, ранний возврат ошибок
4. **Никогда не доверяй input** — всё валидируем, sanitize, лимитируем размер (OWASP)
5. **Observability first** — request_id, structured logging, метрики с первого дня
6. **Graceful degradation** — 503 лучше чем зависший сервер; circuit breakers для зависимостей
7. **Security by default** — headers, CORS, rate limiting, JWT rotation, no secrets in URL
8. **Test everything** — интеграционные тесты для каждого endpoint, включая error cases
9. **Errors have purpose** — control flow (thiserror) vs reporting (anyhow), log at handle point
10. **Standards compliance** — RFC 9457 Problem Details, OWASP guidelines, proper HTTP status codes
11. **Workflow integrity** — server-side state machine validation, idempotency keys
12. **Defence in depth** — каждый слой проверяет безопасность самостоятельно
13. **API как точка входа (multi-crate)** — API зависит от портов (traits); выполнение в engine/workers; run = enqueue + 202 + Location; без тяжёлой логики в API crate (масштабирование n8n/Temporal-класс)

---

## Ссылки и источники

### Официальная документация
- [Axum Documentation](https://docs.rs/axum/latest/axum/)
- [Tower Documentation](https://docs.rs/tower/latest/tower/)
- [Tower HTTP Documentation](https://docs.rs/tower-http/latest/tower_http/)
- [Tokio Documentation](https://docs.rs/tokio/latest/tokio/)
- [tower-governor — Rate Limiting](https://docs.rs/tower-governor/latest/tower_governor/)
- [Axum ECOSYSTEM.md](https://github.com/tokio-rs/axum/blob/main/ECOSYSTEM.md) — список community библиотек

### Стандарты и спецификации
- [RFC 9457 — Problem Details for HTTP APIs](https://www.rfc-editor.org/rfc/rfc9457) (July 2023, заменяет RFC 7807)
- [RFC 6901 — JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901) (для `pointer` в validation errors)
- [RFC 9110 — HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110) (status codes, methods, headers)

### Безопасность
- [OWASP REST Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)
- [OWASP JSON Web Token Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_for_Java_Cheat_Sheet.html)
- [OWASP Transport Layer Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Transport_Layer_Security_Cheat_Sheet.html)
- [OWASP Input Validation Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Input_Validation_Cheat_Sheet.html)

### Книги и статьи
- [Luca Palmieri — Error Handling In Rust: A Deep Dive](https://www.lpalmieri.com/posts/error-handling-rust/) (Zero To Production In Rust)
- [Zero To Production In Rust](https://zero2prod.com/) — полная книга по backend разработке на Rust

### Туториалы и примеры
- [Rust on Nails](https://rust-on-nails.com/) — full stack architecture для Rust web apps
- [Rust Axum Full Course](https://www.youtube.com/) — YouTube видеокурс
- [axum-rest-api-postgres-redis-jwt-docker](https://github.com/) — getting started template
- [clean_axum_demo](https://github.com/) — clean architecture + DDD + JWT + OpenTelemetry
- [realworld-axum-sqlx](https://github.com/) — Realworld spec implementation

### Crates
- [thiserror](https://crates.io/crates/thiserror) — derive macro для error types
- [anyhow](https://crates.io/crates/anyhow) — гибкая обработка ошибок с контекстом
- [validator](https://crates.io/crates/validator) — валидация данных с derive макросами
- [axum-test](https://crates.io/crates/axum-test) — интеграционные тесты для axum
- [aide](https://crates.io/crates/aide) — code-first OpenAPI генерация