# Nebula Complete Documentation - Part 3

---
## FILE: docs/crates/nebula-storage.md
---

# nebula-storage

## Purpose

`nebula-storage` provides the abstraction layer for persistent storage, allowing Nebula to work with different storage backends while maintaining a consistent API.

## Responsibilities

- Storage backend abstraction
- Workflow definition persistence
- Execution state management
- Query and filtering capabilities
- Transaction support
- Migration management

## Architecture

### Core Traits

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync {
    // Workflow management
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error>;
    async fn load_workflow(&self, id: &WorkflowId) -> Result<Workflow, Error>;
    async fn update_workflow(&self, id: &WorkflowId, workflow: &Workflow) -> Result<(), Error>;
    async fn delete_workflow(&self, id: &WorkflowId) -> Result<(), Error>;
    async fn list_workflows(&self, filter: WorkflowFilter) -> Result<Vec<WorkflowSummary>, Error>;
    
    // Execution management
    async fn create_execution(&self, execution: &ExecutionState) -> Result<(), Error>;
    async fn update_execution(&self, execution: &ExecutionState) -> Result<(), Error>;
    async fn load_execution(&self, id: &ExecutionId) -> Result<ExecutionState, Error>;
    async fn list_executions(&self, filter: ExecutionFilter) -> Result<Vec<ExecutionSummary>, Error>;
    
    // Node outputs
    async fn save_node_output(&self, execution_id: &ExecutionId, node_id: &NodeId, output: &WorkflowDataItem) -> Result<(), Error>;
    async fn load_node_output(&self, execution_id: &ExecutionId, node_id: &NodeId) -> Result<WorkflowDataItem, Error>;
    
    // Transactions
    async fn transaction<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut dyn Transaction) -> Result<R, Error> + Send,
        R: Send;
}

