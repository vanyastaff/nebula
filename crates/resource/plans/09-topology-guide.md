# Выбор topology для ресурса

## Зачем topology

Внешний ресурс — база данных, API, бот, browser — имеет определённую **природу доступа**. Postgres connection stateful, одноразовый per-query. HTTP client stateless, shared. SSH — одно соединение, много сессий. Kafka consumer — один владелец, нельзя шарить.

Topology кодифицирует эту природу. Framework знает как управлять lifecycle: когда создавать, сколько держать, как переиспользовать, что делать при сбое. Resource author выбирает topology один раз — framework делает всё остальное.

Неправильный выбор topology = либо waste ресурсов (Pool для stateless client), либо race conditions (Resident для stateful connection), либо deadlock (не Exclusive для single-owner resource).

---

## Дерево решений

```
Ресурс нуждается в acquire/release?
│
├─ НЕТ → фоновый процесс, просто бежит
│        → Daemon
│
└─ ДА
   │
   ├─ Runtime клонируемый (Clone) И stateless?
   │  │
   │  ├─ ДА → один shared instance, callers получают clone
   │  │       → Resident
   │  │
   │  └─ НЕТ
   │     │
   │     ├─ Одно соединение, много мультиплексированных сессий?
   │     │  → Transport
   │     │
   │     ├─ Только один владелец в момент времени?
   │     │  → Exclusive
   │     │
   │     ├─ Long-lived процесс + lightweight tokens для callers?
   │     │  → Service
   │     │
   │     ├─ Входящий поток событий (subscribe/recv)?
   │     │  → EventSource
   │     │
   │     └─ N взаимозаменяемых stateful instances?
   │        → Pool
   │
   └─ Гибрид? (outbound API + incoming events)
      → Service + EventSource на одном Resource struct
```

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

### Примеры

| Resource | Почему Pool | Create cost | Recycle |
|---|---|---|---|
| PostgreSQL | TCP connection, per-connection state (search_path, prepared stmts) | ~50ms (TCP + auth) | DISCARD ALL ~1ms |
| MySQL | TCP connection, per-connection variables | ~30ms | RESET CONNECTION |
| Redis Dedicated | TCP, per-connection SELECT db, WATCH state | ~10ms | UNWATCH + SELECT 0 |
| Headless Browser Page | Chrome DevTools page in shared browser process | ~200ms | Clear cookies/storage ~500ms |
| LDAP Connection | TCP + BIND | ~100ms | Unbind + rebind |

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

- **Не делать Pool для stateless client** (reqwest::Client). Клон дешевле pool checkout. Используй Resident.
- **Не делать Pool с max_size=1**. Это Exclusive с overhead pool machinery. Используй Exclusive.
- **Не забывать recycle()**. Без recycle — state протекает между callers. Cross-tenant data leak.
- **Не забывать prepare()**. Без prepare — tenant isolation manual, caller может забыть.

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

### Примеры

| Resource | Почему Resident | Что внутри Clone |
|---|---|---|
| reqwest::Client | HTTP/2 multiplexing, internal connection pool | Arc<ClientInner> |
| fred::Client (Redis) | Multiplexed pipelining, internal reconnection | Arc<ClientInner> |
| rdkafka::FutureProducer | Internal buffer, background thread sends batches | Arc<ProducerInner> |
| tonic::Channel (gRPC) | HTTP/2 multiplexed, internal load balancing | Arc<Channel> |
| LLM API client | HTTP client + rate limiter + usage tracker | Arc<...> |

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
        // Credential access — design deferred.
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

### Регистрация

```rust
manager.register(RedisShared)
    .config(RedisResourceConfig::default())
    .id(resource_id)
    .resident(resident::Config { eager_create: true })
    .build().await?;
```

### Анти-паттерны

- **Не делать Resident для stateful connection** (Postgres connection). Два caller-а одновременно пишут в один TCP socket → corrupt data.
- **Не забывать stale_after для network clients**. Без check — clone-ы молча мертвы после network failure. fred.rs: broker disconnect → все clone-ы broken → is_alive() catches.
- **Не делать Resident если Clone дорогой.** Clone должен быть O(1) Arc increment, не deep copy.

---

## Service

### Когда

