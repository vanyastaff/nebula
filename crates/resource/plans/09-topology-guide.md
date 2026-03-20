# Выбор topology для ресурса

## Зачем topology

Внешний ресурс — база данных, API, бот, browser — имеет определённую **природу доступа**. Postgres connection stateful, одноразовый per-query. HTTP client stateless, shared. SSH — одно соединение, много сессий. Kafka consumer — один владелец, нельзя шарить.

Topology кодифицирует эту природу. Framework знает как управлять lifecycle: когда создавать, сколько держать, как переиспользовать, что делать при сбое. Resource author выбирает topology один раз — framework делает всё остальное.

Неправильный выбор topology = либо waste ресурсов (Pool для stateless client), либо race conditions (Resident для stateful connection), либо deadlock (не Exclusive для single-owner resource).

**Этот файл — standalone гайд.** Если вы реализуете новый ресурс, прочитайте только его. Decision tree → Quick-reference table → секция выбранной topology → примеры + анти-паттерны.

---

## Quick-reference: topology selection table

Найдите ваш ресурс (или похожий) и используйте рекомендованную topology:

| Resource type | Primary topology | Secondary | Rationale |
|---|---|---|---|
| **Postgres** | Pool | — | N interchangeable TCP connections, per-tenant prepare, server-side `max_connections` |
| **Redis (shared/multiplexed)** | Resident | EventSource (if pub/sub) | One `fred::Client`, internal multiplexing, `Clone = Arc` |
| **Redis (dedicated connections)** | Pool | — | N dedicated TCP connections, per-connection `SELECT db` |
| **HTTP Client** (`reqwest`) | Resident | — | Stateless, internal connection pool, `Clone = Arc` |
| **Telegram Bot** | Service | EventSource + Daemon | Long-lived bot + token handles + polling loop |
| **SSH** | Transport | — | One TCP connection + N multiplexed shell sessions |
| **WebSocket (outbound)** | Service | EventSource + Daemon | Long-lived connection + send handle tokens + incoming messages |
| **Postgres** | Pool | — | Stateful TCP connections, expensive create, server-side limits |
| **Kafka Producer** | Resident | — | One producer with internal buffer/batching, `Clone = Arc` |
| **Kafka Consumer** | Exclusive | — | Partition assignment requires single owner, concurrent access = rebalance storm |
| **gRPC** (`tonic::Channel`) | Resident | — | HTTP/2 multiplexed channel, internal load balancing, `Clone = Arc` |
| **SMTP** | Pool | — | N SMTP connections, per-connection session state (EHLO, AUTH, STARTTLS) |
| **Logger** (structured) | Resident | — | Shared logger handle, `Clone = Arc`, no per-caller state |
| **Metric** (collector) | Resident | — | Shared metrics registry/client, `Clone = Arc`, append-only |
| **Headless Browser** | Pool | — | N browser pages, heavy recycle (~500ms), per-page isolation |
| **LLM API client** | Resident | — | Stateless HTTP wrapper + rate limiter, `Clone = Arc` |
| **AMQP (RabbitMQ)** | Transport | — | One TCP connection + N channels |
| **Serial Port** | Exclusive | — | Physical device, single writer |

---

## Дерево решений

Следуйте дереву сверху вниз. Каждый вопрос — да/нет. Первое совпадение = ваша topology.