#[async_trait]
pub trait Transaction: Send {
    async fn save_workflow(&mut self, workflow: &Workflow) -> Result<(), Error>;
    async fn update_execution(&mut self, execution: &ExecutionState) -> Result<(), Error>;
    async fn commit(self) -> Result<(), Error>;
    async fn rollback(self) -> Result<(), Error>;
}
```

### Query System

```rust
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilter {
    pub ids: Option<Vec<WorkflowId>>,
    pub name_pattern: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub status: Option<WorkflowStatus>,
    pub sort_by: Option<WorkflowSortField>,
    pub sort_order: Option<SortOrder>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionFilter {
    pub ids: Option<Vec<ExecutionId>>,
    pub workflow_ids: Option<Vec<WorkflowId>>,
    pub status: Option<Vec<ExecutionStatus>>,
    pub started_after: Option<DateTime<Utc>>,
    pub started_before: Option<DateTime<Utc>>,
    pub completed_after: Option<DateTime<Utc>>,
    pub completed_before: Option<DateTime<Utc>>,
    pub error_type: Option<String>,
    pub sort_by: Option<ExecutionSortField>,
    pub sort_order: Option<SortOrder>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub enum WorkflowSortField {
    Name,
    CreatedAt,
    UpdatedAt,
    ExecutionCount,
}

pub enum ExecutionSortField {
    StartedAt,
    CompletedAt,
    Duration,
    WorkflowName,
}
```

### Storage Configuration

```rust
pub struct StorageConfig {
    // Connection settings
    pub connection: ConnectionConfig,
    
    // Pool settings
    pub pool: PoolConfig,
    
    // Performance settings
    pub performance: PerformanceConfig,
    
    // Maintenance settings
    pub maintenance: MaintenanceConfig,
}

pub struct ConnectionConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connection_timeout: Duration,
    pub idle_timeout: Option<Duration>,
}

pub struct PerformanceConfig {
    pub statement_cache_size: usize,
    pub query_timeout: Duration,
    pub batch_size: usize,
}

pub struct MaintenanceConfig {
    pub auto_vacuum: bool,
    pub analyze_interval: Duration,
    pub backup_schedule: Option<CronExpression>,
}
```

## PostgreSQL Implementation

### Schema Design

```sql
-- Workflows table
CREATE TABLE workflows (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    version VARCHAR(50) NOT NULL,
    category VARCHAR(100),
    tags JSONB DEFAULT '[]',
    definition JSONB NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by VARCHAR(255),
    INDEX idx_workflows_name (name),
    INDEX idx_workflows_category (category),
    INDEX idx_workflows_status (status),
    INDEX idx_workflows_created_at (created_at)
);

-- Executions table
CREATE TABLE executions (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflows(id),
    status VARCHAR(50) NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    error JSONB,
    context JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    INDEX idx_executions_workflow_id (workflow_id),
    INDEX idx_executions_status (status),
    INDEX idx_executions_started_at (started_at)
);

-- Node outputs table
CREATE TABLE node_outputs (
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    node_id VARCHAR(255) NOT NULL,
    output JSONB NOT NULL,
    binary_refs JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (execution_id, node_id),
    INDEX idx_node_outputs_created_at (created_at)
);

-- Workflow triggers table
CREATE TABLE workflow_triggers (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    trigger_type VARCHAR(100) NOT NULL,
    configuration JSONB NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    last_triggered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    INDEX idx_triggers_workflow_id (workflow_id),
    INDEX idx_triggers_type (trigger_type)
);
```

### Implementation

```rust
pub struct PostgresStorage {
    pool: PgPool,
    query_builder: QueryBuilder,
    migration_runner: MigrationRunner,
}

impl PostgresStorage {
    pub async fn new(config: StorageConfig) -> Result<Self, Error> {
        let pool = PgPoolOptions::new()
            .max_connections(config.pool.max_connections)
            .min_connections(config.pool.min_connections)
            .connect_timeout(config.connection.connection_timeout)
            .connect(&config.connection.url)
            .await?;
            
        let storage = Self {
            pool,
            query_builder: QueryBuilder::new(),
            migration_runner: MigrationRunner::new(),
        };
        
        // Run migrations
        storage.migration_runner.run(&storage.pool).await?;
        
        Ok(storage)
    }
}

#[async_trait]
impl StorageBackend for PostgresStorage {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        let definition_json = serde_json::to_value(&workflow.definition)?;
        let tags_json = serde_json::to_value(&workflow.tags)?;
        
        sqlx::query!(
            r#"
            INSERT INTO workflows (id, name, description, version, category, tags, definition, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                version = EXCLUDED.version,
                category = EXCLUDED.category,
                tags = EXCLUDED.tags,
                definition = EXCLUDED.definition,
                status = EXCLUDED.status,
                updated_at = NOW()
            "#,
            workflow.id.as_uuid(),
            workflow.name,
            workflow.description,
            workflow.version.to_string(),
            workflow.category,
            tags_json,
            definition_json,
            workflow.status.as_str()
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    async fn list_workflows(&self, filter: WorkflowFilter) -> Result<Vec<WorkflowSummary>, Error> {
        let query = self.query_builder.build_workflow_query(&filter);
        
        let rows = sqlx::query_as::<_, WorkflowRow>(&query)
            .bind(filter.limit.unwrap_or(100) as i64)
            .bind(filter.offset.unwrap_or(0) as i64)
            .fetch_all(&self.pool)
            .await?;
            
        Ok(rows.into_iter().map(WorkflowSummary::from).collect())
    }
    
    async fn transaction<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut dyn Transaction) -> Result<R, Error> + Send,
        R: Send,
    {
        let mut tx = self.pool.begin().await?;
        
        let mut postgres_tx = PostgresTransaction { tx: &mut tx };
        
        match f(&mut postgres_tx) {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }
}
```

### Query Builder

```rust
pub struct QueryBuilder;

impl QueryBuilder {
    pub fn build_workflow_query(&self, filter: &WorkflowFilter) -> String {
        let mut query = String::from(
            "SELECT id, name, description, version, category, tags, status, created_at, updated_at 
             FROM workflows WHERE 1=1"
        );
        
        if let Some(ids) = &filter.ids {
            let ids_str = ids.iter()
                .map(|id| format!("'{}'", id))
                .collect::<Vec<_>>()
                .join(",");
            query.push_str(&format!(" AND id IN ({})", ids_str));
        }
        
        if let Some(pattern) = &filter.name_pattern {
            query.push_str(&format!(" AND name ILIKE '%{}%'", pattern));
        }
        
        if let Some(category) = &filter.category {
            query.push_str(&format!(" AND category = '{}'", category));
        }
        
        if let Some(created_after) = &filter.created_after {
            query.push_str(&format!(" AND created_at >= '{}'", created_after));
        }
        
        if let Some(created_before) = &filter.created_before {
            query.push_str(&format!(" AND created_at <= '{}'", created_before));
        }
        
        // Sorting
        let sort_field = match filter.sort_by {
            Some(WorkflowSortField::Name) => "name",
            Some(WorkflowSortField::CreatedAt) => "created_at",
            Some(WorkflowSortField::UpdatedAt) => "updated_at",
            _ => "created_at",
        };
        
        let sort_order = match filter.sort_order {
            Some(SortOrder::Asc) => "ASC",
            Some(SortOrder::Desc) => "DESC",
            None => "DESC",
        };
        
        query.push_str(&format!(" ORDER BY {} {}", sort_field, sort_order));
        query.push_str(" LIMIT $1 OFFSET $2");
        
        query
    }
}
```

### Migration System

```rust
pub struct MigrationRunner {
    migrations: Vec<Migration>,
}

pub struct Migration {
    pub version: i32,
    pub name: String,
    pub up: String,
    pub down: String,
}

impl MigrationRunner {
    pub async fn run(&self, pool: &PgPool) -> Result<(), Error> {
        // Create migrations table if not exists
        self.create_migrations_table(pool).await?;
        
        // Get current version
        let current_version = self.get_current_version(pool).await?;
        
        // Run pending migrations
        for migration in &self.migrations {
            if migration.version > current_version {
                self.run_migration(pool, migration).await?;
            }
        }
        
        Ok(())
    }
    
    async fn create_migrations_table(&self, pool: &PgPool) -> Result<(), Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#
        )
        .execute(pool)
        .await?;
        
        Ok(())
    }
    
    async fn run_migration(&self, pool: &PgPool, migration: &Migration) -> Result<(), Error> {
        let mut tx = pool.begin().await?;
        
        // Execute migration
        sqlx::query(&migration.up)
            .execute(&mut tx)
            .await?;
            
        // Record migration
        sqlx::query!(
            "INSERT INTO schema_migrations (version, name) VALUES ($1, $2)",
            migration.version,
            migration.name
        )
        .execute(&mut tx)
        .await?;
        
        tx.commit().await?;
        
        info!("Applied migration: {} - {}", migration.version, migration.name);
        
        Ok(())
    }
}
```

## Caching Layer

```rust
pub struct CachedStorage<S: StorageBackend> {
    backend: S,
    cache: Arc<StorageCache>,
}

pub struct StorageCache {
    workflows: RwLock<LruCache<WorkflowId, Workflow>>,
    executions: RwLock<LruCache<ExecutionId, ExecutionState>>,
    node_outputs: RwLock<LruCache<(ExecutionId, NodeId), WorkflowDataItem>>,
    ttl: Duration,
}

#[async_trait]
impl<S: StorageBackend> StorageBackend for CachedStorage<S> {
    async fn load_workflow(&self, id: &WorkflowId) -> Result<Workflow, Error> {
        // Check cache first
        if let Some(workflow) = self.cache.workflows.read().await.get(id) {
            return Ok(workflow.clone());
        }
        
        // Load from backend
        let workflow = self.backend.load_workflow(id).await?;
        
        // Update cache
        self.cache.workflows.write().await.put(id.clone(), workflow.clone());
        
        Ok(workflow)
    }
    
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        // Save to backend
        self.backend.save_workflow(workflow).await?;
        
        // Update cache
        self.cache.workflows.write().await.put(workflow.id.clone(), workflow.clone());
        
        Ok(())
    }
}
```

## Storage Adapters

### Redis Adapter

```rust
pub struct RedisStorage {
    client: RedisClient,
    serializer: Box<dyn Serializer>,
}

