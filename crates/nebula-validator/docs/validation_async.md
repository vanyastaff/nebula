# Async Validation

> Database uniqueness checks, external API validation, and async validation patterns

## Quick Start

```rust
#[cfg(feature = "validation-async")]
use nebula_value::prelude::*;
use nebula_value::validation::{AsyncValidator, ValidationError};

// Simple async validator
async fn validate_email_unique(email: &Text, db: &Database) -> Result<(), ValidationError> {
    let exists = db.email_exists(email.as_ref()).await?;
    if exists {
        Err(ValidationError::new("email_taken", "Email already registered"))
    } else {
        Ok(())
    }
}

// Usage
let email = Text::new("user@example.com");
validate_email_unique( & email, & db).await?;
```

## AsyncValidator Trait

Unlike the synchronous `Validator` trait, `AsyncValidator` uses associated types to avoid `async-trait` allocations:

```rust
#[cfg(feature = "validation-async")]
pub trait AsyncValidator<T>: Send + Sync {
    type Future: std::future::Future<Output=Result<(), ValidationError>> + Send;

    fn validate_async(&self, value: &T) -> Self::Future;

    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
```

## Database Uniqueness Validation

### SQLx Integration

```rust
use nebula_value::prelude::*;
use sqlx::{PgPool, Row};
use std::pin::Pin;
use std::future::Future;

struct DatabaseUniqueValidator {
    pool: PgPool,
    table: &'static str,
    column: &'static str,
    error_code: &'static str,
}

impl DatabaseUniqueValidator {
    pub fn new(
        pool: PgPool,
        table: &'static str,
        column: &'static str,
        error_code: &'static str
    ) -> Self {
        Self { pool, table, column, error_code }
    }

    pub fn email(pool: PgPool) -> Self {
        Self::new(pool, "users", "email", "email_taken")
    }

    pub fn username(pool: PgPool) -> Self {
        Self::new(pool, "users", "username", "username_taken")
    }
}

impl AsyncValidator<Text> for DatabaseUniqueValidator {
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &Text) -> Self::Future {
        let query = format!("SELECT 1 FROM {} WHERE {} = $1 LIMIT 1", self.table, self.column);
        let value_str = value.as_ref().to_string();
        let pool = self.pool.clone();
        let error_code = self.error_code;

        Box::pin(async move {
            let exists = sqlx::query(&query)
                .bind(&value_str)
                .fetch_optional(&pool)
                .await
                .map_err(|e| ValidationError::new(
                    "database_error",
                    format!("Database validation failed: {}", e)
                ))?
                .is_some();

            if exists {
                Err(ValidationError::new(
                    error_code,
                    match error_code {
                        "email_taken" => "This email is already registered",
                        "username_taken" => "This username is already taken",
                        _ => "This value already exists",
                    }
                ).with_param("value", value_str))
            } else {
                Ok(())
            }
        })
    }
}

// Usage
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPool::connect("postgresql://localhost/mydb").await?;

    let email_validator = DatabaseUniqueValidator::email(pool.clone());
    let username_validator = DatabaseUniqueValidator::username(pool);

    let email = Text::new("new_user@example.com");
    let username = Text::new("new_username");

    // Validate uniqueness
    email_validator.validate_async(&email).await?;
    username_validator.validate_async(&username).await?;

    println!("Both email and username are unique!");
    Ok(())
}
```

### Diesel Integration

```rust
use nebula_value::prelude::*;
use diesel::prelude::*;
use diesel::pg::PgConnection;
use std::pin::Pin;
use std::future::Future;

struct DieselUniqueValidator {
    connection_url: String,
    table_name: &'static str,
    column_name: &'static str,
}

impl DieselUniqueValidator {
    pub fn new(connection_url: String, table: &'static str, column: &'static str) -> Self {
        Self {
            connection_url,
            table_name: table,
            column_name: column,
        }
    }
}

impl AsyncValidator<Text> for DieselUniqueValidator {
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &Text) -> Self::Future {
        let connection_url = self.connection_url.clone();
        let table = self.table_name;
        let column = self.column_name;
        let value_str = value.as_ref().to_string();

        Box::pin(async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut conn = PgConnection::establish(&connection_url)
                    .map_err(|e| ValidationError::new(
                        "connection_error",
                        format!("Database connection failed: {}", e)
                    ))?;

                // Use raw SQL for dynamic table/column names
                let query = format!("SELECT 1 FROM {} WHERE {} = $1 LIMIT 1", table, column);
                let exists = diesel::sql_query(query)
                    .bind::<diesel::sql_types::Text, _>(&value_str)
                    .get_result::<(i32,)>(&mut conn)
                    .optional()
                    .map_err(|e| ValidationError::new(
                        "query_error",
                        format!("Database query failed: {}", e)
                    ))?
                    .is_some();

                if exists {
                    Err(ValidationError::new(
                        "value_exists",
                        "This value already exists in the database"
                    ).with_param("value", value_str))
                } else {
                    Ok(())
                }
            }).await;

            match result {
                Ok(validation_result) => validation_result,
                Err(e) => Err(ValidationError::new(
                    "task_error",
                    format!("Async task failed: {}", e)
                )),
            }
        })
    }
}
```