```
1. Ресурс нуждается в acquire/release?
   (Могут ли actions запрашивать handle у framework?)
   │
   ├─ НЕТ → Callers не взаимодействуют напрямую.
   │         Ресурс просто бежит в фоне (polling loop, scheduler, watcher).
   │         ──→ Daemon
   │         Примеры: Telegram polling loop, cron scheduler, metrics scraper.
   │
   └─ ДА → Продолжай ↓
      │
      2. Runtime реализует Clone И clone дешёвый (Arc inside)?
         │
         ├─ ДА → Есть ли per-caller mutable state?
         │        │
         │        ├─ НЕТ → Один shared instance, callers получают clone.
         │        │         ──→ Resident
         │        │         Примеры: reqwest::Client, fred::Client, tonic::Channel,
         │        │                  rdkafka::FutureProducer, logger handle, metrics client.
         │        │
         │        └─ ДА → Clone есть, но каждый caller меняет state?
         │                 Это значит Clone бессмыслен для sharing.
         │                 ──→ Переходи к вопросу 3
         │
         └─ НЕТ → Runtime НЕ Clone. Продолжай ↓
            │
            3. Это одно соединение с N мультиплексированными сессиями поверх?
               (Один дорогой TCP/TLS handshake, много дешёвых logical channels)
               │
               ├─ ДА ──→ Transport
               │         Примеры: SSH (1 TCP → N shell sessions),
               │                  AMQP (1 TCP → N channels).
               │
               └─ НЕТ ↓
                  │
                  4. Только один владелец допустим в любой момент времени?
                     (Concurrent access = corruption / rebalance / physical conflict)
                     │
                     ├─ ДА ──→ Exclusive
                     │         Примеры: Kafka consumer (partition assignment),
                     │                  serial port, file lock, GPIO pin.
                     │
                     └─ НЕТ ↓
                        │
                        5. Runtime = long-lived процесс, callers получают lightweight token/handle?
                           (Runtime сам не Clone, но создаёт cheap tokens для callers)
                           │
                           ├─ ДА ──→ Service
                           │         Примеры: Telegram Bot (bot handle + update rx),
                           │                  WebSocket outbound (mpsc::Sender),
                           │                  rate-limited API (semaphore permit).
                           │
                           └─ НЕТ ↓
                              │
                              6. N взаимозаменяемых stateful instances?
                                 (Создание дорогое, instances независимы, нужен recycle между callers)
                                 │
                                 └─ ДА ──→ Pool
                                           Примеры: Postgres connections, Redis dedicated,
                                                    SMTP connections, browser pages.
```

**Для входящих событий (incoming event stream):** добавьте `EventSource` как secondary capability
к любой primary topology. EventSource НЕ является primary topology — он дополняет другую.

**Для гибридов (outbound API + incoming events + background loop):**
используйте primary topology + `.also_event_source()` + `.also_daemon()` на builder.

---

## Маппинг целевых ресурсов через дерево

Для каждого ресурса из спецификации — путь через дерево:

| Resource | Q1 acquire? | Q2 Clone? | Q3 multiplex? | Q4 single-owner? | Q5 long-lived+token? | Q6 pool? | → Topology |
|---|---|---|---|---|---|---|---|
| **Logger** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **Metric** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **HTTP** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **gRPC** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **Kafka Producer** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **Redis (shared)** | ДА | ДА (Arc) | — | — | — | — | **Resident** |
| **SSH** | ДА | НЕТ | ДА (1 TCP → N sessions) | — | — | — | **Transport** |
| **Kafka Consumer** | ДА | НЕТ | НЕТ | ДА (partitions) | — | — | **Exclusive** |
| **Telegram Bot** | ДА | НЕТ | НЕТ | НЕТ | ДА (bot+tokens) | — | **Service** |
| **WebSocket** | ДА | НЕТ | НЕТ | НЕТ | ДА (conn+handles) | — | **Service** |
| **Postgres** | ДА | НЕТ | НЕТ | НЕТ | НЕТ | ДА | **Pool** |
| **Redis (dedicated)** | ДА | НЕТ | НЕТ | НЕТ | НЕТ | ДА | **Pool** |
| **SMTP** | ДА | НЕТ | НЕТ | НЕТ | НЕТ | ДА | **Pool** |

---

## Pool

### Когда

Runtime = stateful connection. Каждый instance независим и взаимозаменяем. Caller получает один instance, использует, возвращает. Следующий caller может получить тот же instance.

### Признаки

- Connection к серверу: TCP, WebSocket, Unix socket.
- Instance хранит state: prepared statements, transaction context, selected database.
- Создание дорогое: TCP handshake, TLS negotiation, auth.
- Нужен cleanup между использованиями: DISCARD ALL, UNWATCH, navigate blank.
- Ограничение на стороне сервера: `max_connections` в Postgres, maxclients в Redis.

### Real-world примеры

| Resource | Почему Pool | Create cost | Recycle | Server-side limit |
|---|---|---|---|---|
| **PostgreSQL** | TCP connection, per-connection state (search_path, prepared stmts) | ~50ms (TCP + auth) | DISCARD ALL ~1ms | `max_connections` (default 100) |
| **SMTP** | TCP + EHLO + AUTH + STARTTLS per connection, session state | ~100ms (TLS handshake) | RSET ~1ms | Varies by provider |
| **Redis Dedicated** | TCP, per-connection SELECT db, WATCH state | ~10ms | UNWATCH + SELECT 0 ~1ms | `maxclients` (default 10000) |
| **Headless Browser Page** | Chrome DevTools page in shared browser process | ~200ms | Clear cookies/storage ~500ms | Memory-bound |
| **MySQL** | TCP connection, per-connection variables | ~30ms | RESET CONNECTION | `max_connections` |

### Как выглядит

