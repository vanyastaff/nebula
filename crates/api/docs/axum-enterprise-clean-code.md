# Enterprise Clean Code для Axum

Краткий гайд по структуре и практикам для поддерживаемых API на Axum (по материалам PropelAuth, clean-axum, HN/community).

---

## 1. Extractors вместо ручной разборки запроса

**Плохо:** в хендлере вручную читаем заголовки, парсим тело, проверяем авторизацию.

```rust
async fn save_url(
    headers: HeaderMap,
    State(pool): State<PgPool>,
    Json(create_url): Json<CreateUrl>,
) -> Response {
    let token = headers.get("X-Auth-Token").and_then(|h| h.to_str().ok());
    let user = match verify_token(token).await {
        Some(u) => u,
        None => return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    };
    // ... ещё 20 строк
}
```

**Хорошо:** вынести извлечение и проверку в кастомный extractor (`FromRequest` / `FromRequestParts`). Хендлер получает уже готовый тип.

```rust
// Один раз реализуем FromRequestParts для User
impl<S> FromRequestParts<S> for User
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("X-Auth-Token")
            .and_then(|h| h.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;
        verify_auth_token(auth_header)
            .await
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))
    }
}

// Хендлер только про бизнес-логику
async fn save_url(
    user: User,
    State(pool): State<PgPool>,
    Json(create_url): Json<CreateUrl>,
) -> Result<impl IntoResponse, ApiError> {
    let id = Db::save_url(&create_url.url, &user, &pool).await?;
    Ok((StatusCode::CREATED, Json(CreateUrlResponse { id })))
}
```

**Правило:** один extractor — одна ответственность; переиспользуем через параметры хендлера, а не копипастой.

---

## 2. Порядок extractors в сигнатуре

- **Body потребляется только один раз** — только один extractor, который реализует `FromRequest` (например `Json<T>`), и он должен быть **последним** параметром.
- Остальные — `FromRequestParts` (например `State`, заголовки, `User`): порядок между ними не важен, но они идут **до** body extractor.

```rust
// ✅ Верно: State и User до Json
async fn handler(
    user: User,
    State(state): State<AppState>,
    Json(body): Json<CreateRequest>,
) -> Result<impl IntoResponse, ApiError>

// ❌ Неверно: Json не последний
async fn handler(
    Json(body): Json<CreateRequest>,
    user: User,
    State(state): State<AppState>,
) -> ...
```

