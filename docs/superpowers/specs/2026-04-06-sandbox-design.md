# Nebula Sandbox — Design Spec

## Goal

Define the phased isolation model for action execution: from in-process capability checks (Phase 1) through WASM isolation (Phase 2) to Firecracker microVMs (Phase 3). The `SandboxRunner` trait is the stable abstraction that all phases implement.

## Philosophy

- **One trait, multiple backends.** `SandboxRunner` is the interface. InProcess, WASM, Firecracker are implementations.
- **Capability-based, not blanket isolation.** Actions declare what they need. Sandbox enforces they only access declared capabilities.
- **Phase 1 = correctness, not security.** In-process sandbox prevents ACCIDENTAL undeclared access. Real security isolation starts at Phase 2 (WASM).
- **Progressive:** First-party trusted actions run unboxed. Third-party actions run sandboxed. Untrusted actions run in VM.

---

## 1. IsolationLevel — Declared on ActionMetadata

```rust
/// How isolated this action's execution should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum IsolationLevel {
    /// No isolation. Action runs directly in engine process.
    /// For first-party trusted code only.
    None,

    /// Capability-gated in-process. SandboxedContext wraps ActionContext
    /// and denies undeclared resource/credential access.
    /// Correctness check, NOT a security boundary.
    CapabilityGated,

    /// Full isolation via WASM or microVM.
    /// Action cannot access host memory, filesystem, or network
    /// except through declared capabilities.
    Isolated,
}
```

Default: `IsolationLevel::None` for first-party actions (Essential 50).
Default: `IsolationLevel::CapabilityGated` for verified community plugins.
Default: `IsolationLevel::Isolated` for untrusted/unreviewed plugins.

---

## 2. SandboxRunner Trait (stable across all phases)

```rust
/// Abstraction for action execution isolation.
/// Implementations provide different isolation guarantees.
pub trait SandboxRunner: Send + Sync {
    fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> + Send;

    /// What isolation level this runner provides.
    fn isolation_level(&self) -> IsolationLevel;
}
```

---

## 3. Phase 1 — InProcessSandbox (Capability-Gated)

**Status:** Trait exists, routing TODO. This spec completes it.

### SandboxedContext — wrapper that enforces capabilities

```rust
/// Wraps ActionContext with capability checks.
/// Denies access to undeclared credentials and resources.
pub struct SandboxedContext {
    inner: ActionContext,
    allowed_credentials: HashSet<CredentialKey>,
    allowed_resources: HashSet<ResourceKey>,
}

impl SandboxedContext {
    pub fn new(ctx: ActionContext, metadata: &ActionMetadata) -> Self {
        Self {
            inner: ctx,
            allowed_credentials: metadata.dependencies.credential_keys().into_iter().collect(),
            allowed_resources: metadata.dependencies.resource_keys().into_iter().collect(),
        }
    }

    /// Credential access — checks against declared dependencies.
    pub fn credential<S: AuthScheme>(&self, key: &str) -> Result<S, ActionError> {
        let cred_key = CredentialKey::new(key)
            .map_err(|_| ActionError::sandbox_violation(format!("invalid credential key: {key}")))?;
        if !self.allowed_credentials.contains(&cred_key) {
            return Err(ActionError::sandbox_violation(
                format!("action did not declare credential dependency: {key}")
            ));
        }
        self.inner.credential::<S>(key)
    }

    /// Resource access — checks against declared dependencies.
    pub fn resource<R: Resource>(&self, key: &str) -> Result<R::Lease, ActionError> {
        let res_key = ResourceKey::new(key)
            .map_err(|_| ActionError::sandbox_violation(format!("invalid resource key: {key}")))?;
        if !self.allowed_resources.contains(&res_key) {
            return Err(ActionError::sandbox_violation(
                format!("action did not declare resource dependency: {key}")
            ));
        }
        self.inner.resource::<R>(key)
    }

    /// Input data — always allowed.
    pub fn input_data(&self) -> &Value { self.inner.input_data() }

    /// Execution identity — always allowed.
    pub fn execution_id(&self) -> &ExecutionId { self.inner.execution_id() }
    pub fn node_id(&self) -> &NodeId { self.inner.node_id() }

    /// Cancellation — always allowed.
    pub fn cancellation(&self) -> &CancellationToken { self.inner.cancellation() }
}
```

### InProcessSandbox implementation

```rust
pub struct InProcessSandbox {
    executor: ActionExecutor,
}

impl SandboxRunner for InProcessSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: Value,
    ) -> Result<ActionResult<Value>, ActionError> {
        // Check cancellation before execution
        if context.cancellation().is_cancelled() {
            return Err(ActionError::cancelled());
        }
        // Execute with capability-gated context
        (self.executor)(context, metadata, input).await
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::CapabilityGated
    }
}
```

### Runtime routing

```rust
// In ActionRuntime::execute_action:
let result = match metadata.isolation_level {
    IsolationLevel::None => {
        // Direct execution — no sandbox
        handler.execute(input, &ctx).await
    }
    IsolationLevel::CapabilityGated => {
        // Wrap context with capability checks
        let sandboxed = SandboxedContext::new(ctx, metadata);
        self.sandbox.execute(sandboxed, metadata, input).await
    }
    IsolationLevel::Isolated => {
        // Full isolation — WASM or microVM
        self.isolated_sandbox
            .as_ref()
            .ok_or(RuntimeError::no_isolated_sandbox())?
            .execute(SandboxedContext::new(ctx, metadata), metadata, input)
            .await
    }
};
```

