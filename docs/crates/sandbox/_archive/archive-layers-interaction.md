# Archived From "docs/archive/layers-interaction.md"

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

