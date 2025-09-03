---
title: Quick Start
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---


# Quick Start Guide

> Создайте и используйте ресурсы за 5 минут

## 1️⃣ Basic Usage

### Initialize Resource Manager

```rust
use nebula_resource::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Создаём менеджер ресурсов
    let manager = ResourceManager::builder()
        .with_default_resources()    // HTTP, Logger, Metrics
        .with_health_checks(true)     // Автоматические health checks
        .build()
        .await?;
    
    println!("✅ Resource Manager initialized");
    Ok(())
}
```

### Use Built-in Resources

```rust
// Получаем HTTP клиент
let http = manager.get::<HttpClient>("default").await?;

// Делаем запрос
let response = http
    .get("https://api.github.com/users/rust-lang")
    .header("User-Agent", "nebula-resource")
    .send()
    .await?;

let data: serde_json::Value = response.json().await?;
println!("GitHub user: {}", data["name"]);

// Получаем логгер
let logger = manager.get::<Logger>("default").await?;
logger.info("Successfully fetched GitHub user data");
```

## 2️⃣ Database Connection

### Configure PostgreSQL Pool

```rust
use nebula_resource::prelude::*;
use nebula_resource::resources::database::*;

// Создаём конфигурацию
let pg_config = PgPoolConfig {
    url: "postgresql://user:pass@localhost/mydb".into(),
    max_connections: 10,
    min_connections: 2,
    connection_timeout: Duration::from_secs(30),
    idle_timeout: Some(Duration::from_secs(600)),
    max_lifetime: Some(Duration::from_secs(1800)),
};

// Регистрируем ресурс
manager.register(
    "main_db",
    PgPoolResource::new(pg_config)
).await?;

// Используем в коде
let db = manager.get::<PgPool>("main_db").await?;

// Выполняем запрос
let rows = sqlx::query!("SELECT id, name FROM users WHERE active = true")
    .fetch_all(&*db)
    .await?;

for row in rows {
    println!("User: {} - {}", row.id, row.name);
}
```

### Connection Pool Monitoring

```rust
// Получаем статистику пула
let stats = db.pool_stats();
println!("Active connections: {}", stats.active);
println!("Idle connections: {}", stats.idle);
println!("Total connections: {}", stats.total);
println!("Wait queue: {}", stats.waiting);
```

## 3️⃣ Cache Resource

### Redis Cache Setup

```rust
use nebula_resource::resources::cache::*;

// Конфигурация Redis
let cache_config = RedisCacheConfig {
    url: "redis://localhost:6379".into(),
    default_ttl: Duration::from_secs(300),
    max_connections: 10,
    connection_timeout: Duration::from_secs(5),
    command_timeout: Duration::from_secs(1),
};

// Регистрируем кэш
manager.register(
    "cache",
    RedisCacheResource::new(cache_config)
).await?;

// Используем кэш
let cache = manager.get::<RedisCache>("cache").await?;

// Сохраняем данные
#[derive(Serialize, Deserialize)]
struct User {
    id: i32,
    name: String,
    email: String,
}

let user = User {
    id: 1,
    name: "Alice".into(),
    email: "alice@example.com".into(),
};

// Кэшируем с TTL
cache.set("user:1", &user, Duration::from_secs(600)).await?;

// Читаем из кэша
let cached_user: Option<User> = cache.get("user:1").await?;

match cached_user {
    Some(user) => println!("Cached user: {}", user.name),
    None => println!("User not in cache"),
}
```

### Cache Patterns

```rust
// Cache-aside pattern
async fn get_user(id: i32, db: &PgPool, cache: &RedisCache) -> Result<User> {
    let key = format!("user:{}", id);
    
    // Проверяем кэш
    if let Some(user) = cache.get::<User>(&key).await? {
        return Ok(user);
    }
    
    // Загружаем из БД
    let user = sqlx::query_as!(User, 
        "SELECT id, name, email FROM users WHERE id = $1", 
        id
    )
    .fetch_one(db)
    .await?;
    
    // Сохраняем в кэш
    cache.set(&key, &user, Duration::from_secs(300)).await?;
    
    Ok(user)
}
```

## 4️⃣ Context-Aware Resources

### Logger with Automatic Context

```rust
use nebula_resource::resources::observability::*;

// Создаём контекстный логгер
let logger_config = ContextLoggerConfig {
    level: LogLevel::Info,
    format: LogFormat::Json,
    include_context: true,
    include_trace: true,
};

manager.register(
    "logger",
    ContextLoggerResource::new(logger_config)
).await?;

// В action или workflow
async fn process_order(ctx: &ExecutionContext) -> Result<()> {
    let logger = ctx.resource::<ContextLogger>("logger").await?;
    
    // Автоматически включает workflow_id, execution_id, action_id
    logger.info("Processing order started");
    
    // Добавляем дополнительный контекст
    logger.with_fields(vec![
        ("order_id", "12345"),
        ("customer_id", "67890"),
    ]).info("Order validation completed");
    
    // Структурированное логирование
    logger.info_with_data("Order processed", json!({
        "items": 5,
        "total": 99.99,
        "currency": "USD"
    }));
    
    Ok(())
}
```