```rust
pub struct Postgres;

impl Resource for Postgres {
    type Config  = PgResourceConfig;
    type Runtime = PgConnection;
    type Lease   = PgConnection;       // = Runtime (Pool topology)
    type Error   = PgError;
    const KEY: ResourceKey = resource_key!("postgres");

    async fn create(&self, config: &PgResourceConfig, ctx: &dyn Ctx) -> Result<PgConnection, PgError> {
        // Credential access — design deferred.
        let cred = todo!("credential integration — see deferred design");
        let (client, conn) = tokio_postgres::Config::new()
            .host(&cred.host).port(cred.port)
            .dbname(&cred.database).user(&cred.username)
            .password(cred.password.expose())
            .connect(NoTls).await?;
        tokio::spawn(conn);
        Ok(PgConnection::new(client))
    }

    async fn check(&self, conn: &PgConnection) -> Result<(), PgError> {
        conn.client().simple_query("SELECT 1").await?;
        Ok(())
    }
}

impl Pooled for Postgres {
    fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
        if conn.client.is_closed() { BrokenCheck::Broken("closed".into()) }
        else { BrokenCheck::Healthy }
    }

    async fn recycle(&self, conn: &PgConnection, metrics: &InstanceMetrics)
        -> Result<RecycleDecision, PgError>
    {
        if metrics.error_count >= 5 { return Ok(RecycleDecision::Drop); }
        match conn.client.simple_query("DISCARD ALL").await {
            Ok(_) => Ok(RecycleDecision::Keep),
            Err(_) => Ok(RecycleDecision::Drop),
        }
    }

    async fn prepare(&self, conn: &PgConnection, ctx: &dyn Ctx) -> Result<(), PgError> {
        if let Some(tenant) = ctx.ext::<TenantContext>() {
            conn.client.simple_query(&format!("SET search_path TO {}", tenant.schema)).await?;
        }
        Ok(())
    }
}
```

**SMTP пример:**

```rust
pub struct SmtpConnection;

impl Resource for SmtpConnection {
    type Config  = SmtpResourceConfig;
    type Runtime = SmtpTransport;
    type Lease   = SmtpTransport;      // = Runtime (Pool topology)
    type Error   = SmtpError;
    const KEY: ResourceKey = resource_key!("smtp");

    async fn create(&self, config: &SmtpResourceConfig, ctx: &dyn Ctx) -> Result<SmtpTransport, SmtpError> {
        let cred = todo!("credential integration — see deferred design");
        let transport = lettre::AsyncSmtpTransport::<Tokio1Executor>::relay(&cred.host)?
            .port(cred.port)
            .credentials(Credentials::new(cred.username.clone(), cred.password.expose().to_string()))
            .build();
        Ok(SmtpTransport { inner: transport })
    }

    async fn check(&self, transport: &SmtpTransport) -> Result<(), SmtpError> {
        transport.inner.test_connection().await.map_err(SmtpError::Connection)?;
        Ok(())
    }
}

impl Pooled for SmtpConnection {
    fn is_broken(&self, transport: &SmtpTransport) -> BrokenCheck {
        if transport.inner.is_closed() { BrokenCheck::Broken("closed".into()) }
        else { BrokenCheck::Healthy }
    }

    async fn recycle(&self, transport: &SmtpTransport, _metrics: &InstanceMetrics)
        -> Result<RecycleDecision, SmtpError>
    {
        // RSET clears any in-progress mail transaction.
        match transport.inner.command(lettre::transport::smtp::commands::Rset).await {
            Ok(_) => Ok(RecycleDecision::Keep),
            Err(_) => Ok(RecycleDecision::Drop),
        }
    }
}
```

### Регистрация

```rust
manager.register(Postgres)
    .config(PgResourceConfig { connect_timeout: Duration::from_secs(5), ..Default::default() })
    .id(resource_id)
    .pool(pool::Config {
        min_size: 2, max_size: 20,
        strategy: pool::Strategy::Lifo,
        warmup: pool::WarmupStrategy::Staggered { delay: Duration::from_millis(200) },
        test_on_checkout: true,
        ..Default::default()
    })
    .build().await?;
```

### Анти-паттерны

- **❌ Pool для stateless client** (reqwest::Client). Clone дешевле pool checkout. → Используй **Resident**.
- **❌ Pool с max_size=1**. Это Exclusive с overhead pool machinery. → Используй **Exclusive**.
- **❌ Pool без recycle()**. Без recycle — state протекает между callers. Cross-tenant data leak.
- **❌ Pool без prepare()**. Без prepare — tenant isolation manual, caller может забыть.
- **❌ Pool для multiplexed protocol** (SSH, AMQP). Один TCP connection дешевле N connections. → Используй **Transport**.

