---
title: Performance Tuning
tags: [performance, optimization, latency, throughput, caching, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, platform-engineer, sre]
estimated_reading: 20
priority: P3
---

# Performance Tuning

> [!NOTE] Production Performance
> nebula-credential is optimized for low latency (<100ms p95) and high throughput (10K ops/sec). This guide covers performance targets, optimization strategies, and troubleshooting.

## TL;DR

Performance targets and optimizations:
- **Latency**: <10ms (cache hit), <100ms (cache miss) at p95
- **Throughput**: 10K operations/second per instance
- **Caching**: TTL-based with LRU eviction
- **Connection Pooling**: Database and HTTP connections
- **Concurrency**: Lock-free reads, optimistic writes

---

## Performance Targets

### Latency Targets

| Operation | Cache Hit (p95) | Cache Miss (p95) | Target |
|-----------|-----------------|------------------|--------|
| **Get Credential** | <10ms | <100ms | ✅ |
| **Store Credential** | N/A | <150ms | ✅ |
| **Rotate Credential** | N/A | <500ms | ✅ |
| **Delete Credential** | N/A | <100ms | ✅ |
| **Encrypt/Decrypt** | N/A | <5ms | ✅ |
| **Key Derivation** | N/A | <50ms | ✅ |

### Throughput Targets

| Metric | Target | Notes |
|--------|--------|-------|
| **Operations/sec (single instance)** | 10,000 | Cache hit heavy |
| **Operations/sec (cluster)** | 100,000 | Linear scaling |
| **Concurrent connections** | 1,000 | Per instance |
| **Cache hit ratio** | >80% | Production workload |

---

## Caching Strategy

### Cache Implementation

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use lru::LruCache;

pub struct CredentialCache {
    cache: Arc<RwLock<LruCache<CredentialId, CachedCredential>>>,
    ttl: Duration,
    max_size: usize,
}

pub struct CachedCredential {
    credential: Credential,
    cached_at: DateTime<Utc>,
}

impl CredentialCache {
    pub fn new(max_size: usize, ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(max_size).unwrap()))),
            ttl,
            max_size,
        }
    }
    
    /// Get from cache (lock-free read)
    pub async fn get(&self, id: &CredentialId) -> Option<Credential> {
        let cache = self.cache.read().await;
        
        if let Some(cached) = cache.peek(id) {
            // Check TTL
            if Utc::now() - cached.cached_at < self.ttl {
                return Some(cached.credential.clone());
            }
        }
        
        None
    }
    
    /// Put into cache
    pub async fn put(&self, id: CredentialId, credential: Credential) {
        let mut cache = self.cache.write().await;
        
        cache.put(id, CachedCredential {
            credential,
            cached_at: Utc::now(),
        });
    }
    
    /// Invalidate cached entry
    pub async fn invalidate(&self, id: &CredentialId) {
        let mut cache = self.cache.write().await;
        cache.pop(id);
    }
}
```

### Cache Configuration

```rust
pub struct CacheConfig {
    /// Maximum cache size (number of credentials)
    pub max_size: usize,  // Default: 10,000
    
    /// Time-to-live for cached credentials
    pub ttl: Duration,  // Default: 5 minutes
    
    /// Eviction policy
    pub eviction: EvictionPolicy::Lru,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: 10_000,
            ttl: Duration::minutes(5),
            eviction: EvictionPolicy::Lru,
        }
    }
}
```

**Cache Hit Ratio**:
```rust
pub fn calculate_cache_hit_ratio(&self) -> f64 {
    let hits = self.metrics.cache_hits.get();
    let misses = self.metrics.cache_misses.get();
    let total = hits + misses;
    
    if total == 0 {
        0.0
    } else {
        hits as f64 / total as f64
    }
}
```

---

## Connection Pooling

### Database Connection Pool

```rust
use sqlx::postgres::PgPoolOptions;

pub async fn create_db_pool() -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(100)        // Maximum connections
        .min_connections(10)         // Minimum idle connections
        .acquire_timeout(Duration::from_secs(3))
        .idle_timeout(Duration::from_secs(600))  // 10 minutes
        .max_lifetime(Duration::from_secs(1800))  // 30 minutes
        .connect("postgresql://user:pass@localhost/credentials")
        .await
}
```

### HTTP Connection Pool

```rust
use reqwest::Client;

pub fn create_http_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .pool_max_idle_per_host(50)  // Max idle connections per host
        .pool_idle_timeout(Duration::from_secs(90))
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
}
```

---

## Concurrency Optimizations

### Lock-Free Reads

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct CredentialStore {
    // Read-heavy workload: RwLock allows multiple concurrent readers
    credentials: Arc<RwLock<HashMap<CredentialId, Credential>>>,
}

impl CredentialStore {
    /// Concurrent reads (no locking between readers)
    pub async fn get(&self, id: &CredentialId) -> Option<Credential> {
        let credentials = self.credentials.read().await;  // Multiple readers
        credentials.get(id).cloned()
    }
    
    /// Exclusive write (blocks readers)
    pub async fn put(&self, id: CredentialId, credential: Credential) {
        let mut credentials = self.credentials.write().await;  // Exclusive
        credentials.insert(id, credential);
    }
}
```