#[async_trait]
impl StorageBackend for RedisStorage {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        let key = format!("workflow:{}", workflow.id);
        let value = self.serializer.serialize(workflow)?;
        
        self.client
            .set_ex(key, value, self.ttl.as_secs())
            .await?;
            
        // Update index
        self.client
            .sadd("workflows:all", workflow.id.to_string())
            .await?;
            
        Ok(())
    }
}
```

### S3 Adapter

```rust
pub struct S3Storage {
    client: S3Client,
    bucket: String,
    prefix: String,
}

#[async_trait]
impl StorageBackend for S3Storage {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        let key = format!("{}/workflows/{}.json", self.prefix, workflow.id);
        let body = serde_json::to_vec(workflow)?;
        
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(body.into())
            .send()
            .await?;
            
        Ok(())
    }
}
```

---
## FILE: docs/crates/nebula-binary.md
---

# nebula-binary

## Purpose

`nebula-binary` handles the storage and management of binary data (files, images, documents) with automatic tiering, garbage collection, and efficient streaming.

## Responsibilities

- Binary data storage strategies
- Automatic storage tiering
- Streaming upload/download
- Garbage collection
- Content deduplication
- Compression support

## Architecture

### Core Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinaryDataLocation {
    /// Small files kept in memory (< 1MB)
    InMemory {
        id: Uuid,
        data: Vec<u8>,
        metadata: BinaryMetadata,
    },
    
    /// Medium files in temporary storage (1-100MB)
    Temp {
        id: Uuid,
        path: PathBuf,
        expires_at: DateTime<Utc>,
        metadata: BinaryMetadata,
    },
    
    /// Large files in object storage (> 100MB)
    Remote {
        id: Uuid,
        storage_type: RemoteStorageType,
        key: String,
        metadata: BinaryMetadata,
    },
    
    /// Generated on-demand (AI images, reports, etc)
    Generated {
        id: Uuid,
        generator: GeneratorType,
        params: serde_json::Value,
        cache_key: Option<String>,
        metadata: BinaryMetadata,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryMetadata {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: usize,
    pub checksum: String,
    pub created_at: DateTime<Utc>,
    pub accessed_at: DateTime<Utc>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteStorageType {
    S3,
    AzureBlob,
    GoogleCloudStorage,
    MinIO,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneratorType {
    AiImage { model: String },
    PdfReport { template: String },
    DataExport { format: ExportFormat },
    Archive { compression: CompressionType },
}
```

### Smart Storage Manager

```rust
pub struct SmartBinaryStorage {
    // Storage tiers
    memory_storage: Arc<MemoryStorage>,
    temp_storage: Arc<TempStorage>,
    remote_storage: Arc<dyn RemoteStorage>,
    
    // Configuration
    config: StorageConfig,
    
    // Metrics
    metrics: Arc<StorageMetrics>,
    
    // Background tasks
    gc_handle: JoinHandle<()>,
    tiering_handle: JoinHandle<()>,
}

pub struct StorageConfig {
    pub memory_threshold: usize,      // Default: 1MB
    pub temp_threshold: usize,        // Default: 100MB
    pub temp_ttl: Duration,          // Default: 24 hours
    pub compression_threshold: usize, // Default: 10MB
    pub deduplication: bool,         // Default: true
    pub gc_interval: Duration,       // Default: 1 hour
}

impl SmartBinaryStorage {
    pub async fn store(
        &self,
        data: BinaryData,
        hints: StorageHints,
    ) -> Result<BinaryHandle, Error> {
        let size = data.size();
        let metadata = self.create_metadata(&data)?;
        
        // Check deduplication
        if self.config.deduplication {
            if let Some(existing) = self.find_duplicate(&metadata.checksum).await? {
                self.metrics.record_dedup_hit();
                return Ok(existing);
            }
        }
        
        // Determine storage location
        let location = match (size, hints) {
            (size, _) if size < self.config.memory_threshold => {
                self.store_in_memory(data, metadata).await?
            }
            
            (size, StorageHints { temp: true, .. }) |
            (size, _) if size < self.config.temp_threshold => {
                self.store_in_temp(data, metadata).await?
            }
            
            _ => {
                self.store_in_remote(data, metadata).await?
            }
        };
        
        Ok(BinaryHandle {
            id: location.id(),
            location,
        })
    }
    
    async fn store_in_memory(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let bytes = data.into_bytes().await?;
        
        self.memory_storage.store(id, bytes.clone()).await?;
        
        Ok(BinaryDataLocation::InMemory {
            id,
            data: bytes,
            metadata,
        })
    }
    
    async fn store_in_temp(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let path = self.temp_storage.create_file(id).await?;
        
        // Stream to file
        let mut file = File::create(&path).await?;
        let mut stream = data.into_stream();
        
        while let Some(chunk) = stream.next().await {
            file.write_all(&chunk?).await?;
        }
        
        file.sync_all().await?;
        
        let expires_at = Utc::now() + self.config.temp_ttl;
        
        Ok(BinaryDataLocation::Temp {
            id,
            path,
            expires_at,
            metadata,
        })
    }
    
    async fn store_in_remote(
        &self,
        data: BinaryData,
        metadata: BinaryMetadata,
    ) -> Result<BinaryDataLocation, Error> {
        let id = Uuid::new_v4();
        let key = self.generate_storage_key(&id, &metadata);
        
        // Apply compression if needed
        let data = if metadata.size > self.config.compression_threshold {
            self.compress_data(data).await?
        } else {
            data
        };
        
        self.remote_storage.upload(&key, data).await?;
        
        Ok(BinaryDataLocation::Remote {
            id,
            storage_type: self.remote_storage.storage_type(),
            key,
            metadata,
        })
    }
}
```