## External API Validation

### Email Verification Service

```rust
use nebula_value::prelude::*;
use reqwest::Client;
use serde::Deserialize;
use std::pin::Pin;
use std::future::Future;

#[derive(Deserialize)]
struct EmailVerificationResponse {
    valid: bool,
    reason: Option<String>,
    disposable: bool,
}

struct EmailVerificationValidator {
    client: Client,
    api_key: String,
    allow_disposable: bool,
}

impl EmailVerificationValidator {
    pub fn new(api_key: String, allow_disposable: bool) -> Self {
        Self {
            client: Client::new(),
            api_key,
            allow_disposable,
        }
    }
}

impl AsyncValidator<Text> for EmailVerificationValidator {
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, email: &Text) -> Self::Future {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let allow_disposable = self.allow_disposable;
        let email_str = email.as_ref().to_string();

        Box::pin(async move {
            let response = client
                .get("https://api.emailverification.com/v1/verify")
                .query(&[
                    ("email", &email_str),
                    ("api_key", &api_key),
                ])
                .send()
                .await
                .map_err(|e| ValidationError::new(
                    "api_request_failed",
                    format!("Email verification API request failed: {}", e)
                ))?;

            if !response.status().is_success() {
                return Err(ValidationError::new(
                    "api_error",
                    format!("Email verification API returned status: {}", response.status())
                ));
            }

            let verification: EmailVerificationResponse = response
                .json()
                .await
                .map_err(|e| ValidationError::new(
                    "api_parse_error",
                    format!("Failed to parse API response: {}", e)
                ))?;

            if !verification.valid {
                return Err(ValidationError::new(
                    "invalid_email",
                    verification.reason.unwrap_or("Email address is invalid".to_string())
                ).with_param("email", email_str));
            }

            if !allow_disposable && verification.disposable {
                return Err(ValidationError::new(
                    "disposable_email",
                    "Disposable email addresses are not allowed"
                ).with_param("email", email_str));
            }

            Ok(())
        })
    }
}

// Usage
let email_validator = EmailVerificationValidator::new(
"your-api-key".to_string(),
false // Don't allow disposable emails
);

let email = Text::new("user@tempmail.com");
match email_validator.validate_async( & email).await {
Ok(()) => println!("Email is valid and not disposable"),
Err(error) if error.code == "disposable_email" => {
println ! ("Email is disposable: {}", error.message);
}
Err(error) => println!("Email validation failed: {}", error.message),
}
```

### Domain Validation

```rust
use nebula_value::prelude::*;
use trust_dns_resolver::{AsyncResolver, config::*};
use std::pin::Pin;
use std::future::Future;

struct DomainValidator {
    resolver: AsyncResolver,
}

impl DomainValidator {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let resolver = AsyncResolver::tokio(
            ResolverConfig::default(),
            ResolverOpts::default(),
        ).await?;

        Ok(Self { resolver })
    }
}

impl AsyncValidator<Text> for DomainValidator {
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, domain: &Text) -> Self::Future {
        let resolver = self.resolver.clone();
        let domain_str = domain.as_ref().to_string();

        Box::pin(async move {
            // Check if domain has MX records (for email validation)
            match resolver.mx_lookup(&domain_str).await {
                Ok(_) => Ok(()), // Domain has MX records
                Err(_) => {
                    // Fallback: check for A records
                    match resolver.ipv4_lookup(&domain_str).await {
                        Ok(_) => Err(ValidationError::new(
                            "no_mx_record",
                            "Domain exists but cannot receive email"
                        ).with_param("domain", domain_str)),
                        Err(_) => Err(ValidationError::new(
                            "invalid_domain",
                            "Domain does not exist"
                        ).with_param("domain", domain_str)),
                    }
                }
            }
        })
    }
}
```