---

## Resident

### Когда

Runtime = Clone, stateless или internally managed state. Один instance, все callers шарят через clone. Acquire = clone. Zero overhead, zero contention.

### Признаки

- Клиент внутри использует connection pooling сам (reqwest, fred.rs, rdkafka).
- Clone = дёшево (Arc inside).
- Нет per-caller state.
- Нет ограничения на количество concurrent callers.
- Создание дорогое но одноразовое.

### Real-world примеры

| Resource | Почему Resident | Что внутри Clone | Health check |
|---|---|---|---|
| **reqwest::Client** (HTTP) | HTTP/2 multiplexing, internal connection pool | Arc\<ClientInner\> | Нет (stateless) |
| **fred::Client** (Redis shared) | Multiplexed pipelining, internal reconnection | Arc\<ClientInner\> | `is_connected()` every 15s |
| **rdkafka::FutureProducer** (Kafka) | Internal buffer, background thread sends batches | Arc\<ProducerInner\> | `fetch_metadata()` via `check()` |
| **tonic::Channel** (gRPC) | HTTP/2 multiplexed, internal load balancing | Arc\<Channel\> | Нет (internal reconnect) |
| **Logger handle** | Shared structured logger, append-only | Arc\<LoggerInner\> | Нет (local) |
| **Metrics client** | Shared metrics registry | Arc\<MetricsInner\> | Нет (local) |
| **LLM API client** | HTTP client + rate limiter + usage tracker | Arc\<...\> | Нет (stateless) |

### Как выглядит

```rust
pub struct HttpClient;

impl Resource for HttpClient {
    type Config  = HttpConfig;
    type Runtime = reqwest::Client;
    type Lease   = reqwest::Client;    // = Runtime (Clone for Resident)
    type Error   = HttpError;
    const KEY: ResourceKey = resource_key!("http.client");

    async fn create(&self, config: &HttpConfig, _ctx: &dyn Ctx) -> Result<reqwest::Client, HttpError> {
        reqwest::Client::builder()
            .timeout(config.timeout)
            .pool_max_idle_per_host(config.max_idle)
            .build()
            .map_err(HttpError::Build)
    }
}

impl Resident for HttpClient {
    // Все defaults. reqwest::Client stateless — is_alive всегда true, stale_after None.
}
```

С health check (Redis):

```rust
pub struct RedisShared;

impl Resource for RedisShared {
    type Config  = RedisResourceConfig;
    type Runtime = fred::Client;
    type Lease   = fred::Client;       // = Runtime (Clone for Resident)
    type Error   = RedisError;
    const KEY: ResourceKey = resource_key!("redis.shared");

    async fn create(&self, config: &RedisResourceConfig, ctx: &dyn Ctx) -> Result<fred::Client, RedisError> {
        let cred = todo!("credential integration — see deferred design");
        let client = fred::Client::new(/* from cred.host, cred.port, cred.password */);
        client.init().await?;
        Ok(client)
    }

    async fn check(&self, client: &fred::Client) -> Result<(), RedisError> {
        let _: String = client.ping(None).await?;
        Ok(())
    }
}

impl Resident for RedisShared {
    fn is_alive(&self, client: &fred::Client) -> bool {
        client.is_connected()
    }

    fn stale_after(&self) -> Option<Duration> {
        Some(Duration::from_secs(15)) // проверять каждые 15 секунд
    }
}
```

**gRPC пример:**

```rust
pub struct GrpcChannel;

impl Resource for GrpcChannel {
    type Config  = GrpcConfig;
    type Runtime = tonic::transport::Channel;
    type Lease   = tonic::transport::Channel;  // Clone = Arc<Channel>
    type Error   = GrpcError;
    const KEY: ResourceKey = resource_key!("grpc.channel");

    async fn create(&self, config: &GrpcConfig, _ctx: &dyn Ctx) -> Result<tonic::transport::Channel, GrpcError> {
        let cred = todo!("credential integration — see deferred design");
        tonic::transport::Channel::from_shared(cred.endpoint.clone())?
            .connect_timeout(config.connect_timeout)
            .connect().await
            .map_err(GrpcError::Connect)
    }
}

impl Resident for GrpcChannel {
    // tonic::Channel handles reconnection internally. No stale_after needed.
}
```

### Регистрация