Один long-lived runtime (process, connection, polling loop) + lightweight tokens для callers. Runtime живёт долго. Callers получают token — cheap handle для взаимодействия.

### Признаки

- Runtime = процесс или long-lived connection. Не клонируемый напрямую.
- Callers нуждаются в handle для взаимодействия (send message, make request).
- Handle может быть Clone (cheap token) или tracked (semaphore permit).
- Runtime обслуживает множество callers одновременно.

### Примеры

| Resource | Runtime | Token | TokenMode |
|---|---|---|---|
| Telegram Bot | Bot + polling loop | TelegramBotHandle (Bot.clone() + broadcast rx) | Cloned |
| WebSocket (outbound) | WsRuntime (connection + background loop) | WsHandle (mpsc::Sender clone) | Cloned |
| Rate-Limited API | HTTP client + semaphore | SemaphorePermit | Tracked |
| gRPC Service | Server connection + stream manager | RequestHandle | Tracked |

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
        // Credential access — design deferred.
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
        // Framework cancels Daemon separately via CancellationToken.
        drop(runtime);
        Ok(())
    }
}

impl Service for TelegramBot {
    // Lease = TelegramBotHandle (defined in Resource trait above).
    // No separate `type Token` — Service uses Self::Lease.
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

- **Не путать Service с Resident.** Resident: `Runtime: Clone`, acquire = clone runtime. Service: Runtime не Clone, acquire = create token. Если runtime Clone — используй Resident (проще).
- **Не делать Service если нет long-lived runtime.** Service подразумевает background process (polling, connection). Если runtime создаётся и уничтожается на каждый acquire — это Pool.

---

## Transport

### Когда

Одно соединение (transport) + N мультиплексированных сессий поверх. Создание transport дорогое. Сессии дешёвые. Callers получают session, transport shared.

### Признаки

- Один TCP/TLS connection, множество logical channels.
- SSH: одно TCP connection, множество spawned processes.
- AMQP: одно TCP connection, множество channels.
- Session дешевле чем transport (ms vs seconds).

### Примеры

| Resource | Transport (одно) | Session (много) |
|---|---|---|
| SSH | TCP connection + auth handshake (~2s) | Spawned child process (~10ms) |
| AMQP (RabbitMQ) | TCP connection + SASL auth | Channel (~1ms) |
| HTTP/2 (raw) | TLS connection | Stream |

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
        // Credential access — design deferred.
        let cred = todo!("credential integration — see deferred design");
        let session = openssh::SessionBuilder::default()
            .host(&cred.host).port(cred.port).user(&cred.username)
            .connect_timeout(config.connect_timeout)
            .connect().await?;
        Ok(SshRuntime { session })
    }
}

