# Nebula API

REST API server для Nebula workflow engine.

## 🎯 Принципы

**API как точка входа** — тонкий HTTP-слой без бизнес-логики:
- ✅ Handlers только извлекают данные и делегируют в services/ports
- ✅ Бизнес-логика в engine/storage/credential через порты (traits)
- ✅ Выполнение workflow в отдельных workers (не блокируем HTTP запросы)
- ✅ RFC 9457 Problem Details для ошибок
- ✅ Production-ready middleware (tracing, CORS, compression, security headers)

## 📁 Структура

```
src/
├── lib.rs              # Точка входа, экспорты
├── app.rs              # Сборка Router + middleware
├── config.rs           # Конфигурация API
├── state.rs            # AppState (порты через traits)
├── errors.rs           # RFC 9457 Problem Details
├── extractors/         # Кастомные extractors
├── handlers/           # Тонкие HTTP handlers
│   ├── health.rs       # Health checks
│   ├── workflow.rs     # Workflow CRUD
│   └── execution.rs    # Workflow executions
├── middleware/         # Custom middleware
│   ├── auth.rs         # JWT authentication
│   ├── rate_limit.rs   # Rate limiting
│   ├── request_id.rs   # Request ID tracking
│   └── security_headers.rs
├── models/             # DTOs (Request/Response)
│   ├── health.rs
│   ├── workflow.rs
│   └── execution.rs
├── routes/             # Модульная маршрутизация
│   ├── mod.rs          # create_routes()
│   ├── health.rs
│   ├── workflow.rs
│   └── execution.rs
└── services/           # Business logic (пока пусто)
```

## 🔌 Порты (Ports & Adapters)

API зависит только от **traits** (портов), не от конкретных реализаций:

```rust
pub struct AppState {
    pub config: Arc<Config>,
    pub workflow_repo: Arc<dyn WorkflowRepo>,      // ← trait
    pub execution_repo: Arc<dyn ExecutionRepo>,    // ← trait
}
```

Конкретные реализации (Postgres, Redis, in-memory) подставляются при сборке приложения.

## 🚀 API Endpoints

### Health Checks

- `GET /health` — Health check (всегда доступен)
- `GET /ready` — Readiness check (проверка зависимостей)

### Workflows (API v1)

- `GET /api/v1/workflows` — List workflows
- `POST /api/v1/workflows` — Create workflow
- `GET /api/v1/workflows/:id` — Get workflow by ID
- `PUT /api/v1/workflows/:id` — Update workflow
- `DELETE /api/v1/workflows/:id` — Delete workflow

### Executions (API v1)

- `GET /api/v1/workflows/:workflow_id/executions` — List executions
- `POST /api/v1/workflows/:workflow_id/executions` — Start execution (202 Accepted)
- `GET /api/v1/executions/:id` — Get execution status
- `POST /api/v1/executions/:id/cancel` — Cancel execution

## 🛠 Использование

```rust
use nebula_api::{build_app, ApiConfig, AppState};
use nebula_storage::{InMemoryWorkflowRepo, InMemoryExecutionRepo};
use nebula_config::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create state with in-memory repos
    let config = Config::default();
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    
    let state = AppState::new(config, workflow_repo, execution_repo);

    // Build app with config
    let api_config = ApiConfig::default();
    let app = build_app(state, &api_config);

    // Serve
    let addr = api_config.bind_address;
    tracing::info!("Starting server on {}", addr);
    
    nebula_api::app::serve(app, addr).await?;
    
    Ok(())
}
```

## 🔒 Security

- **CORS** — настраивается через `ApiConfig`
- **Security Headers** — X-Content-Type-Options, и др.
- **Rate Limiting** — TODO: добавить governor/tower-governor
- **Authentication** — JWT middleware (TODO: реализовать валидацию)
- **Input Validation** — через ValidatedJson extractor

## 📊 Observability

- **Tracing** — tower-http TraceLayer для всех запросов
- **Request ID** — уникальный ID для каждого запроса
- **Error Logging** — автоматическое логирование ошибок в ApiError::into_response

## 🎨 Error Handling (RFC 9457)

Все ошибки возвращаются в формате Problem Details:

```json
{
  "type": "https://nebula.dev/problems/not-found",
  "title": "Not Found",
  "status": 404,
  "detail": "Workflow abc123 not found"
}
```

## 📝 TODO

- [ ] Реализовать JWT authentication middleware
- [ ] Добавить rate limiting (tower-governor)
- [ ] Реализовать service layer для бизнес-логики
- [ ] Добавить pagination для list endpoints
- [ ] WebSocket/SSE для real-time execution updates
- [ ] OpenAPI/Swagger documentation
- [ ] Request/Response validation
- [ ] Metrics (Prometheus)
- [ ] Idempotency keys для POST/PUT

## 📚 Документация

См. также:
- [REST API AXUM Guide](./docs/REST_API_AXUM_GUIDE.md) — полный гайд по production REST API
- [Architecture](./docs/architecture.md) — архитектура API
- [Nebula Architecture](../../docs/ARCHITECTURE.md) — общая архитектура системы

## 🧪 Тестирование

```bash
# Run tests
cargo test -p nebula-api

# Run with tracing
RUST_LOG=nebula_api=debug cargo test -p nebula-api
```