```rust
manager.register(RedisShared)
    .config(RedisResourceConfig::default())
    .id(resource_id)
    .resident(resident::Config { eager_create: true })
    .build().await?;
```

### Анти-паттерны

- **❌ Resident для stateful connection** (Postgres connection). Два caller-а одновременно пишут в один TCP socket → corrupt data. → Используй **Pool**.
- **❌ Resident без stale_after для network clients.** Без check — clone-ы молча мертвы после network failure. → Добавь `stale_after(Some(15s))` + `is_alive()`.
- **❌ Resident если Clone дорогой.** Clone должен быть O(1) Arc increment, не deep copy.
- **❌ Resident для dedicated (non-multiplexed) connections.** Если клиент НЕ мультиплексирует внутри (как `fred::Client` делает), каждый clone использует тот же TCP socket → contention. → Используй **Pool**.

---

## Service

### Когда

Один long-lived runtime (process, connection, polling loop) + lightweight tokens для callers. Runtime живёт долго. Callers получают token — cheap handle для взаимодействия.

### Признаки

- Runtime = процесс или long-lived connection. Не клонируемый напрямую.
- Callers нуждаются в handle для взаимодействия (send message, make request).
- Handle может быть Clone (cheap token) или tracked (semaphore permit).
- Runtime обслуживает множество callers одновременно.

### Real-world примеры

| Resource | Runtime | Token | TokenMode | Почему не другое |
|---|---|---|---|---|
| **Telegram Bot** | Bot + polling loop | TelegramBotHandle (Bot.clone() + broadcast rx) | Cloned | Runtime не Clone (polling state), но tokens cheap |
| **WebSocket (outbound)** | WsRuntime (connection + background loop) | WsHandle (mpsc::Sender clone) | Cloned | Runtime не Clone (connection state), tokens = sender clone |
| **Rate-Limited API** | HTTP client + semaphore | SemaphorePermit | Tracked | Нужен capacity control — permits limited |

### Как выглядит

```rust
pub struct TelegramBot;

impl Resource for TelegramBot {
    type Config  = TelegramResourceConfig;
    type Runtime = TelegramBotRuntime;
    type Lease   = TelegramBotHandle;   // Token (Service topology)
    type Error   = TelegramError;
    const KEY: ResourceKey = resource_key!("telegram.bot");

    async fn create(&self, config: &TelegramResourceConfig, ctx: &dyn Ctx) -> Result<TelegramBotRuntime, TelegramError> {
        let cred = todo!("credential integration — see deferred design");
        // Setup infrastructure ONLY. DO NOT start polling loop here.
        // Polling = Daemon::run(), started by framework.
        let bot = Bot::new(cred.token.expose());
        let info = bot.get_me().await.map_err(TelegramError::Api)?;
        let (update_tx, _) = broadcast::channel(config.buffer_size);
        Ok(TelegramBotRuntime {
            inner: Arc::new(BotInner { bot, info, update_tx }),
        })
    }

    async fn destroy(&self, runtime: TelegramBotRuntime) -> Result<(), TelegramError> {
        drop(runtime);
        Ok(())
    }
}

impl Service for TelegramBot {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    async fn acquire_token(&self, runtime: &TelegramBotRuntime, _ctx: &dyn Ctx)
        -> Result<TelegramBotHandle, TelegramError>
    {
        Ok(TelegramBotHandle {
            bot:       runtime.inner.bot.clone(),
            update_rx: runtime.inner.update_tx.subscribe(),
            info:      Arc::clone(&runtime.inner.info),
        })
    }
}
```

### Когда Cloned, когда Tracked

**Cloned** — Token дешёвый. Нет ограничения на количество одновременных holders. release = noop (drop). Telegram Bot, WebSocket outbound.

**Tracked** — Token = permit. Ограниченное количество. release обязателен (вернуть permit). Rate-limited API с semaphore. LeaseGuard оборачивает Token.

### Анти-паттерны

- **❌ Service когда Runtime: Clone.** Resident проще — acquire = clone, нет token machinery. → Используй **Resident**.
- **❌ Service без long-lived runtime.** Service подразумевает background process. Если runtime создаётся и уничтожается на каждый acquire — это **Pool**.
- **❌ Service для request-response без background process.** Если нет polling loop / connection loop — зачем Service? Проверь: может быть **Pool** или **Resident**.

---

## Transport

### Когда

Одно соединение (transport) + N мультиплексированных сессий поверх. Создание transport дорогое. Сессии дешёвые. Callers получают session, transport shared.

### Признаки

- Один TCP/TLS connection, множество logical channels.
- SSH: одно TCP connection, множество spawned processes.
- AMQP: одно TCP connection, множество channels.
- Session дешевле чем transport (ms vs seconds).