## Async Validation Combinators

### Sequential Validation

```rust
use nebula_value::prelude::*;
use std::pin::Pin;
use std::future::Future;

struct SequentialAsyncValidator<T> {
    validators: Vec<Box<dyn AsyncValidator<T>>>,
}

impl<T> SequentialAsyncValidator<T>
where
    T: Send + Sync,
{
    pub fn new() -> Self {
        Self { validators: Vec::new() }
    }

    pub fn add_validator(mut self, validator: Box<dyn AsyncValidator<T>>) -> Self {
        self.validators.push(validator);
        self
    }
}

impl<T> AsyncValidator<T> for SequentialAsyncValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let validators: Vec<_> = self.validators.iter().collect();

        Box::pin(async move {
            // Run validators sequentially (stop at first error)
            for validator in validators {
                validator.validate_async(value).await?;
            }
            Ok(())
        })
    }
}

// Usage
let email = Text::new("user@example.com");

let email_validator = SequentialAsyncValidator::new()
.add_validator(Box::new(EmailVerificationValidator::new("api-key".to_string(), false)))
.add_validator(Box::new(DatabaseUniqueValidator::email(pool)));

email_validator.validate_async( & email).await?;
```

### Parallel Validation

```rust
use futures::future::join_all;

struct ParallelAsyncValidator<T> {
    validators: Vec<Box<dyn AsyncValidator<T>>>,
}

impl<T> ParallelAsyncValidator<T>
where
    T: Send + Sync,
{
    pub fn new() -> Self {
        Self { validators: Vec::new() }
    }

    pub fn add_validator(mut self, validator: Box<dyn AsyncValidator<T>>) -> Self {
        self.validators.push(validator);
        self
    }
}

impl<T> AsyncValidator<T> for ParallelAsyncValidator<T>
where
    T: Send + Sync + Clone,
{
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let value = value.clone();
        let futures: Vec<_> = self.validators
            .iter()
            .map(|validator| validator.validate_async(&value))
            .collect();

        Box::pin(async move {
            let results = join_all(futures).await;
            let mut errors = ValidationErrors::new();

            for result in results {
                if let Err(error) = result {
                    errors.add(error);
                }
            }

            if errors.is_empty() {
                Ok(())
            } else {
                // Return first error for simplicity, or combine them
                Err(errors.errors.into_iter().next().unwrap())
            }
        })
    }
}
```

## Timeout and Circuit Breaker

### Validation with Timeout

```rust
use tokio::time::{timeout, Duration};
use std::pin::Pin;
use std::future::Future;

struct TimeoutValidator<T> {
    inner: Box<dyn AsyncValidator<T>>,
    timeout_duration: Duration,
}

impl<T> TimeoutValidator<T> {
    pub fn new(validator: Box<dyn AsyncValidator<T>>, timeout_duration: Duration) -> Self {
        Self {
            inner: validator,
            timeout_duration,
        }
    }
}

impl<T> AsyncValidator<T> for TimeoutValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let inner_future = self.inner.validate_async(value);
        let timeout_duration = self.timeout_duration;

        Box::pin(async move {
            match timeout(timeout_duration, inner_future).await {
                Ok(result) => result,
                Err(_) => Err(ValidationError::new(
                    "validation_timeout",
                    format!("Validation timed out after {:?}", timeout_duration)
                )),
            }
        })
    }
}

// Usage
let email_validator = TimeoutValidator::new(
Box::new(EmailVerificationValidator::new("api-key".to_string(), false)),
Duration::from_secs(5), // 5 second timeout
);

let email = Text::new("user@slowdomain.com");
email_validator.validate_async( & email).await?;
```

### Circuit Breaker Pattern