### Storage Tiering

```rust
pub struct StorageTiering {
    analyzer: UsageAnalyzer,
    migrator: DataMigrator,
}

impl StorageTiering {
    pub async fn run_tiering_cycle(&self) -> Result<TieringStats, Error> {
        let mut stats = TieringStats::default();
        
        // Analyze usage patterns
        let analysis = self.analyzer.analyze().await?;
        
        // Promote frequently accessed temp files to memory
        for file in analysis.hot_temp_files {
            if file.access_count > 10 && file.size < 1_000_000 {
                self.migrator.promote_to_memory(&file).await?;
                stats.promoted_to_memory += 1;
            }
        }
        
        // Demote cold memory items to temp
        for item in analysis.cold_memory_items {
            if item.last_access.elapsed() > Duration::from_hours(1) {
                self.migrator.demote_to_temp(&item).await?;
                stats.demoted_to_temp += 1;
            }
        }
        
        // Archive old temp files to remote
        for file in analysis.old_temp_files {
            if file.age > Duration::from_days(1) {
                self.migrator.archive_to_remote(&file).await?;
                stats.archived_to_remote += 1;
            }
        }
        
        Ok(stats)
    }
}
```

### Streaming Support

```rust
pub struct StreamingHandler {
    chunk_size: usize,
    buffer_pool: BufferPool,
}

impl StreamingHandler {
    pub async fn upload_stream<S>(
        &self,
        stream: S,
        hints: UploadHints,
    ) -> Result<BinaryHandle, Error>
    where
        S: Stream<Item = Result<Bytes, Error>> + Send,
    {
        let id = Uuid::new_v4();
        let hasher = Sha256::new();
        let mut size = 0;
        
        // Determine storage based on hints
        let storage = match hints.expected_size {
            Some(s) if s < 1_000_000 => StreamStorage::Memory,
            Some(s) if s < 100_000_000 => StreamStorage::Temp,
            _ => StreamStorage::Remote,
        };
        
        match storage {
            StreamStorage::Memory => {
                let mut buffer = Vec::new();
                pin_mut!(stream);
                
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    hasher.update(&chunk);
                    buffer.extend_from_slice(&chunk);
                    size += chunk.len();
                    
                    if size > 1_000_000 {
                        // Switch to temp storage
                        return self.upgrade_to_temp_storage(buffer, stream).await;
                    }
                }
                
                // Store in memory
                Ok(self.finalize_memory_storage(id, buffer, hasher.finalize()))
            }
            
            StreamStorage::Temp => {
                self.stream_to_temp(id, stream, hasher).await
            }
            
            StreamStorage::Remote => {
                self.stream_to_remote(id, stream, hasher).await
            }
        }
    }
    
    pub fn download_stream(
        &self,
        handle: &BinaryHandle,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>, Error> {
        match &handle.location {
            BinaryDataLocation::InMemory { data, .. } => {
                // Convert to stream
                let chunks = data.chunks(self.chunk_size)
                    .map(|chunk| Ok(Bytes::from(chunk.to_vec())))
                    .collect::<Vec<_>>();
                    
                Ok(Box::pin(futures::stream::iter(chunks)))
            }
            
            BinaryDataLocation::Temp { path, .. } => {
                // Stream from file
                Ok(Box::pin(self.stream_from_file(path.clone())))
            }
            
            BinaryDataLocation::Remote { key, .. } => {
                // Stream from remote storage
                Ok(Box::pin(self.remote_storage.download_stream(key)))
            }
            
            BinaryDataLocation::Generated { generator, params, .. } => {
                // Generate on-the-fly
                Ok(Box::pin(self.generate_stream(generator, params)))
            }
        }
    }
}
```

### Garbage Collection