При непонятных ошибках компиляции можно использовать [axum-macros](https://github.com/tokio-rs/axum/discussions/641) для более ясных сообщений.

---

## 3. Ошибки: свой тип + IntoResponse + ?

Вместо `match` на каждый вызов — один тип ошибки API и `?` в хендлерах.

```rust
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("not found")]
    NotFound,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
            ApiError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "Not found"),
        };
        (status, body).into_response()
    }
}
```

Тогда в хендлере:

```rust
async fn save_url(...) -> Result<impl IntoResponse, ApiError> {
    let id = Db::save_url(&create_url.url, &user, &pool).await?;
    Ok((StatusCode::CREATED, Json(CreateUrlResponse { id })))
}
```

Для маршрутов с 3–4 разными сценариями ошибок можно явно маппить их в ответы; для типичных DB/auth — `ApiError` + `IntoResponse` уменьшают шум.

---

## 4. Разделение слоёв: хендлеры не знают про SQL/ORM

**Плохо:** SQL или вызовы репозитория прямо в хендлере.

**Хорошо:** слой приложения (persistence / use case) — отдельный модуль; хендлер только вызывает его и формирует HTTP-ответ.

```rust
// app::persistence или domain::url_service
pub async fn save_url(url: &str, user: &User, pool: &PgPool) -> Result<String, sqlx::Error> {
    sqlx::query_scalar!("INSERT INTO urls (url, user_id) VALUES (lower($1), $2) RETURNING id", url, user.id())
        .fetch_one(pool)
        .await
}

// api handler
async fn save_url(
    user: User,
    State(pool): State<PgPool>,
    Json(create_url): Json<CreateUrl>,
) -> Result<impl IntoResponse, ApiError> {
    let id = save_url(&create_url.url, &user, &pool).await?;
    Ok((StatusCode::CREATED, Json(CreateUrlResponse { id })))
}
```

Итог: «сохраняем URL и возвращаем ответ» видно по сигнатуре; детали БД — в другом слое.

---

## 5. Модели: не смешивать API и домен

- **Запрос/ответ API** — отдельные DTO (например `CreateUrl`, `CreateUrlResponse`) в слое API.
- **Домен/персистенция** — свои типы (entity, params, query); слой API собирает из них DTO.

Так проще версионировать API, менять формат ответа и тестировать бизнес-логику без HTTP.

---

## 6. Структура проекта (clean architecture–style)

Рекомендуемое разбиение (по аналогии с [clean-axum](https://github.com/kigawas/clean-axum)):

| Слой | Содержимое | Зависимости |
|------|------------|-------------|
| **API** | Роутеры, хендлеры, extractors, API-модели, маппинг ошибок в HTTP | axum, serde; не импортировать SQL/ORM в роутеры |
| **Application** | Use cases, сервисы, конфиг, состояние приложения (пулы, репозитории) | домен; без axum |
| **Persistence** | CRUD, запросы к БД, маппинг domain ↔ DB | домен, sqlx/SeaORM/… |
| **Domain** | Модели, ошибки домена, правила (без фреймворков и БД) | минимум зависимостей |

Принципы:

- Фреймворк (Axum) и БД можно менять, не ломая ядро.
- Тесты домена и use cases — без поднятия HTTP и БД.

---

## 7. State и конфигурация

- Один тип состояния приложения (например `AppState`) с полями: пул БД, конфиг, репозитории, shared handles.
- Пробрасывать через `Router::with_state(state)`.
- В хендлерах брать только нужное: `State<AppState>` или обёртки (например `Extension<PgPool>` при желании).

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<AppConfig>,
}

let app = Router::new()
    .route("/users", get(users_get).post(users_post))
    .with_state(state);
```

---

## 8. Валидация ввода

- Использовать один и тот же механизм для тела и query: например кастомный extractor `Valid<Json<T>>` с `validator` или аналогом, чтобы не дублировать проверки в хендлере.
- Ошибки валидации маппить в один формат (например 422 + JSON с полями ошибок) и в общий `ApiError`/`IntoResponse`.

---

## 9. Документация API

- Для enterprise-проектов имеет смысл держать контракт в OpenAPI (например [utoipa](https://github.com/juhaku/utoipa)) и отдавать Swagger UI / Scalar с `/docs`.
- Модели запроса/ответа и маршруты описывать через derive (`ToSchema`, `IntoParams`) и один раз привязать к роутерам.

---

## 10. Итоговый чеклист

- [ ] Auth/контекст вынесены в extractors (`FromRequestParts`), не размазаны по хендлерам.
- [ ] Один body extractor в конце списка параметров.
- [ ] Свой `ApiError` + `IntoResponse`; в хендлерах по возможности только `?`.
- [ ] В хендлерах нет SQL/ORM — только вызовы app/persistence слоя.
- [ ] Отдельные типы для API (request/response) и для домена/БД.
- [ ] Чёткое разделение: API → Application → Persistence → Domain.
- [ ] Состояние в одном `AppState`, передаётся через `with_state`.
- [ ] Валидация ввода централизована (extractor/слой), ответы по ошибкам единообразны.
- [ ] При необходимости — OpenAPI + UI под `/docs`.

---

## Ссылки

- [Clean Code with Rust & Axum (PropelAuth)](https://www.propelauth.com/post/clean-code-with-rust-and-axum) — extractors, IntoResponse, вынос SQL.
- [clean-axum](https://github.com/kigawas/clean-axum) — скелет проекта и разделение api/app/models.
- [Rustacean Clean Architecture (kigawas)](https://kigawas.me/posts/rustacean-clean-architecture-approach/) — обоснование слоёв и структуры.
- [Axum Extractors (docs.rs)](https://docs.rs/axum/latest/axum/extract/index.html) — порядок и ограничения extractors.