### Optimistic Concurrency Control

```rust
pub struct VersionedCredential {
    pub credential: Credential,
    pub version: u64,
}

impl CredentialStore {
    /// Update with optimistic locking (CAS - Compare-And-Swap)
    pub async fn update_optimistic(
        &self,
        id: &CredentialId,
        expected_version: u64,
        new_credential: Credential,
    ) -> Result<(), UpdateError> {
        let mut credentials = self.credentials.write().await;
        
        match credentials.get_mut(id) {
            Some(versioned) => {
                // Check version (optimistic lock)
                if versioned.version != expected_version {
                    return Err(UpdateError::VersionMismatch);
                }
                
                // Update
                versioned.credential = new_credential;
                versioned.version += 1;
                Ok(())
            }
            None => Err(UpdateError::NotFound),
        }
    }
}
```

---

## Batch Operations

### Batch Retrieval

```rust
impl CredentialService {
    /// Retrieve multiple credentials in one operation
    pub async fn get_batch(
        &self,
        ids: &[CredentialId],
    ) -> Result<Vec<Credential>, CredentialError> {
        // Check cache first
        let mut results = Vec::with_capacity(ids.len());
        let mut cache_misses = Vec::new();
        
        for id in ids {
            if let Some(credential) = self.cache.get(id).await {
                results.push(credential);
            } else {
                cache_misses.push(id.clone());
            }
        }
        
        // Batch fetch cache misses
        if !cache_misses.is_empty() {
            let fetched = self.storage.get_batch(&cache_misses).await?;
            
            for credential in fetched {
                // Cache each credential
                self.cache.put(credential.id.clone(), credential.clone()).await;
                results.push(credential);
            }
        }
        
        Ok(results)
    }
}
```

---

## Memory Optimization

### Lazy Deserialization

```rust
pub struct LazyCredential {
    id: CredentialId,
    encrypted_data: Vec<u8>,  // Keep encrypted until needed
    decrypted: OnceCell<Credential>,
}

impl LazyCredential {
    /// Decrypt only when accessed
    pub fn get(&self, key: &EncryptionKey) -> Result<&Credential, EncryptionError> {
        self.decrypted.get_or_try_init(|| {
            decrypt_credential(&self.encrypted_data, key)
        })
    }
}
```

### Memory-Efficient Caching

```rust
/// Only cache hot credentials (frequently accessed)
pub struct HotCredentialCache {
    cache: LruCache<CredentialId, Credential>,
    access_count: HashMap<CredentialId, usize>,
    hot_threshold: usize,  // Cache if accessed > threshold
}

impl HotCredentialCache {
    pub async fn get(&mut self, id: &CredentialId) -> Option<&Credential> {
        // Track access
        *self.access_count.entry(id.clone()).or_insert(0) += 1;
        
        self.cache.get(id)
    }
    
    pub async fn consider_caching(&mut self, id: CredentialId, credential: Credential) {
        // Only cache if hot
        if self.access_count.get(&id).copied().unwrap_or(0) >= self.hot_threshold {
            self.cache.put(id, credential);
        }
    }
}
```

---

## Database Query Optimization

### Index Strategy

```sql
-- Primary key index (automatic)
CREATE INDEX idx_credentials_id ON credentials(id);

-- Owner lookup index
CREATE INDEX idx_credentials_owner ON credentials(owner);

-- Scope filtering index
CREATE INDEX idx_credentials_scope ON credentials(scope_type, scope_value);

-- Expiration cleanup index
CREATE INDEX idx_credentials_expires ON credentials(expires_at)
WHERE expires_at IS NOT NULL;

-- Composite index for common queries
CREATE INDEX idx_credentials_owner_scope ON credentials(owner, scope_type);
```

### Query Optimization

```rust
// BAD: N+1 query problem
for id in credential_ids {
    let credential = sqlx::query_as!(Credential, "SELECT * FROM credentials WHERE id = $1", id)
        .fetch_one(&pool)
        .await?;
}

// GOOD: Single batch query
let credentials = sqlx::query_as!(
    Credential,
    "SELECT * FROM credentials WHERE id = ANY($1)",
    &credential_ids[..]
)
.fetch_all(&pool)
.await?;
```

---

## Benchmarking

### Latency Benchmarks

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_get_credential(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let service = rt.block_on(async { setup_test_service().await });
    
    let mut group = c.benchmark_group("get_credential");
    
    for cache_status in ["cache_hit", "cache_miss"].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(cache_status),
            cache_status,
            |b, &status| {
                b.to_async(&rt).iter(|| async {
                    if status == &"cache_miss" {
                        service.cache.invalidate(&test_id).await;
                    }
                    
                    black_box(service.get_credential(&test_id).await)
                });
            }
        );
    }
    
    group.finish();
}

criterion_group!(benches, bench_get_credential);
criterion_main!(benches);
```

**Results**:
```
get_credential/cache_hit    time: [8.2ms 8.5ms 8.9ms]
get_credential/cache_miss   time: [87ms 92ms 98ms]
```

---

## Profiling

### CPU Profiling

```bash
# Install cargo-flamegraph
$ cargo install flamegraph

