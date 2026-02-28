# Archived From "docs/archive/layers-interaction.md"

### 4. nebula-action ↔ nebula-resource ↔ nebula-credential

**Цепочка:** Action запрашивает Resource, который может требовать Credential

```rust
// nebula-action определяет что нужно
#[derive(Action)]
#[resources([DatabaseResource])]
#[credentials(["database"])]
pub struct QueryUserAction;

impl ProcessAction for QueryUserAction {
    async fn execute(&self, input: Input, ctx: &ActionContext) -> Result<Output> {
        // Action запрашивает resource
        let db = ctx.get_resource::<DatabaseResource>().await?;
        
        // Resource внутри использует credential
        let users = db.query("SELECT * FROM users").await?;
        Ok(users)
    }
}

// nebula-resource создает ресурс с credential
pub struct DatabaseResource;

impl Resource for DatabaseResource {
    type Instance = DatabaseInstance;
    
    async fn create(&self, ctx: &ResourceContext) -> Result<Self::Instance> {
        // Resource запрашивает credential из контекста
        let cred = ctx.get_credential("database").await?;
        
        // Используем credential для создания подключения
        let connection = match cred {
            AuthData::Basic { username, password } => {
                let conn_string = format!(
                    "postgres://{}:{}@localhost/db",
                    username,
                    password.expose_secret()
                );
                PgConnection::connect(&conn_string).await?
            }
            _ => return Err(ResourceError::InvalidCredentialType),
        };
        
        Ok(DatabaseInstance { connection })
    }
}

// ActionContext связывает все вместе
impl ActionContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance> {
        // Определяем scope
        let scope = self.determine_resource_scope::<R>();
        
        // ResourceManager проверяет, нужен ли credential
        let required_creds = R::required_credentials();
        
        // Создаем ResourceContext с доступом к credentials
        let resource_ctx = ResourceContext {
            scope,
            credential_resolver: Box::new(move |cred_id| {
                self.execution_context.get_credential(cred_id)
            }),
        };
        
        // ResourceManager создает или возвращает существующий
        self.resource_manager.get_or_create::<R>(resource_ctx).await
    }
}
```