### Real-world примеры

| Resource | Transport (одно) | Session (много) | Transport cost | Session cost |
|---|---|---|---|---|
| **SSH** | TCP connection + auth handshake | Spawned child process | ~2s (TCP + key exchange + auth) | ~10ms |
| **AMQP (RabbitMQ)** | TCP connection + SASL auth | Channel | ~500ms | ~1ms |
| **HTTP/2 (raw)** | TLS connection | Stream | ~200ms | ~0ms |

### Как выглядит

```rust
pub struct Ssh;

impl Resource for Ssh {
    type Config  = SshResourceConfig;
    type Runtime = SshRuntime;          // one TCP connection
    type Lease   = SshSession;          // Session (Transport topology)
    type Error   = SshError;
    const KEY: ResourceKey = resource_key!("ssh");

    async fn create(&self, config: &SshResourceConfig, ctx: &dyn Ctx) -> Result<SshRuntime, SshError> {
        let cred = todo!("credential integration — see deferred design");
        let session = openssh::SessionBuilder::default()
            .host(&cred.host).port(cred.port).user(&cred.username)
            .connect_timeout(config.connect_timeout)
            .connect().await?;
        Ok(SshRuntime { session })
    }
}

impl Transport for Ssh {
    async fn open_session(&self, transport: &SshRuntime, _ctx: &dyn Ctx) -> Result<SshSession, SshError> {
        let child = transport.session.command("bash").spawn().await?;
        Ok(SshSession { child, opened_at: Instant::now() })
    }

    async fn close_session(&self, _: &SshRuntime, session: SshSession, _healthy: bool) -> Result<(), SshError> {
        drop(session.child);
        Ok(())
    }

    async fn keepalive(&self, transport: &SshRuntime) -> Result<(), SshError> {
        transport.session.check().await.map_err(SshError::Keepalive)
    }
}
```

### Action использует session, не transport

```rust
let ssh = ctx.resource::<Ssh>().await?;
// ssh: ResourceHandle<Ssh> — deref к SshSession (session, не transport).
// Framework: open_session() уже вызван. Caller видит session.
let output = ssh.exec("ls -la").await?;
// drop(ssh) → close_session()
```

### Анти-паттерны

- **❌ Transport когда каждый "session" = отдельный TCP connect.** Это **Pool**, не Transport. Transport = ОДНО connection + N sessions.
- **❌ Transport без keepalive().** Transport connection idle может быть closed сервером. Keepalive = periodic probe. SSH: `sshd` drops after `ClientAliveInterval`.
- **❌ Transport для HTTP/2 через tonic/reqwest.** Эти клиенты уже мультиплексируют внутри. → Используй **Resident** (tonic::Channel Clone = Arc).

---

## Exclusive

### Когда

Один владелец в момент времени. Нельзя шарить. Mutex semantics. Следующий caller ждёт пока предыдущий отпустит.

### Признаки

- Runtime хранит mutable state (consumer offsets, file position, port lock).
- Concurrent access = data corruption.
- Не Clone (или Clone бессмыслен).
- Reset между владельцами: commit offsets, flush buffers, release lock.

### Real-world примеры

| Resource | Почему Exclusive | reset() | Что будет без Exclusive |
|---|---|---|---|
| **Kafka Consumer** | Consumer group offsets, partition assignment | commit offsets | Два consumer-а = rebalance storm, duplicate processing |
| **Serial Port** | Physical device, one writer | flush buffers | Interleaved bytes on wire |
| **File Lock** | flock(), один процесс | release lock | Lock contention, deadlock |
| **Hardware device** | GPIO pin, один controller | reset state | Conflicting signals |

### Как выглядит

```rust
pub struct KafkaConsumer;

impl Resource for KafkaConsumer {
    type Config  = KafkaConsumerResourceConfig;
    type Runtime = StreamConsumer;
    type Lease   = StreamConsumer;     // = Runtime (Exclusive topology)
    type Error   = KafkaError;
    const KEY: ResourceKey = resource_key!("kafka.consumer");

    async fn create(&self, config: &KafkaConsumerResourceConfig, ctx: &dyn Ctx) -> Result<StreamConsumer, KafkaError> {
        let cred = todo!("credential integration — see deferred design");
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &cred.brokers)
            .set("group.id", &config.group_id)
            .create()?;
        consumer.subscribe(&config.topics)?;
        Ok(consumer)
    }
}

impl Exclusive for KafkaConsumer {
    async fn reset(&self, consumer: &StreamConsumer) -> Result<(), KafkaError> {
        consumer.commit_consumer_state(CommitMode::Sync)?;
        Ok(())
    }
}
```