```rust
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
struct CircuitBreakerValidator<T> {
    inner: Arc<dyn AsyncValidator<T>>,
    state: Arc<AtomicU8>, // 0=Closed, 1=Open, 2=HalfOpen
    failure_count: Arc<AtomicU64>,
    last_failure_time: Arc<std::sync::Mutex<Option<Instant>>>,
    failure_threshold: u64,
    recovery_timeout: Duration,
}

const CLOSED: u8 = 0;
const OPEN: u8 = 1;
const HALF_OPEN: u8 = 2;

impl<T> CircuitBreakerValidator<T> {
    pub fn new(
        validator: Arc<dyn AsyncValidator<T>>, 
        failure_threshold: u64,
        recovery_timeout: Duration,
    ) -> Self {
        Self {
            inner: validator,
            state: Arc::new(AtomicU8::new(CLOSED)),
            failure_count: Arc::new(AtomicU64::new(0)),
            last_failure_time: Arc::new(std::sync::Mutex::new(None)),
            failure_threshold,
            recovery_timeout,
        }
    }
    
    fn should_attempt_reset(&self) -> bool {
        if let Ok(last_failure) = self.last_failure_time.lock() {
            if let Some(last_time) = *last_failure {
                return Instant::now().duration_since(last_time) > self.recovery_timeout;
            }
        }
        false
    }
    
    fn on_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        self.state.store(CLOSED, Ordering::Relaxed);
    }
    
    fn on_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        
        if failures >= self.failure_threshold {
            self.state.store(OPEN, Ordering::Relaxed);
            if let Ok(mut last_failure) = self.last_failure_time.lock() {
                *last_failure = Some(Instant::now());
            }
        }
    }
}

impl<T> AsyncValidator<T> for CircuitBreakerValidator<T>
where
    T: Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output = Result<(), ValidationError>> + Send>>;
    
    fn validate_async(&self, value: &T) -> Self::Future {
        let state = self.state.load(Ordering::Relaxed);
        
        match state {
            OPEN => {
                if self.should_attempt_reset() {
                    self.state.store(HALF_OPEN, Ordering::Relaxed);
                } else {
                    return Box::pin(async {
                        Err(ValidationError::new(
                            "circuit_breaker_open",
                            "Validation service is temporarily unavailable"
                        ))
                    });
                }
            }
            _ => {}
        }
        
        let inner_future = self.inner.validate_async(value);
        let circuit_breaker = self.clone();
        
        Box::pin(async move {
            match inner_future.await {
                Ok(()) => {
                    circuit_breaker.on_success();
                    Ok(())
                }
                Err(error) => {
                    circuit_breaker.on_failure();
                    Err(error)
                }
            }
        })
    }
}
```

## Real-World Integration

### User Registration with Async Validation

```rust
use nebula_value::prelude::*;
use sqlx::PgPool;

struct UserRegistrationValidator {
    db_pool: PgPool,
    email_api_key: String,
}

impl UserRegistrationValidator {
    pub fn new(db_pool: PgPool, email_api_key: String) -> Self {
        Self { db_pool, email_api_key }
    }

    pub async fn validate_registration(
        &self,
        email: &Text,
        username: &Text,
    ) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        // Parallel async validations
        let email_unique = DatabaseUniqueValidator::email(self.db_pool.clone())
            .validate_async(email);
        let email_verification = EmailVerificationValidator::new(
            self.email_api_key.clone(),
            false
        ).validate_async(email);
        let username_unique = DatabaseUniqueValidator::username(self.db_pool.clone())
            .validate_async(username);

        // Wait for all validations
        let (email_unique_result, email_verification_result, username_unique_result) =
            futures::join!(email_unique, email_verification, username_unique);

        if let Err(e) = email_unique_result {
            errors.add(e.with_path("email"));
        }

        if let Err(e) = email_verification_result {
            errors.add(e.with_path("email"));
        }

        if let Err(e) = username_unique_result {
            errors.add(e.with_path("username"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// Usage in web handler
async fn register_user(
    db: &PgPool,
    email_api_key: &str,
    registration_data: UserRegistrationData,
) -> Result<User, ValidationErrors> {
    let validator = UserRegistrationValidator::new(db.clone(), email_api_key.to_string());

    let email = Text::new(registration_data.email);
    let username = Text::new(registration_data.username);

    // Synchronous validation first (fast fail)
    let mut sync_errors = ValidationErrors::new();

    if let Err(e) = email.validate(&Email) {
        sync_errors.add(e.with_path("email"));
    }

    if let Err(e) = username.validate(&MinLength::new(3).and(AlphanumericOnly)) {
        sync_errors.add(e.with_path("username"));
    }

    if !sync_errors.is_empty() {
        return Err(sync_errors);
    }

    // Async validation (potentially slow)
    validator.validate_registration(&email, &username).await?;

    // Create user if all validation passes
    create_user(db, email, username).await
}
```

