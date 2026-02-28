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

---

### 8. nebula-sandbox ↔ nebula-runtime ↔ nebula-action

**Паттерн:** Runtime делегирует выполнение Action в Sandbox, который контролирует доступ через capabilities

```rust
// nebula-runtime использует sandbox для выполнения actions
impl ActionRuntime {
    pub async fn execute_action(
        &self,
        action_id: &ActionId,
        context: ActionContext,
    ) -> Result<ActionResult> {
        let action = self.action_registry.get(action_id)?;
        let metadata = action.metadata();
        
        // Определяем уровень изоляции из metadata или config
        let isolation = self.resolve_isolation_level(metadata);
        
        match isolation {
            IsolationLevel::None => {
                // Builtin actions — выполняем напрямую
                action.execute(context).await
            }
            _ => {
                // Создаем sandboxed context — проксирует вызовы через capability checks
                let sandboxed = SandboxedContext::new(
                    context,
                    metadata.capabilities.clone(),
                );
                self.sandbox.execute(action.as_ref(), sandboxed).await
            }
        }
    }
}

// nebula-sandbox проверяет capabilities при каждом обращении к ресурсу
impl SandboxedContext {
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance> {
        // Проверяем capability перед делегированием
        let resource_id = R::resource_id();
        self.check_capability(&Capability::Resource(resource_id))?;
        
        // Делегируем в inner ActionContext
        self.inner.get_resource::<R>().await
    }
    
    pub async fn get_credential(&self, id: &str) -> Result<AuthData> {
        self.check_capability(&Capability::Credential(CredentialId::new(id)))?;
        self.inner.get_credential(id).await
    }
}

// nebula-action декларирует capabilities через derive
#[derive(Action)]
#[action(id = "external.api_call")]
#[sandbox(
    isolation = "lightweight",
    capabilities = [
        Network { allowed_hosts: ["api.example.com"] },
        Credential("api_key"),
        MaxCpuTime("30s"),
    ]
)]
pub struct ExternalApiAction;

// При попытке обратиться к не-декларированному ресурсу — SandboxViolation
impl ProcessAction for ExternalApiAction {
    async fn execute(&self, input: Input, ctx: &SandboxedContext) -> Result<Output> {
        // OK — credential "api_key" декларирован
        let key = ctx.get_credential("api_key").await?;
        
        // FAIL — DatabaseResource не в capabilities → SandboxViolation
        // let db = ctx.get_resource::<DatabaseResource>().await?;
        
        let client = HttpClient::new(key);
        let result = client.get(&input.url).await?;
        Ok(result)
    }
}
```