```rust
pub struct GarbageCollector {
    storage: Arc<SmartBinaryStorage>,
    rules: Vec<Box<dyn GcRule>>,
}

#[async_trait]
pub trait GcRule: Send + Sync {
    async fn should_delete(&self, item: &GcItem) -> bool;
    fn name(&self) -> &str;
}

pub struct TtlRule {
    ttl: Duration,
}

#[async_trait]
impl GcRule for TtlRule {
    async fn should_delete(&self, item: &GcItem) -> bool {
        item.last_access.elapsed() > self.ttl
    }
    
    fn name(&self) -> &str {
        "TTL Rule"
    }
}

impl GarbageCollector {
    pub async fn run_gc_cycle(&self) -> Result<GcStats, Error> {
        let mut stats = GcStats::default();
        
        // Scan all storage locations
        let items = self.scan_all_items().await?;
        
        for item in items {
            let mut should_delete = false;
            let mut matched_rule = None;
            
            // Check all rules
            for rule in &self.rules {
                if rule.should_delete(&item).await {
                    should_delete = true;
                    matched_rule = Some(rule.name());
                    break;
                }
            }
            
            if should_delete {
                self.delete_item(&item).await?;
                stats.deleted_count += 1;
                stats.freed_bytes += item.size;
                
                info!(
                    "GC: Deleted {} ({} bytes) - Rule: {}",
                    item.id,
                    item.size,
                    matched_rule.unwrap_or("unknown")
                );
            }
        }
        
        Ok(stats)
    }
}
```

### Deduplication

```rust
pub struct Deduplicator {
    index: Arc<RwLock<HashMap<String, BinaryHandle>>>,
    hasher: Box<dyn Hasher>,
}

impl Deduplicator {
    pub async fn find_duplicate(&self, data: &[u8]) -> Option<BinaryHandle> {
        let hash = self.hasher.hash(data);
        self.index.read().await.get(&hash).cloned()
    }
    
    pub async fn register(&self, hash: String, handle: BinaryHandle) {
        self.index.write().await.insert(hash, handle);
    }
    
    pub async fn dedup_stats(&self) -> DedupStats {
        let index = self.index.read().await;
        
        DedupStats {
            unique_files: index.len(),
            total_references: self.count_references(&index).await,
            space_saved: self.calculate_space_saved(&index).await,
        }
    }
}
```

### Compression

```rust
pub struct CompressionManager {
    strategies: HashMap<String, Box<dyn CompressionStrategy>>,
}

#[async_trait]
pub trait CompressionStrategy: Send + Sync {
    async fn compress(&self, data: Bytes) -> Result<Bytes, Error>;
    async fn decompress(&self, data: Bytes) -> Result<Bytes, Error>;
    fn content_types(&self) -> Vec<String>;
}

pub struct ZstdCompression {
    level: i32,
}

#[async_trait]
impl CompressionStrategy for ZstdCompression {
    async fn compress(&self, data: Bytes) -> Result<Bytes, Error> {
        tokio::task::spawn_blocking(move || {
            let compressed = zstd::compress(&data, self.level)?;
            Ok(Bytes::from(compressed))
        })
        .await?
    }
    
    async fn decompress(&self, data: Bytes) -> Result<Bytes, Error> {
        tokio::task::spawn_blocking(move || {
            let decompressed = zstd::decompress(&data)?;
            Ok(Bytes::from(decompressed))
        })
        .await?
    }
    
    fn content_types(&self) -> Vec<String> {
        vec![
            "text/plain".to_string(),
            "application/json".to_string(),
            "application/xml".to_string(),
        ]
    }
}
```

---
## FILE: docs/crates/nebula-runtime.md
---

# nebula-runtime

## Purpose

`nebula-runtime` manages the lifecycle of workflow triggers, coordinates workflow activations, and handles the event-driven aspects of the system.

## Responsibilities

- Trigger lifecycle management
- Event listening and processing
- Workflow activation/deactivation
- Runtime coordination
- Health monitoring
- Resource allocation for triggers

## Architecture

### Core Components

```rust
pub struct Runtime {
    // Unique identifier for this runtime instance
    id: RuntimeId,
    
    // Trigger management
    trigger_manager: Arc<TriggerManager>,
    
    // Event processing
    event_processor: Arc<EventProcessor>,
    
    // Workflow coordination
    coordinator: Arc<WorkflowCoordinator>,
    
    // Health monitoring
    health_monitor: Arc<HealthMonitor>,
    
    // Resource management
    resource_manager: Arc<ResourceManager>,
    
    // Metrics
    metrics: Arc<RuntimeMetrics>,
}

pub struct RuntimeConfig {
    pub id: RuntimeId,
    pub event_bus_config: EventBusConfig,
    pub trigger_config: TriggerConfig,
    pub coordination_config: CoordinationConfig,
    pub resource_limits: ResourceLimits,
}
```

### Trigger Management