## Testing Async Validators

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_value::prelude::*;

    #[tokio::test]
    async fn test_database_unique_validator() {
        let pool = setup_test_database().await;

        // Insert test data
        sqlx::query("INSERT INTO users (email) VALUES ('existing@example.com')")
            .execute(&pool)
            .await
            .unwrap();

        let validator = DatabaseUniqueValidator::email(pool);

        // Test unique email (should pass)
        let new_email = Text::new("new@example.com");
        assert!(validator.validate_async(&new_email).await.is_ok());

        // Test existing email (should fail)
        let existing_email = Text::new("existing@example.com");
        let result = validator.validate_async(&existing_email).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.code, "email_taken");
        assert_eq!(error.get_param("value"), Some("existing@example.com"));
    }

    #[tokio::test]
    async fn test_timeout_validator() {
        // Create a slow validator for testing
        struct SlowValidator;

        impl AsyncValidator<Text> for SlowValidator {
            type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

            fn validate_async(&self, _value: &Text) -> Self::Future {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Ok(())
                })
            }
        }

        let timeout_validator = TimeoutValidator::new(
            Box::new(SlowValidator),
            Duration::from_millis(100), // Short timeout
        );

        let text = Text::new("test");
        let result = timeout_validator.validate_async(&text).await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, "validation_timeout");
    }
}
```

## Performance Considerations

### Connection Pooling

```rust
use nebula_value::prelude::*;
use sqlx::PgPool;
use std::sync::Arc;

// Reuse connection pools across validators
struct ValidatorFactory {
    db_pool: Arc<PgPool>,
    email_api_key: String,
}

impl ValidatorFactory {
    pub fn new(db_pool: PgPool, email_api_key: String) -> Self {
        Self {
            db_pool: Arc::new(db_pool),
            email_api_key,
        }
    }

    pub fn email_unique_validator(&self) -> DatabaseUniqueValidator {
        DatabaseUniqueValidator::email(self.db_pool.as_ref().clone())
    }

    pub fn username_unique_validator(&self) -> DatabaseUniqueValidator {
        DatabaseUniqueValidator::username(self.db_pool.as_ref().clone())
    }

    pub fn email_verification_validator(&self) -> EmailVerificationValidator {
        EmailVerificationValidator::new(self.email_api_key.clone(), false)
    }
}
```

### Caching Results

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

struct CachedAsyncValidator<T> {
    inner: Box<dyn AsyncValidator<T>>,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    cache_duration: Duration,
}

struct CacheEntry {
    result: Result<(), ValidationError>,
    timestamp: Instant,
}

impl<T> CachedAsyncValidator<T>
where
    T: std::fmt::Display,
{
    pub fn new(validator: Box<dyn AsyncValidator<T>>, cache_duration: Duration) -> Self {
        Self {
            inner: validator,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_duration,
        }
    }
}

impl<T> AsyncValidator<T> for CachedAsyncValidator<T>
where
    T: std::fmt::Display + Send + Sync,
{
    type Future = Pin<Box<dyn Future<Output=Result<(), ValidationError>> + Send>>;

    fn validate_async(&self, value: &T) -> Self::Future {
        let key = value.to_string();
        let cache = self.cache.clone();
        let cache_duration = self.cache_duration;
        let inner_future = self.inner.validate_async(value);

        Box::pin(async move {
            // Check cache first
            {
                let cache_read = cache.read().await;
                if let Some(entry) = cache_read.get(&key) {
                    if Instant::now().duration_since(entry.timestamp) < cache_duration {
                        return entry.result.clone();
                    }
                }
            }

            // Not in cache or expired, validate and cache result
            let result = inner_future.await;

            {
                let mut cache_write = cache.write().await;
                cache_write.insert(key, CacheEntry {
                    result: result.clone(),
                    timestamp: Instant::now(),
                });
            }

            result
        })
    }
}
```

## Next Steps

- [Error Handling](error-handling.md) - Advanced error patterns for async validation
- [Integration Guides](integration/) - Framework-specific async validation patterns
- [Performance Guide](performance.md) - Optimizing async validation performance
- [Custom Types](custom-types.md) - Building types with async validation built-in