# Run with profiling
$ cargo flamegraph --bin nebula-credential

# Output: flamegraph.svg
```

### Memory Profiling

```bash
# Install heaptrack
$ heaptrack nebula-credential

# Analyze results
$ heaptrack_gui heaptrack.nebula-credential.*.zst
```

---

## Performance Monitoring

### Key Metrics

```rust
pub struct PerformanceMetrics {
    /// Latency percentiles
    pub latency_p50: f64,
    pub latency_p95: f64,
    pub latency_p99: f64,
    
    /// Throughput
    pub ops_per_second: f64,
    
    /// Cache efficiency
    pub cache_hit_ratio: f64,
    
    /// Resource utilization
    pub cpu_usage: f64,
    pub memory_usage_mb: f64,
    pub connection_pool_usage: f64,
}

impl PerformanceMetrics {
    pub async fn collect(&self) -> Self {
        Self {
            latency_p50: self.calculate_percentile(0.50),
            latency_p95: self.calculate_percentile(0.95),
            latency_p99: self.calculate_percentile(0.99),
            ops_per_second: self.calculate_throughput(),
            cache_hit_ratio: self.calculate_cache_hit_ratio(),
            cpu_usage: self.get_cpu_usage(),
            memory_usage_mb: self.get_memory_usage(),
            connection_pool_usage: self.get_pool_usage(),
        }
    }
}
```

---

## Troubleshooting Performance Issues

### Issue: High Latency

**Symptoms**:
```
p95 latency: 500ms (target: <100ms)
```

**Diagnosis**:
```rust
// Check where time is spent
#[instrument]
async fn get_credential_instrumented(&self, id: &CredentialId) -> Result<Credential, Error> {
    let span = tracing::Span::current();
    
    let _guard = span.enter();
    tracing::info!("Checking cache");
    let cache_result = self.cache.get(id).await;
    
    if cache_result.is_none() {
        tracing::info!("Cache miss, querying storage");
        let storage_result = self.storage.get(id).await?;
        
        tracing::info!("Decrypting credential");
        let decrypted = self.decrypt(&storage_result)?;
        
        return Ok(decrypted);
    }
    
    Ok(cache_result.unwrap())
}
```

**Solutions**:
1. Increase cache TTL
2. Add database indexes
3. Enable connection pooling
4. Use batch operations

---

### Issue: Low Throughput

**Symptoms**:
```
Current: 1,000 ops/sec
Target: 10,000 ops/sec
```

**Diagnosis**:
```bash
# Check concurrent connections
$ netstat -an | grep ESTABLISHED | wc -l
50  # Too low, increase connection pool

# Check CPU usage
$ top -bn1 | grep nebula-credential
%CPU: 20%  # Underutilized, increase parallelism
```

**Solutions**:
1. Increase connection pool size
2. Add horizontal scaling (more instances)
3. Enable caching
4. Optimize database queries

---

### Issue: High Memory Usage

**Symptoms**:
```
Memory usage: 2GB
Cache size: 100,000 credentials
```

**Solutions**:
```rust
// Reduce cache size
cache_config.max_size = 10_000;  // Down from 100,000

// Reduce cache TTL
cache_config.ttl = Duration::minutes(1);  // Down from 5 minutes

// Enable lazy deserialization
use LazyCredential instead of Credential
```

---

## Best Practices

### 1. Enable Caching

```rust
// ✅ GOOD: Cache enabled with appropriate TTL
let cache_config = CacheConfig {
    max_size: 10_000,
    ttl: Duration::minutes(5),
    eviction: EvictionPolicy::Lru,
};

// ❌ BAD: No caching
let cache_config = CacheConfig {
    max_size: 0,  // Cache disabled
    ttl: Duration::zero(),
};
```

---

### 2. Use Connection Pooling

```rust
// ✅ GOOD: Connection pool
let pool = PgPoolOptions::new()
    .max_connections(100)
    .connect(db_url)
    .await?;

// ❌ BAD: New connection per request
for request in requests {
    let conn = PgConnection::connect(db_url).await?;  // Slow!
    // ...
}
```

---

### 3. Batch Operations

```rust
// ✅ GOOD: Batch query
let credentials = storage.get_batch(&ids).await?;

// ❌ BAD: Individual queries
let mut credentials = Vec::new();
for id in ids {
    credentials.push(storage.get(id).await?);  // N queries!
}
```

---

## Performance Checklist

### Development

- [ ] Enable caching with appropriate TTL
- [ ] Use connection pooling
- [ ] Implement batch operations
- [ ] Add database indexes
- [ ] Use lock-free data structures where possible
- [ ] Profile hot paths

### Deployment

- [ ] Configure appropriate cache size
- [ ] Tune connection pool size
- [ ] Enable horizontal scaling
- [ ] Monitor latency percentiles (p50, p95, p99)
- [ ] Set up alerting for performance degradation
- [ ] Regular load testing

---

## See Also

- [[Advanced/Observability-Guide|Observability Guide]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[How-To/Enable-Audit-Logging|Audit Logging]]
- [[Reference/API-Reference|API Reference]]