impl Transport for Ssh {
    // Lease = SshSession (defined in Resource trait above).
    // No separate `type Session` — Transport uses Self::Lease.

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

- **Не путать Transport с Pool.** Pool: N независимых connections. Transport: ОДНО connection + N sessions. Если каждый "session" = отдельный TCP connect — это Pool, не Transport.
- **Не забывать keepalive().** Transport connection idle может быть closed сервером. Keepalive = periodic probe.

---

## Exclusive

### Когда

Один владелец в момент времени. Нельзя шарить. Mutex semantics. Следующий caller ждёт пока предыдущий отпустит.

### Признаки

- Runtime хранит mutable state (consumer offsets, file position, port lock).
- Concurrent access = data corruption.
- Не Clone (или Clone бессмыслен).
- Reset между владельцами: commit offsets, flush buffers, release lock.

### Примеры

| Resource | Почему Exclusive | reset() |
|---|---|---|
| Kafka Consumer | Consumer group offsets, partition assignment. Два consumer-а = rebalance storm | commit offsets |
| Serial Port | Physical device, one writer | flush buffers |
| File Lock | flock(), один процесс | release lock |
| Hardware device | GPIO pin, один controller | reset state |

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
        // Credential access — design deferred.
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

- **Не делать Exclusive если можно Pool.** Exclusive = bottleneck. Один caller за раз. Если instances независимы — Pool даёт параллелизм.
- **Не делать Exclusive с max_size > 1.** Exclusive = ровно один instance. Если нужно N exclusive instances — это Pool с per-instance locking (другая задача).
- **Не забывать reset().** Без reset — следующий владелец наследует state предыдущего. Kafka: uncommitted offsets → duplicate processing.

---

## EventSource

### Когда

Входящий поток событий. Runtime подписан на канал/topic. Callers вызывают recv() для получения следующего event.

### Признаки

- Данные приходят извне, ресурс слушает.
- Подписка (channels, topics, patterns) задана в config при регистрации.
- recv() блокирует до получения event.
- Callers не отправляют — только получают.

### Примеры

| Resource | Что слушает | Event type |
|---|---|---|
| Redis Pub/Sub | Channels, patterns | PubSubMessage { channel, payload } |
| Telegram Bot (inbound) | Polling loop → updates | TelegramUpdate { kind, chat_id, text } |
| WebSocket (inbound) | WS connection → messages | WsMessage { payload } |
| Kafka Consumer* | Topics → records | ConsumerRecord { topic, key, value } |
| NATS Subscriber | Subjects | NatsMessage { subject, payload } |

*Kafka Consumer может быть и Exclusive (если нужен offset control), и EventSource (если просто слушать).

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
        // Credential access — design deferred.
        let cred = todo!("credential integration — see deferred design");
        let subscriber = fred::SubscriberClient::new(/* from cred */);
        subscriber.init().await?;
        for ch in &config.channels {
            subscriber.subscribe(ch).await?;
        }
        // Background task: subscriber → broadcast channel
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

- **Не путать EventSource с Resident.** Resident: callers отправляют requests. EventSource: callers получают events. Если ресурс и отправляет и получает — гибрид Service + EventSource.
- **Не делать EventSource для request-response.** Если caller отправляет запрос и ждёт ответ — это не event stream, это Service или Pool.

---

## Daemon

### Когда

Фоновый процесс. Нет acquire/release. Framework только стартует, мониторит, рестартует при crash. Callers не взаимодействуют напрямую.

### Признаки

- Процесс бежит непрерывно с момента старта до shutdown.
- Нет API для callers (нет send, recv, query).
- Framework управляет lifecycle: start, stop, restart on crash.
- Побочные эффекты через другие каналы (EventBus, database writes, metrics).

### Примеры

| Resource | Что делает | Почему Daemon |
|---|---|---|
| Telegram dispatcher | Polling loop → broadcast updates | Нет direct API. Events через broadcast channel |
| Cron scheduler | Periodic task execution | Нет acquire. Запускает tasks по расписанию |
| Metrics collector | Periodic scrape → push to storage | Нет acquire. Background scrape loop |
| Log rotator | Periodic log file rotation | Нет acquire. Background maintenance |

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

- **Не делать Daemon если callers нуждаются в API.** Если есть send_message(), query(), exec() — это Service или Pool, не Daemon.
- **Daemon — не замена для background task внутри другой topology.** Pool maintenance loop — внутренняя деталь Pool, не Daemon. Daemon — отдельный standalone процесс.

---

## Гибриды

Некоторые ресурсы реализуют несколько topology одновременно:

| Resource | Topologies | Почему |
|---|---|---|
| Telegram Bot | Service + EventSource + Daemon | Отправка (Service) + приём (EventSource) + polling lifecycle (Daemon) |
| WebSocket | Service + EventSource | Отправка (Service token = WsHandle) + приём (EventSource recv) |
| Redis Shared + Subscriber | Resident (shared) + EventSource (subscriber) | Два отдельных Resource struct-а, один backend |
| Kafka Producer + Consumer | Resident (producer) + Exclusive (consumer) | Два отдельных Resource struct-а, один backend |

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
| **Pool** | нет | N (до max_size) | Checkout/create | Изолирован (recycle) | DB connections, browser pages |
| **Resident** | да | Unlimited | Clone (~0) | Shared | HTTP clients, multiplexed clients |
| **Service** | нет (runtime), да (token) | Unlimited (Cloned) или limited (Tracked) | Token create | Нет per-caller state | Bots, managed APIs |
| **Transport** | нет | N sessions | Open session | Per-session | SSH, AMQP |
| **Exclusive** | нет | 1 | Lock acquire | Reset between | Kafka consumer, serial port |
| **EventSource** | нет | N listeners | Subscribe | Нет | Pub/Sub, incoming streams |
| **Daemon** | — | 0 (no acquire) | — | — | Background processes |