### What Phase 1 DOES protect against
- Action accessing a credential it didn't declare (returns SandboxViolation)
- Action accessing a resource it didn't declare (returns SandboxViolation)
- Accidental capability creep (action works in dev with None, fails in prod with CapabilityGated → catches the undeclared dependency)

### What Phase 1 does NOT protect against (acknowledged)
- Malicious code reading arbitrary memory (same address space)
- CPU/memory exhaustion (no resource limits)
- Filesystem/network access (no OS-level restriction)
- `unsafe` code bypassing the Rust API boundary

---

## 4. Phase 2 — WASM Sandbox

### WasmSandbox implementation

```rust
pub struct WasmSandbox {
    engine: wasmtime::Engine,
    linker: wasmtime::Linker<ActionState>,
}

impl SandboxRunner for WasmSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: Value,
    ) -> Result<ActionResult<Value>, ActionError> {
        // Load WASM module for this action
        let module = self.load_module(&metadata.key)?;

        // Create WASM instance with capability imports
        let mut store = wasmtime::Store::new(&self.engine, ActionState::new(context));

        // Only link capabilities the action declared
        let instance = self.linker.instantiate_async(&mut store, &module).await?;

        // Call the action's execute function
        let execute = instance.get_typed_func::<(i32, i32), i32>(&mut store, "execute")?;

        // Pass input as JSON bytes in WASM memory
        let input_ptr = self.write_to_wasm(&mut store, &instance, &input)?;
        let output_ptr = execute.call_async(&mut store, input_ptr).await?;

        // Read output from WASM memory
        let output = self.read_from_wasm(&store, &instance, output_ptr)?;
        Ok(output)
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Isolated
    }
}
```

### WASM capabilities as host imports

```rust
// Only linked if action declares the credential dependency:
fn credential_resolve(store: &mut Store, key_ptr: i32, key_len: i32) -> i32 {
    let key = read_string(store, key_ptr, key_len);
    let result = store.data().context.credential_raw(&key);
    write_json(store, &result)
}

// Only linked if action declares the resource dependency:
fn resource_acquire(store: &mut Store, key_ptr: i32, key_len: i32) -> i32 {
    // ...
}

// Always available:
fn log_message(store: &mut Store, level: i32, msg_ptr: i32, msg_len: i32) { }
fn get_input(store: &mut Store) -> i32 { }
```

Undeclared capabilities → function not linked → WASM trap on call → ActionError::SandboxViolation.

### WASM resource limits

```rust
let mut config = wasmtime::Config::new();
config.max_wasm_stack(1024 * 1024);           // 1MB stack
config.memory_limit(128 * 1024 * 1024);        // 128MB heap
config.fuel_async(true);                        // CPU budget via fuel
config.epoch_interruption(true);                // timeout via epoch
```

---

## 5. Phase 3 — Firecracker MicroVM (from breakthrough #9)

```rust
pub struct FirecrackerSandbox {
    vm_pool: VmPool,
    timeout: Duration,
}

impl SandboxRunner for FirecrackerSandbox {
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: Value,
    ) -> Result<ActionResult<Value>, ActionError> {
        let vm = self.vm_pool.acquire(metadata).await?;
        let payload = serde_json::to_vec(&input)?;
        vm.vsock_send(AGENT_PORT, &payload).await?;
        let output = tokio::time::timeout(self.timeout, vm.vsock_recv(AGENT_PORT))
            .await
            .map_err(|_| ActionError::Timeout)?
            .map_err(|e| ActionError::Sandbox(e.to_string()))?;
        vm.release().await;
        serde_json::from_slice(&output).map_err(|e| ActionError::Sandbox(e.to_string()))
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Isolated
    }
}
```

---

## 6. Sandbox Selection Logic

```rust
// Engine configures available sandboxes:
pub struct SandboxConfig {
    /// For IsolationLevel::CapabilityGated
    pub in_process: InProcessSandbox,
    /// For IsolationLevel::Isolated (optional — not all deployments have WASM/VM)
    pub isolated: Option<Box<dyn SandboxRunner>>,
}
```

If action requires `Isolated` but no isolated sandbox configured → `RuntimeError::NoIsolatedSandbox`.

---

## 7. Integration with Plugin Ecosystem

| Plugin tier | Default IsolationLevel |
|-------------|----------------------|
| **Essential 50** (core team maintained) | None |
| **Official** (under nebula-plugins/ org) | CapabilityGated |
| **Community** (unreviewed) | Isolated (when WASM lands) |
| **First-party** (user's own code) | Configurable |

Plugin manifest declares maximum isolation level:
```toml
[plugin]
isolation = "capability_gated"  # or "none", "isolated"
```

Engine can UPGRADE isolation (CapabilityGated → Isolated) but never DOWNGRADE (Isolated → None).

---

## 8. Implementation Phases

| Phase | What | When |
|-------|------|------|
| 1 | SandboxedContext + InProcessSandbox routing by IsolationLevel | v1 |
| 2 | WasmSandbox via wasmtime + host import capabilities | v2 |
| 3 | FirecrackerSandbox for maximum isolation | v3 (optional) |
| - | USDT probes on sandbox entry/exit (RT17) | v1.1 |

---

## 9. Not In Scope

- Network policy enforcement (allow/deny outbound connections) — OS-level, not sandbox
- Filesystem access control — WASM has no filesystem by default; Firecracker has read-only rootfs
- GPU passthrough in sandbox — Phase 3+
- Sandbox performance benchmarking — after implementation