```rust
pub struct TriggerManager {
    // Active triggers indexed by workflow ID
    active_triggers: Arc<DashMap<WorkflowId, Vec<ActiveTrigger>>>,
    
    // Trigger registry
    registry: Arc<TriggerRegistry>,
    
    // Lifecycle manager
    lifecycle: Arc<TriggerLifecycle>,
    
    // State persistence
    state_store: Arc<dyn TriggerStateStore>,
}

pub struct ActiveTrigger {
    pub id: TriggerId,
    pub workflow_id: WorkflowId,
    pub trigger_type: TriggerType,
    pub instance: Box<dyn TriggerAction>,
    pub status: TriggerStatus,
    pub handle: TriggerHandle,
    pub created_at: DateTime<Utc>,
    pub last_fired: Option<DateTime<Utc>>,
}

pub enum TriggerStatus {
    Initializing,
    Active,
    Paused,
    Failed { error: String, retry_count: u32 },
    Stopping,
    Stopped,
}

impl TriggerManager {
    pub async fn activate_trigger(
        &self,
        workflow_id: &WorkflowId,
        trigger_def: &TriggerDefinition,
    ) -> Result<TriggerId, Error> {
        // Create trigger instance
        let instance = self.registry
            .create_trigger(&trigger_def.trigger_type, trigger_def.config.clone())?;
            
        // Initialize trigger
        let mut trigger_instance = instance;
        let context = self.create_trigger_context(workflow_id).await?;
        let handle = trigger_instance.initialize(&context).await?;
        
        // Create active trigger
        let trigger_id = TriggerId::new();
        let active_trigger = ActiveTrigger {
            id: trigger_id.clone(),
            workflow_id: workflow_id.clone(),
            trigger_type: trigger_def.trigger_type.clone(),
            instance: trigger_instance,
            status: TriggerStatus::Active,
            handle,
            created_at: Utc::now(),
            last_fired: None,
        };
        
        // Store and start
        self.active_triggers
            .entry(workflow_id.clone())
            .or_insert_with(Vec::new)
            .push(active_trigger);
            
        // Start listening
        self.lifecycle.start_trigger(&trigger_id).await?;
        
        Ok(trigger_id)
    }
    
    pub async fn deactivate_workflow_triggers(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<(), Error> {
        if let Some((_, triggers)) = self.active_triggers.remove(workflow_id) {
            for trigger in triggers {
                self.lifecycle.stop_trigger(&trigger.id).await?;
            }
        }
        
        Ok(())
    }
}
```

### Trigger Types Implementation

```rust
// HTTP Webhook Trigger
pub struct WebhookTrigger {
    config: WebhookConfig,
    endpoint: String,
    auth: Option<WebhookAuth>,
}

#[async_trait]
impl TriggerAction for WebhookTrigger {
    async fn initialize(&mut self, ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        // Register webhook endpoint
        let endpoint_id = ctx.webhook_registry()
            .register_endpoint(&self.endpoint, ctx.workflow_id())
            .await?;
            
        Ok(TriggerHandle::Webhook(endpoint_id))
    }
    
    async fn listen(&mut self, handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let (tx, rx) = mpsc::channel(100);
        
        // Subscribe to webhook events
        let mut subscription = ctx.webhook_registry()
            .subscribe(handle.as_webhook_id()?)
            .await?;
            
        tokio::spawn(async move {
            while let Some(event) = subscription.next().await {
                if let Err(_) = tx.send(TriggerEvent::from(event)).await {
                    break;
                }
            }
        });
        
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
    
    async fn shutdown(&mut self, handle: TriggerHandle) -> Result<(), Error> {
        ctx.webhook_registry()
            .unregister_endpoint(handle.as_webhook_id()?)
            .await
    }
}

// Kafka Trigger
pub struct KafkaTrigger {
    config: KafkaConfig,
    consumer: Option<StreamConsumer>,
}

#[async_trait]
impl TriggerAction for KafkaTrigger {
    async fn initialize(&mut self, ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        let consumer = ClientConfig::new()
            .set("bootstrap.servers", &self.config.brokers)
            .set("group.id", &format!("nebula-{}", ctx.workflow_id()))
            .set("enable.auto.commit", "false")
            .create::<StreamConsumer>()?;
            
        consumer.subscribe(&[&self.config.topic])?;
        
        self.consumer = Some(consumer);
        
        Ok(TriggerHandle::Kafka {
            topic: self.config.topic.clone(),
            group_id: format!("nebula-{}", ctx.workflow_id()),
        })
    }
    
    async fn listen(&mut self, _handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let consumer = self.consumer.as_ref().ok_or(Error::NotInitialized)?;
        let stream = consumer.stream();
        
        let trigger_stream = stream.map(|result| {
            match result {
                Ok(message) => {
                    let payload = message.payload()
                        .map(|p| String::from_utf8_lossy(p).to_string())
                        .unwrap_or_default();
                        
                    Ok(TriggerEvent {
                        id: Uuid::new_v4(),
                        timestamp: Utc::now(),
                        data: json!({ "message": payload }),
                        metadata: Default::default(),
                    })
                }
                Err(e) => Err(Error::Kafka(e)),
            }
        });
        
        Ok(Box::pin(trigger_stream))
    }
}

// Scheduled Trigger
pub struct ScheduledTrigger {
    config: ScheduleConfig,
    schedule: Schedule,
}

#[async_trait]
impl TriggerAction for ScheduledTrigger {
    async fn initialize(&mut self, _ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        self.schedule = Schedule::from_str(&self.config.cron_expression)?;
        
        Ok(TriggerHandle::Schedule {
            expression: self.config.cron_expression.clone(),
        })
    }
    
    async fn listen(&mut self, _handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let schedule = self.schedule.clone();
        let (tx, rx) = mpsc::channel(10);
        
        tokio::spawn(async move {
            let mut next_time = schedule.upcoming(Utc).next().unwrap();
            
            loop {
                let now = Utc::now();
                if now >= next_time {
                    let event = TriggerEvent {
                        id: Uuid::new_v4(),
                        timestamp: now,
                        data: json!({ "scheduled_time": next_time }),
                        metadata: Default::default(),
                    };
                    
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                    
                    next_time = schedule.upcoming(Utc).next().unwrap();
                } else {
                    tokio::time::sleep_until(next_time.into()).await;
                }
            }
        });
        
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
```

### Event Processing