### Metrics Collector

```rust
// Метрики с автоматическим контекстом
let metrics_config = MetricsConfig {
    namespace: "nebula".into(),
    default_tags: HashMap::from([
        ("environment".into(), "production".into()),
        ("region".into(), "us-east-1".into()),
    ]),
    flush_interval: Duration::from_secs(60),
};

manager.register(
    "metrics",
    MetricsCollectorResource::new(metrics_config)
).await?;

// Использование
let metrics = manager.get::<MetricsCollector>("metrics").await?;

// Счётчики
metrics.increment("api.requests", 1.0);

// Гистограммы
let start = Instant::now();
// ... выполнение операции ...
metrics.histogram("api.latency", start.elapsed().as_millis() as f64);

// Gauges
metrics.gauge("queue.size", 42.0);

// С дополнительными тегами
metrics.increment_with_tags(
    "payment.processed",
    1.0,
    vec![
        ("payment_method", "credit_card"),
        ("currency", "USD"),
    ]
);
```

## 5️⃣ Message Queue

### Kafka Producer/Consumer

```rust
use nebula_resource::resources::queue::*;

// Kafka producer
let producer_config = KafkaProducerConfig {
    brokers: vec!["localhost:9092".into()],
    topic: "events".into(),
    compression: CompressionType::Snappy,
    batch_size: 1000,
    linger_ms: 100,
};

manager.register(
    "kafka_producer",
    KafkaProducerResource::new(producer_config)
).await?;

// Kafka consumer
let consumer_config = KafkaConsumerConfig {
    brokers: vec!["localhost:9092".into()],
    topics: vec!["events".into()],
    group_id: "nebula-consumer".into(),
    auto_offset_reset: "earliest".into(),
};

manager.register(
    "kafka_consumer",
    KafkaConsumerResource::new(consumer_config)
).await?;

// Отправка сообщений
let producer = manager.get::<KafkaProducer>("kafka_producer").await?;

#[derive(Serialize)]
struct Event {
    id: String,
    event_type: String,
    timestamp: i64,
    data: serde_json::Value,
}

let event = Event {
    id: "evt_123".into(),
    event_type: "user.created".into(),
    timestamp: Utc::now().timestamp(),
    data: json!({
        "user_id": "usr_456",
        "email": "user@example.com"
    }),
};

producer.send(&event).await?;

// Получение сообщений
let consumer = manager.get::<KafkaConsumer>("kafka_consumer").await?;

tokio::spawn(async move {
    while let Some(message) = consumer.next().await {
        match message {
            Ok(msg) => {
                let event: Event = msg.deserialize()?;
                println!("Received event: {} - {}", event.id, event.event_type);
                msg.commit().await?;
            }
            Err(e) => eprintln!("Error consuming message: {}", e),
        }
    }
});
```

## 6️⃣ Error Handling

### Circuit Breaker Pattern

```rust
use nebula_resource::resilience::*;

// Конфигурация circuit breaker
let cb_config = CircuitBreakerConfig {
    failure_threshold: 5,
    failure_rate_threshold: 0.5,
    timeout: Duration::from_secs(60),
    minimum_requests: 10,
    half_open_max_calls: 3,
};

// HTTP клиент с circuit breaker
let http_config = HttpClientConfig {
    base_url: Some("https://api.example.com".into()),
    timeout: Duration::from_secs(10),
    circuit_breaker: Some(cb_config),
    retry_policy: Some(RetryPolicy {
        max_attempts: 3,
        backoff: BackoffStrategy::Exponential {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(10),
            multiplier: 2.0,
        },
    }),
};

manager.register(
    "protected_api",
    HttpClientResource::new(http_config)
).await?;

// Использование с автоматической защитой
let http = manager.get::<HttpClient>("protected_api").await?;

match http.get("/endpoint").send().await {
    Ok(response) => {
        println!("Success: {:?}", response.status());
    }
    Err(e) if e.is_circuit_open() => {
        println!("Circuit breaker is open, using fallback");
        // Использовать fallback логику
    }
    Err(e) => {
        println!("Request failed: {}", e);
    }
}
```

## 7️⃣ Resource in Actions

### Define Action with Resources

