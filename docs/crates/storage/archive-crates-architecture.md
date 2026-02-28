# Archived From "docs/archive/crates-architecture.md"

## 7. nebula-storage & nebula-storage-postgres

**Purpose**: Storage abstraction and PostgreSQL implementation.

```rust
// nebula-storage/src/lib.rs
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error>;
    async fn load_workflow(&self, id: &WorkflowId) -> Result<Workflow, Error>;
    async fn list_workflows(&self, filter: WorkflowFilter) -> Result<Vec<WorkflowSummary>, Error>;
    
    async fn save_execution(&self, execution: &ExecutionState) -> Result<(), Error>;
    async fn load_execution(&self, id: &ExecutionId) -> Result<ExecutionState, Error>;
    async fn update_execution_status(&self, id: &ExecutionId, status: ExecutionStatus) -> Result<(), Error>;
}

// nebula-storage-postgres/src/lib.rs
use sqlx::{PgPool, postgres::PgPoolOptions};

pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    pub async fn new(database_url: &str) -> Result<Self, Error> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;
            
        Ok(Self { pool })
    }
    
    pub async fn migrate(&self) -> Result<(), Error> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for PostgresStorage {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), Error> {
        let workflow_json = serde_json::to_value(workflow)?;
        
        sqlx::query!(
            r#"
            INSERT INTO workflows (id, name, version, definition, created_at, updated_at)
            VALUES ($1, $2, $3, $4, NOW(), NOW())
            ON CONFLICT (id) DO UPDATE SET
                definition = EXCLUDED.definition,
                updated_at = NOW()
            "#,
            workflow.id.as_str(),
            workflow.name,
            workflow.version.to_string(),
            workflow_json
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    // Other implementations...
}
```