### Анти-паттерны

- **❌ Exclusive когда instances независимы.** Exclusive = bottleneck (один caller за раз). Если instances взаимозаменяемы → **Pool** даёт параллелизм.
- **❌ Exclusive с Pool вместо.** Pool с max_size=1 ≈ Exclusive но с лишним overhead. → Используй **Exclusive** напрямую.
- **❌ Exclusive без reset().** Без reset — следующий владелец наследует state предыдущего. Kafka: uncommitted offsets → duplicate processing.

---

## EventSource

### Когда

Входящий поток событий. Runtime подписан на канал/topic. Engine вызывает recv() для получения следующего event. **EventSource — secondary capability, не primary topology.** Всегда комбинируется с другой primary topology.

### Признаки

- Данные приходят извне, ресурс слушает.
- Подписка (channels, topics, patterns) задана в config при регистрации.
- recv() блокирует до получения event.
- Callers не отправляют — только получают.

### Real-world примеры

| Resource | Primary topology | Что слушает | Event type |
|---|---|---|---|
| **Redis Pub/Sub** | Resident | Channels, patterns | PubSubMessage { channel, payload } |
| **Telegram Bot (inbound)** | Service | Polling loop → updates | TelegramUpdate { kind, chat_id, text } |
| **WebSocket (inbound)** | Service | WS connection → messages | WsMessage { payload } |

### Как выглядит

```rust
pub struct RedisSubscriber;

impl Resource for RedisSubscriber {
    type Config  = RedisSubscriberConfig;
    type Runtime = RedisPubSubRuntime;
    type Lease   = RedisPubSubRuntime; // = Runtime (EventSource — no direct acquire)
    type Error   = RedisError;
    const KEY: ResourceKey = resource_key!("redis.subscriber");

    async fn create(&self, config: &RedisSubscriberConfig, ctx: &dyn Ctx) -> Result<RedisPubSubRuntime, RedisError> {
        let cred = todo!("credential integration — see deferred design");
        let subscriber = fred::SubscriberClient::new(/* from cred */);
        subscriber.init().await?;
        for ch in &config.channels {
            subscriber.subscribe(ch).await?;
        }
        let (tx, _) = broadcast::channel(config.buffer_size);
        // ... spawn forward task ...
        Ok(RedisPubSubRuntime { subscriber, message_tx: tx })
    }
}

impl EventSource for RedisSubscriber {
    type Event = PubSubMessage;
    type Subscription = broadcast::Receiver<PubSubMessage>;

    async fn subscribe(
        &self,
        runtime: &RedisPubSubRuntime,
        _ctx: &dyn Ctx,
    ) -> Result<broadcast::Receiver<PubSubMessage>, RedisError> {
        Ok(runtime.message_tx.subscribe())
    }

    async fn recv(
        &self,
        subscription: &mut broadcast::Receiver<PubSubMessage>,
    ) -> Result<PubSubMessage, RedisError> {
        subscription.recv().await.map_err(|_| RedisError::SubscriptionClosed)
    }
}
```

### EventSource + EventTrigger

EventSource — resource level. EventTrigger — action level. Trigger использует EventSource:

```rust
struct OrderEventTrigger;

impl EventTrigger for OrderEventTrigger {
    type Source = RedisSubscriber;
    type Event  = OrderEvent;

    async fn on_event(&self, sub: &mut broadcast::Receiver<PubSubMessage>, _ctx: &TriggerContext)
        -> Result<Option<OrderEvent>>
    {
        let msg = sub.recv().await.map_err(|_| RedisError::SubscriptionClosed)?;
        if msg.channel.starts_with("orders.") {
            Ok(Some(serde_json::from_str(&msg.payload)?))
        } else {
            Ok(None) // skip
        }
    }
}
```

### Анти-паттерны

- **❌ EventSource как primary topology.** EventSource дополняет другую topology. Всегда нужна primary (Resident, Service, etc.).
- **❌ EventSource для request-response.** Если caller отправляет запрос и ждёт ответ — это не event stream. → **Service** или **Pool**.
- **❌ Путать EventSource с Resident.** Resident: callers отправляют requests. EventSource: callers получают events. Гибрид → Service + EventSource.

---

## Daemon

### Когда

Фоновый процесс. Нет acquire/release. Framework только стартует, мониторит, рестартует при crash. Callers не взаимодействуют напрямую. **Daemon — secondary capability.** Обычно комбинируется с Service или EventSource.