```rust
use nebula_action::prelude::*;
use nebula_resource::prelude::*;

#[derive(Action)]
#[action(id = "send_notification")]
#[resources([HttpClient, Logger, MetricsCollector])]
pub struct SendNotificationAction {
    recipient: String,
    message: String,
}

#[async_trait]
impl ActionHandler for SendNotificationAction {
    type Output = NotificationResult;
    
    async fn execute(&self, ctx: &ExecutionContext) -> Result<Self::Output> {
        // Ресурсы автоматически доступны
        let http = ctx.resource::<HttpClient>("default").await?;
        let logger = ctx.resource::<Logger>("default").await?;
        let metrics = ctx.resource::<MetricsCollector>("default").await?;
        
        logger.info(&format!("Sending notification to {}", self.recipient));
        
        let start = Instant::now();
        
        let response = http
            .post("/notifications")
            .json(&json!({
                "to": self.recipient,
                "message": self.message
            }))
            .send()
            .await?;
        
        let duration = start.elapsed();
        metrics.histogram("notification.send_time", duration.as_millis() as f64);
        
        if response.status().is_success() {
            metrics.increment("notification.success", 1.0);
            logger.info("Notification sent successfully");
            
            Ok(NotificationResult {
                success: true,
                message_id: response.json::<Value>().await?["id"].as_str().unwrap().into(),
            })
        } else {
            metrics.increment("notification.failure", 1.0);
            logger.error(&format!("Failed to send notification: {}", response.status()));
            
            Ok(NotificationResult {
                success: false,
                message_id: String::new(),
            })
        }
    }
}
```

## 8️⃣ Testing Resources

### Mock Resources

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::testing::*;
    
    #[tokio::test]
    async fn test_notification_action() {
        // Создаём тестовый контекст
        let mut context = TestContext::new();
        
        // Создаём мок HTTP клиента
        let mut mock_http = MockHttpClient::new();
        mock_http
            .expect_post("/notifications")
            .with_json(json!({
                "to": "user@example.com",
                "message": "Test message"
            }))
            .returning(|| {
                Ok(MockResponse::json(json!({
                    "id": "msg_123",
                    "status": "sent"
                })))
            });
        
        // Регистрируем мок
        context.register_mock("default", mock_http);
        
        // Тестируем action
        let action = SendNotificationAction {
            recipient: "user@example.com".into(),
            message: "Test message".into(),
        };
        
        let result = action.execute(&context).await.unwrap();
        
        assert!(result.success);
        assert_eq!(result.message_id, "msg_123");
    }
}
```

## 🎯 Complete Example

### Multi-Resource Workflow

```rust
use nebula_resource::prelude::*;
use nebula_workflow::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize resource manager
    let manager = ResourceManager::builder()
        .with_config_dir("./configs/resources")
        .with_credential_provider(credential_provider)
        .build()
        .await?;
    
    // Register all required resources
    manager.register_from_config("database.toml").await?;
    manager.register_from_config("cache.yaml").await?;
    manager.register_from_config("http.json").await?;
    
    // Create workflow
    let workflow = Workflow::builder()
        .with_resource_manager(manager)
        .add_action(FetchUserAction { user_id: 123 })
        .add_action(EnrichUserDataAction {})
        .add_action(SendNotificationAction {
            recipient: "user@example.com".into(),
            message: "Welcome!".into(),
        })
        .build();
    
    // Execute workflow
    let result = workflow.execute().await?;
    
    println!("Workflow completed: {:?}", result);
    
    // Cleanup
    manager.shutdown().await?;
    
    Ok(())
}
```

## Next Steps

- [[BasicConcepts|Basic Concepts]] - Основные концепции системы
- [[FirstResource|Create First Resource]] - Создание кастомного ресурса
- [[How-To/CreateResource|Advanced Resource Creation]] - Продвинутые техники
- [[Examples/|More Examples]] - Больше примеров использования

## Tips & Tricks

1. **Всегда используйте builder pattern** для конфигурации
2. **Включайте health checks** для production
3. **Используйте circuit breakers** для внешних сервисов
4. **Логируйте важные операции** с контекстом
5. **Мониторьте метрики** использования ресурсов
6. **Тестируйте с моками** для изоляции
7. **Настройте graceful shutdown** для корректного завершения

## Common Patterns

### Lazy Initialization

```rust
// Ресурсы создаются только при первом использовании
let db = manager.get_or_create::<PgPool>("main_db").await?;
```

### Resource Sharing

```rust
// Ресурсы автоматически переиспользуются между actions
let http = Arc::clone(&manager.get::<HttpClient>("api").await?);
```

### Fallback Resources

```rust
// Fallback при недоступности основного ресурса
let cache = manager
    .get::<RedisCache>("primary")
    .or_else(|_| manager.get::<MemoryCache>("fallback"))
    .await?;
```

### Resource Composition

```rust
// Комбинирование ресурсов
struct DataService {
    db: Arc<PgPool>,
    cache: Arc<RedisCache>,
    logger: Arc<Logger>,
}

impl DataService {
    async fn get_data(&self, id: i32) -> Result<Data> {
        // Check cache first
        if let Some(data) = self.cache.get(&format!("data:{}", id)).await? {
            self.logger.debug("Cache hit");
            return Ok(data);
        }
        
        // Load from database
        self.logger.debug("Cache miss, loading from DB");
        let data = self.db.query("SELECT * FROM data WHERE id = $1", &[&id]).await?;
        
        // Update cache
        self.cache.set(&format!("data:{}", id), &data, Duration::from_secs(300)).await?;
        
        Ok(data)
    }
}
```