```rust
pub struct EventProcessor {
    event_bus: Arc<dyn EventBus>,
    handlers: Arc<RwLock<HashMap<String, Vec<Box<dyn EventHandler>>>>>,
    processor_threads: Vec<JoinHandle<()>>,
}

#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &RuntimeEvent) -> Result<(), Error>;
    fn event_type(&self) -> &str;
}

pub enum RuntimeEvent {
    TriggerFired {
        trigger_id: TriggerId,
        workflow_id: WorkflowId,
        event: TriggerEvent,
    },
    
    WorkflowDeployed {
        workflow_id: WorkflowId,
        version: Version,
    },
    
    WorkflowActivated {
        workflow_id: WorkflowId,
    },
    
    WorkflowDeactivated {
        workflow_id: WorkflowId,
    },
    
    TriggerFailed {
        trigger_id: TriggerId,
        error: Error,
    },
    
    RuntimeStarted {
        runtime_id: RuntimeId,
    },
    
    RuntimeStopping {
        runtime_id: RuntimeId,
    },
}

impl EventProcessor {
    pub async fn start(&self, num_threads: usize) -> Result<(), Error> {
        for i in 0..num_threads {
            let event_bus = self.event_bus.clone();
            let handlers = self.handlers.clone();
            
            let handle = tokio::spawn(async move {
                let mut subscription = event_bus.subscribe("runtime.*").await.unwrap();
                
                while let Some(event) = subscription.next().await {
                    if let Err(e) = Self::process_event(event, &handlers).await {
                        error!("Event processing error: {}", e);
                    }
                }
            });
            
            self.processor_threads.push(handle);
        }
        
        Ok(())
    }
    
    async fn process_event(
        event: RuntimeEvent,
        handlers: &Arc<RwLock<HashMap<String, Vec<Box<dyn EventHandler>>>>>,
    ) -> Result<(), Error> {
        let event_type = event.event_type();
        let handlers = handlers.read().await;
        
        if let Some(event_handlers) = handlers.get(event_type) {
            for handler in event_handlers {
                handler.handle(&event).await?;
            }
        }
        
        Ok(())
    }
}
```

### Workflow Coordination

```rust
pub struct WorkflowCoordinator {
    // Workflow assignments
    assignments: Arc<DashMap<WorkflowId, RuntimeId>>,
    
    // Coordination strategy
    strategy: Box<dyn CoordinationStrategy>,
    
    // Runtime registry
    runtime_registry: Arc<RuntimeRegistry>,
    
    // Load balancer
    load_balancer: Arc<LoadBalancer>,
}

#[async_trait]
pub trait CoordinationStrategy: Send + Sync {
    async fn assign_workflow(
        &self,
        workflow_id: &WorkflowId,
        runtimes: &[RuntimeInfo],
    ) -> Result<RuntimeId, Error>;
    
    async fn rebalance(
        &self,
        assignments: &HashMap<WorkflowId, RuntimeId>,
        runtimes: &[RuntimeInfo],
    ) -> HashMap<WorkflowId, RuntimeId>;
}

pub struct ConsistentHashStrategy {
    hasher: ConsistentHash<RuntimeId>,
}

impl WorkflowCoordinator {
    pub async fn assign_workflow(&self, workflow_id: &WorkflowId) -> Result<RuntimeId, Error> {
        // Get available runtimes
        let runtimes = self.runtime_registry.get_healthy_runtimes().await?;
        
        if runtimes.is_empty() {
            return Err(Error::NoAvailableRuntime);
        }
        
        // Use strategy to select runtime
        let runtime_id = self.strategy.assign_workflow(workflow_id, &runtimes).await?;
        
        // Store assignment
        self.assignments.insert(workflow_id.clone(), runtime_id.clone());
        
        // Notify runtime
        self.notify_runtime_of_assignment(&runtime_id, workflow_id).await?;
        
        Ok(runtime_id)
    }
    
    pub async fn handle_runtime_failure(&self, failed_runtime: &RuntimeId) -> Result<(), Error> {
        // Find affected workflows
        let affected_workflows: Vec<WorkflowId> = self.assignments
            .iter()
            .filter(|entry| entry.value() == failed_runtime)
            .map(|entry| entry.key().clone())
            .collect();
            
        // Reassign workflows
        for workflow_id in affected_workflows {
            self.reassign_workflow(&workflow_id).await?;
        }
        
        Ok(())
    }
}
```

### Health Monitoring

```rust
pub struct HealthMonitor {
    checks: Vec<Box<dyn HealthCheck>>,
    interval: Duration,
    status: Arc<RwLock<HealthStatus>>,
}

#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn check(&self) -> ComponentHealth;
    fn component_name(&self) -> &str;
}

pub struct ComponentHealth {
    pub status: HealthState,
    pub message: Option<String>,
    pub metrics: HashMap<String, f64>,
}

pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
}

impl HealthMonitor {
    pub async fn start(&self) {
        let checks = self.checks.clone();
        let status = self.status.clone();
        let interval = self.interval;
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                let mut overall_status = HealthState::Healthy;
                let mut component_results = HashMap::new();
                
                for check in &checks {
                    let result = check.check().await;
                    
                    match &result.status {
                        HealthState::Unhealthy => overall_status = HealthState::Unhealthy,
                        HealthState::Degraded if matches!(overall_status, HealthState::Healthy) => {
                            overall_status = HealthState::Degraded;
                        }
                        _ => {}
                    }
                    
                    component_results.insert(check.component_name().to_string(), result);
                }
                
                let health_status = HealthStatus {
                    status: overall_status,
                    components: component_results,
                    timestamp: Utc::now(),
                };
                
                *status.write().await = health_status;
            }
        });
    }
}
```

