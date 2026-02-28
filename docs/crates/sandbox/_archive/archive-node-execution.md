# Archived From "docs/archive/node-execution.md"

### nebula-sandbox
**Назначение:** Изоляция выполнения Actions с capability-based моделью безопасности.

**Ключевые концепции:**
- Capability-based access control — Action декларирует что ему нужно
- Pluggable isolation strategies (in-process, WASM, process)
- Resource и network ограничения
- Audit trail для security events

```rust
/// Уровень изоляции при выполнении Action
pub enum IsolationLevel {
    /// Доверенный код — internal/builtin actions, без ограничений
    None,
    /// Capability-based ограничения в рамках текущего процесса.
    /// Sandbox проверяет capabilities перед каждым вызовом ресурса/credential.
    Lightweight,
    /// Полная изоляция — WASM runtime или отдельный процесс.
    /// Для third-party и community actions.
    Full,
}

/// Capability — единица доступа, запрашиваемая Action
pub enum Capability {
    /// Доступ к сети с ограничением по хостам
    Network { allowed_hosts: Vec<String> },
    /// Доступ к файловой системе
    FileSystem { paths: Vec<PathBuf>, read_only: bool },
    /// Доступ к конкретному Resource
    Resource(ResourceId),
    /// Доступ к конкретному Credential
    Credential(CredentialId),
    /// Лимит потребления памяти
    MaxMemory(usize),
    /// Лимит CPU-времени
    MaxCpuTime(Duration),
    /// Доступ к environment variables
    Environment { keys: Vec<String> },
}

/// Декларация capabilities на уровне Action
#[derive(Action)]
#[action(id = "slack.send_message")]
#[sandbox(
    isolation = "lightweight",
    capabilities = [
        Network { allowed_hosts: ["slack.com", "api.slack.com"] },
        Credential("slack_token"),
        MaxCpuTime("30s"),
        MaxMemory("128MB"),
    ]
)]
pub struct SlackSendAction;

/// Trait для pluggable sandbox implementations
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Проверяет, разрешено ли выполнение Action с данными capabilities
    fn check_capabilities(
        &self,
        action: &ActionMetadata,
        requested: &[Capability],
        granted: &[Capability],
    ) -> Result<(), SandboxViolation>;

    /// Выполняет Action в изолированной среде
    async fn execute(
        &self,
        action: &dyn Action,
        context: SandboxedContext,
    ) -> Result<serde_json::Value, SandboxError>;
}

/// SandboxedContext — урезанная версия ActionContext,
/// которая проксирует вызовы через capability checks
pub struct SandboxedContext {
    inner: ActionContext,
    granted_capabilities: Vec<Capability>,
    violation_handler: Arc<dyn ViolationHandler>,
}

impl SandboxedContext {
    /// Доступ к ресурсу — только если есть Capability::Resource
    pub async fn get_resource<R: Resource>(&self) -> Result<R::Instance> {
        let resource_id = R::resource_id();
        self.check_capability(&Capability::Resource(resource_id.clone()))?;
        self.inner.get_resource::<R>().await
    }

    /// Доступ к credential — только если есть Capability::Credential
    pub async fn get_credential(&self, id: &str) -> Result<AuthData> {
        self.check_capability(&Capability::Credential(CredentialId::new(id)))?;
        self.inner.get_credential(id).await
    }

    fn check_capability(&self, required: &Capability) -> Result<(), SandboxViolation> {
        if !self.granted_capabilities.iter().any(|c| c.satisfies(required)) {
            let violation = SandboxViolation {
                action_id: self.inner.action_id().clone(),
                required: required.clone(),
                message: format!("Action lacks capability: {:?}", required),
            };
            self.violation_handler.on_violation(&violation);
            return Err(violation);
        }
        Ok(())
    }
}

/// Встроенные реализации Sandbox
pub struct InProcessSandbox {
    capability_checker: CapabilityChecker,
    resource_limiter: ResourceLimiter,
}

pub struct WasmSandbox {
    engine: wasmtime::Engine,
    memory_limit: usize,
    fuel_limit: u64,  // CPU limiter
}

/// Конфигурация sandbox через config
/*
[sandbox]
default_isolation = "lightweight"
max_memory_mb = 256
max_cpu_time = "60s"

[sandbox.trusted_actions]
patterns = ["builtin.*", "internal.*"]
isolation = "none"

[sandbox.community_actions]
patterns = ["community.*", "hub.*"]
isolation = "full"
*/
```

---