### Признаки

- Процесс бежит непрерывно с момента старта до shutdown.
- Нет API для callers (нет send, recv, query).
- Framework управляет lifecycle: start, stop, restart on crash.
- Побочные эффекты через другие каналы (EventBus, database writes, metrics).

### Real-world примеры

| Resource | Что делает | Primary topology | Почему Daemon secondary |
|---|---|---|---|
| **Telegram dispatcher** | Polling loop → broadcast updates | Service | Daemon manages polling lifecycle |
| **WebSocket connection** | Maintain connection + reconnect | Service | Daemon manages reconnect loop |
| **Cron scheduler** | Periodic task execution | — (standalone) | Нет acquire, pure background |
| **Metrics collector** | Periodic scrape → push to storage | — (standalone) | Нет acquire, pure background |

### Как выглядит

```rust
impl Daemon for TelegramBot {
    async fn run(&self, runtime: &TelegramBotRuntime, _ctx: &dyn Ctx, cancel: CancellationToken)
        -> Result<(), TelegramError>
    {
        tokio::select! {
            _ = cancel.cancelled() => Ok(()),
            result = runtime.poll_loop() => result,
        }
    }
}
```

### Daemon + другие topology

Daemon часто комбинируется. Telegram Bot:
- **Service** — outbound (send messages через TelegramBotHandle token).
- **EventSource** — inbound (recv updates).
- **Daemon** — lifecycle (polling loop runs, restart on crash).

Один Resource struct, три topology impl:

```rust
impl Service for TelegramBot { ... }
impl EventSource for TelegramBot { ... }
impl Daemon for TelegramBot { ... }
```

### Анти-паттерны

- **❌ Daemon если callers нуждаются в API.** Если есть `send_message()`, `query()`, `exec()` — это **Service** или **Pool**, не Daemon.
- **❌ Daemon как замена background task внутри другой topology.** Pool maintenance loop — внутренняя деталь Pool, не Daemon. Daemon — отдельный standalone или secondary capability.
- **❌ Daemon без CancellationToken check.** `run()` ОБЯЗАН проверять `cancel`. Без проверки — framework не может остановить daemon gracefully.

---

## Гибриды

Некоторые ресурсы реализуют несколько topology одновременно:

| Resource | Primary | Secondary | Почему |
|---|---|---|---|
| **Telegram Bot** | Service | EventSource + Daemon | Отправка (Service) + приём (EventSource) + polling lifecycle (Daemon) |
| **WebSocket** | Service | EventSource + Daemon | Отправка (Service token = WsHandle) + приём (EventSource recv) + connection loop (Daemon) |
| **Redis Shared + Subscriber** | Resident | EventSource | Shared client (Resident) + pub/sub messages (EventSource) |
| **Kafka Producer + Consumer** | — | — | Два отдельных Resource struct-а: Resident (producer) + Exclusive (consumer) |

Гибриды на одном Resource struct регистрируются через secondary topologies:

```rust
// Telegram Bot: Service (primary) + EventSource + Daemon (secondary)
manager.register(TelegramBot)
    .config(tg_config)
    .id(tg_id)
    .service(service::Config::default())               // primary topology
    .also_event_source(event_source::Config::default()) // secondary
    .also_daemon(daemon::Config::default())             // secondary
    .build().await?;
```

Для гибридов на разных Resource struct-ах (Redis Shared + Subscriber, Kafka Producer + Consumer) — регистрируйте как отдельные ресурсы. ResourceGroup для группировки — v2 (deferred).

---

## Сводная таблица

| Topology | Runtime Clone? | Callers одновременно | Acquire cost | State между callers | Use case |
|---|---|---|---|---|---|
| **Pool** | нет | N (до max_size) | Checkout/create | Изолирован (recycle) | DB connections, browser pages, SMTP |
| **Resident** | да | Unlimited | Clone (~0) | Shared | HTTP clients, multiplexed clients, loggers, metrics |
| **Service** | нет (runtime), да (token) | Unlimited (Cloned) или limited (Tracked) | Token create | Нет per-caller state | Bots, managed APIs, WebSocket |
| **Transport** | нет | N sessions | Open session | Per-session | SSH, AMQP |
| **Exclusive** | нет | 1 | Lock acquire | Reset between | Kafka consumer, serial port |
| **EventSource** | — (secondary) | N listeners | Subscribe | Нет | Pub/Sub, incoming streams |
| **Daemon** | — (secondary) | 0 (no acquire) | — | — | Background processes, polling loops |