### Resource Management

```rust
pub struct ResourceManager {
    // Resource pools
    pools: HashMap<String, Box<dyn ResourcePool>>,
    
    // Resource limits
    limits: ResourceLimits,
    
    // Usage tracking
    usage: Arc<RwLock<ResourceUsage>>,
}

pub struct ResourceLimits {
    pub max_memory: usize,
    pub max_triggers: usize,
    pub max_connections: usize,
    pub max_cpu_percent: f64,
}

pub struct ResourceUsage {
    pub memory_used: usize,
    pub trigger_count: usize,
    pub connection_count: usize,
    pub cpu_percent: f64,
}

impl ResourceManager {
    pub async fn allocate_trigger_resources(
        &self,
        trigger_type: &TriggerType,
    ) -> Result<TriggerResources, Error> {
        // Check limits
        let usage = self.usage.read().await;
        
        if usage.trigger_count >= self.limits.max_triggers {
            return Err(Error::ResourceLimitExceeded("max_triggers"));
        }
        
        // Estimate resource requirements
        let requirements = self.estimate_trigger_requirements(trigger_type)?;
        
        if usage.memory_used + requirements.memory > self.limits.max_memory {
            return Err(Error::ResourceLimitExceeded("memory"));
        }
        
        // Allocate resources
        let resources = TriggerResources {
            memory_limit: requirements.memory,
            connection_pool: self.get_connection_pool(trigger_type)?,
            rate_limiter: self.create_rate_limiter(trigger_type)?,
        };
        
        // Update usage
        self.usage.write().await.trigger_count += 1;
        self.usage.write().await.memory_used += requirements.memory;
        
        Ok(resources)
    }
}
```

## Runtime Lifecycle

### Startup Process

```rust
impl Runtime {
    pub async fn start(config: RuntimeConfig) -> Result<Self, Error> {
        info!("Starting runtime {}", config.id);
        
        // Initialize components
        let trigger_manager = Arc::new(TriggerManager::new(&config.trigger_config).await?);
        let event_processor = Arc::new(EventProcessor::new(&config.event_bus_config).await?);
        let coordinator = Arc::new(WorkflowCoordinator::new(&config.coordination_config).await?);
        let health_monitor = Arc::new(HealthMonitor::new());
        let resource_manager = Arc::new(ResourceManager::new(config.resource_limits));
        let metrics = Arc::new(RuntimeMetrics::new());
        
        let runtime = Self {
            id: config.id,
            trigger_manager,
            event_processor,
            coordinator,
            health_monitor,
            resource_manager,
            metrics,
        };
        
        // Start components
        runtime.event_processor.start(4).await?;
        runtime.health_monitor.start().await;
        
        // Register with coordinator
        runtime.coordinator.register_runtime(&runtime.id).await?;
        
        // Load assigned workflows
        runtime.load_assigned_workflows().await?;
        
        // Emit started event
        runtime.event_processor.publish(RuntimeEvent::RuntimeStarted {
            runtime_id: runtime.id.clone(),
        }).await?;
        
        Ok(runtime)
    }
    
    async fn load_assigned_workflows(&self) -> Result<(), Error> {
        let assignments = self.coordinator.get_runtime_assignments(&self.id).await?;
        
        for workflow_id in assignments {
            if let Err(e) = self.activate_workflow(&workflow_id).await {
                error!("Failed to activate workflow {}: {}", workflow_id, e);
            }
        }
        
        Ok(())
    }
}
```

### Shutdown Process

```rust
impl Runtime {
    pub async fn shutdown(&self) -> Result<(), Error> {
        info!("Shutting down runtime {}", self.id);
        
        // Emit stopping event
        self.event_processor.publish(RuntimeEvent::RuntimeStopping {
            runtime_id: self.id.clone(),
        }).await?;
        
        // Stop accepting new workflows
        self.coordinator.mark_runtime_draining(&self.id).await?;
        
        // Deactivate all triggers
        let workflows = self.get_active_workflows().await?;
        for workflow_id in workflows {
            self.deactivate_workflow(&workflow_id).await?;
        }
        
        // Stop components
        self.event_processor.stop().await?;
        self.health_monitor.stop().await?;
        
        // Unregister from coordinator
        self.coordinator.unregister_runtime(&self.id).await?;
        
        info!("Runtime {} shutdown complete", self.id);
        
        Ok(())
    }
}
```

## Metrics

```rust
pub struct RuntimeMetrics {
    // Workflow metrics
    workflows_active: Gauge,
    workflows_activated: Counter,
    workflows_deactivated: Counter,
    
    // Trigger metrics
    triggers_active: Gauge,
    triggers_fired: Counter,
    trigger_errors: Counter,
    trigger_latency: Histogram,
    
    // Event metrics
    events_processed: Counter,
    event_processing_duration: Histogram,
    
    // Resource metrics
    memory_usage: Gauge,
    cpu_usage: Gauge,
    connection_pool_size: Gauge,
}

impl RuntimeMetrics {
    pub fn record_trigger_fired(&self, trigger_type: &str, latency: Duration) {
        self.triggers_fired
            .with_label_values(&[trigger_type])
            .increment();
            
        self.trigger_latency
            .with_label_values(&[trigger_type])
            .record(latency.as_secs_f64());
    }
    
    pub fn record_workflow_activated(&self, workflow_id: &WorkflowId) {
        self.workflows_activated.increment();
        self.workflows_active.increment();
        
        debug!("Workflow {} activated", workflow_id);
    }
}
```