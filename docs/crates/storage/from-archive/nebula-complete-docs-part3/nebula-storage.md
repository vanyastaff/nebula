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

