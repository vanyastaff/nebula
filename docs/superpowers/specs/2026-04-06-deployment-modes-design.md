# Nebula Deployment Modes — Architecture Spec

## Goal

Define how Local Desktop, Self-Hosted Server, and SaaS Cloud modes are architecturally separated WITHOUT introducing a `DeploymentMode` enum. Mode is an emergent property of which components `main()` wires together.

## Philosophy

- **Capabilities, not modes.** Inspired by n8n's lesson: don't `if mode == Cloud`, compose capabilities.
- **Binary-level separation.** Desktop and server are separate Cargo binary targets with different dependency trees.
- **Runtime config for behavioral differences.** Same crate, different config → different behavior.
- **No mode enum in any library crate.** Crates don't know which mode they're in. They receive injected dependencies and work.

## Research Basis

| Project | Approach | Lesson |
|---------|----------|--------|
| n8n | Runtime env var `N8N_DEPLOYMENT_TYPE` | "Hundreds of `if cloud` scattered everywhere. Use capability sets, not mode enums." |
| Windmill | Single binary + license key | "Feature flags in 40 files in one month. License check is simpler." |
| Temporal | Separate codebases, shared libs | Works for multi-org, overkill for single team |
| Supabase | Service composition (OSS + cloud containers) | "Deployment topology IS the mode" |
| GitLab | `ee/` directory + license check | "Legible to lawyers, auditors, and new hires" |

**Nebula choice: Supabase model (service composition) + Windmill simplicity (single codebase, binary selection).**

---

## 1. Three Binary Targets, One Codebase

```
nebula/
├── apps/
│   ├── desktop/          ← Binary 1: Tauri desktop app
│   │   ├── Cargo.toml    (depends on: engine, storage[sqlite], credential, expression, ...)
│   │   └── src/main.rs   (wires: SQLite + LocalMetrics + LocalEventStore + no API)
│   │
│   ├── server/           ← Binary 2: Self-hosted server
│   │   ├── Cargo.toml    (depends on: engine, storage[postgres], api, auth, webhook, ...)
│   │   └── src/main.rs   (wires: Postgres + Prometheus + API + Auth + ScopeLayer)
│   │
│   └── cloud/            ← Binary 3: SaaS (extends server)
│       ├── Cargo.toml    (depends on: server + billing + multi-tenant + managed-creds)
│       └── src/main.rs   (wires: everything from server + Billing + Sentry always-on)
│
├── crates/               ← Library crates (mode-independent)
│   ├── core/
│   ├── engine/
│   ├── storage/          (features: sqlite, postgres, redis, s3)
│   ├── credential/
│   ├── ...
```

**Desktop binary** doesn't compile `nebula-api`, auth subsystem code, or Postgres driver.
**Server binary** doesn't compile Tauri, desktop UI code.
**Cloud binary** extends server with billing, managed credentials, Sentry always-on.

Dead code physically absent — not gated, not compiled, not in binary.

---

## 2. Crate Classification

### Mode-Independent (20 crates — no changes ever)

These crates receive injected dependencies and DO NOT know about deployment modes:

```
core, validator, parameter, expression, memory, workflow, execution,
error, eventbus, resilience, system, resource, action, plugin, sdk,
engine, runtime, metrics, telemetry, all macros crates
```

**Invariant: ZERO `if mode ==` or `#[cfg(feature = "cloud")]` in any of these.**

### Mode-Aware (5 crates — behavior via config/DI)

| Crate | What changes | Mechanism |
|-------|-------------|-----------|
| **storage** | Backend: SQLite vs Postgres vs S3 | Cargo features (`sqlite`, `postgres`) |
| **config** | Source priority, defaults | Different config file per binary |
| **log** | Output: file vs stdout/JSON vs OTLP | `LogConfig` sink selection |
| **telemetry** | Export: none vs Prometheus vs OTLP | `TelemetryProfile` in config |
| **credential** | Layers: local keychain vs encrypted store + scope + cache | `CredentialStack` builder composition |

**These crates are configured, not branched.** They don't check a mode — they receive a config struct.

### Binary-Only (exist in some binaries, not others)

| Component | Desktop | Server | Cloud |
|-----------|---------|--------|-------|
| **nebula-api** (HTTP server) | ❌ | ✅ | ✅ |
| **Auth subsystem** (JWT/session) | ❌ | ✅ | ✅ |
| **nebula-webhook** | ❌ | ✅ | ✅ |
| **Tauri IPC** | ✅ | ❌ | ❌ |
| **BillingCollector** | ❌ | ❌ | ✅ |
| **Managed credential vault** | ❌ | ❌ | ✅ |
| **Multi-tenant enforcement** | ❌ | Optional | ✅ |

---

## 3. How main() Wires Each Mode

### Desktop main()

```rust
// apps/desktop/src/main.rs
fn main() {
    let config = Config::load("nebula-desktop.toml");

    // Storage: embedded SQLite (or libSQL)
    let storage = SqliteStorage::new(&config.data_dir).await;
    let workflow_repo = storage.workflow_repo();
    let execution_repo = storage.execution_repo();

    // Credentials: local keychain, no scope layer, no encryption layer
    let credential_store = LocalKeychainStore::new();
    let credential_resolver = CredentialResolver::new(credential_store);

    // Telemetry: local only, no export
    let metrics = LocalMetricsStore::new();
    let event_store = LocalEventStore::new(&config.data_dir);

    // Sentry: opt-in
    let _sentry = if config.crash_reports_enabled {
        Some(init_sentry_opt_in())
    } else { None };

    // Engine: same as server, different injected deps
    let runtime = ActionRuntime::new(registry, sandbox, policy, metrics.registry());
    let engine = WorkflowEngine::new(runtime, workflow_repo, execution_repo);

    // NO API server. NO auth. NO webhook listener.
    // Desktop communicates via Tauri IPC commands.
    tauri::Builder::default()
        .manage(engine)
        .manage(credential_resolver)
        .manage(metrics)
        .manage(event_store)
        .invoke_handler(tauri::generate_handler![
            cmd_execute_workflow,
            cmd_list_workflows,
            cmd_get_execution,
            // ...
        ])
        .run(tauri::generate_context!());
}
```

### Server main()

```rust
// apps/server/src/main.rs
#[tokio::main]
async fn main() {
    let config = Config::load("nebula-server.toml");

    // Storage: Postgres
    let pool = PgPool::connect(&config.database_url).await;
    let workflow_repo = PgWorkflowRepo::new(pool.clone());
    let execution_repo = PgExecutionRepo::new(pool.clone());

    // Credentials: encrypted store + cache + scope + audit
    let credential_store = CredentialStack::builder()
        .backend(PgCredentialStore::new(pool.clone()))
        .encryption(EncryptionLayer::new(&config.encryption_key))
        .cache(CacheLayer::new(config.credential_cache))
        .scope(ScopeLayer::new())  // multi-tenant filtering
        .audit(AuditLayer::new())
        .build();
    let credential_resolver = CredentialResolver::new(credential_store);

    // Telemetry: Prometheus + optional OTEL
    let metrics = MetricsRegistry::new();
    let _prometheus = PrometheusExporter::new(metrics.clone(), config.prometheus);
    let _otel = config.otel.as_ref().map(|c| OtelExporter::new(metrics.clone(), c));

    // Sentry: off by default, configurable
    let _sentry = config.sentry_dsn.map(|dsn| init_sentry(dsn));

    // Engine
    let runtime = ActionRuntime::new(registry, sandbox, policy, metrics.clone());
    let engine = WorkflowEngine::new(runtime, workflow_repo.clone(), execution_repo.clone());

    // API server
    let app = nebula_api::create_app(
        engine,
        credential_resolver,
        workflow_repo,
        execution_repo,
        config.auth,
    );

    // Webhook listener
    let webhook_handler = WebhookHandler::new(queue_backend);

    axum::serve(listener, app).await;
}
```

### Cloud main()

```rust
// apps/cloud/src/main.rs
// Extends server with billing + managed credentials + always-on Sentry
#[tokio::main]
async fn main() {
    let config = Config::load("nebula-cloud.toml");

    // Everything from server, PLUS:

    // Billing: per-tenant usage tracking
    let billing = BillingCollector::new(config.billing);

    // Managed credentials: KMS-backed encryption
    let credential_store = CredentialStack::builder()
        .backend(PgCredentialStore::new(pool.clone()))
        .encryption(KmsEncryptionLayer::new(&config.kms))  // Cloud KMS, not local key
        .cache(CacheLayer::new(config.credential_cache))
        .scope(ScopeLayer::new())
        .audit(AuditLayer::new())
        .build();

    // Sentry: always on
    let _sentry = init_sentry(&config.sentry_dsn);

    // Execution history: ClickHouse
    let history_writer = ExecutionHistoryWriter::new(config.clickhouse);

    // Engine with billing hooks
    let engine = WorkflowEngine::builder()
        .runtime(runtime)
        .workflow_repo(workflow_repo)
        .execution_repo(execution_repo)
        .on_node_complete(move |exec_id, node_id, duration, bytes| {
            billing.record_node_execution(&owner_id, duration, bytes);
            history_writer.record_node_output(/* ... */);
        })
        .build();

    // Same API, same webhook handler, different injected deps
    axum::serve(listener, app).await;
}
```

---

## 4. Config Files Per Binary

### nebula-desktop.toml
```toml
[storage]
backend = "sqlite"
path = "~/.nebula/data.db"

[telemetry]
profile = "local"
crash_reports = true  # user can toggle

[credential]
backend = "local_keychain"

# NO [api], [auth], [webhook], [billing] sections
```

### nebula-server.toml
```toml
[storage]
backend = "postgres"
url = "postgresql://localhost/nebula"

[telemetry]
profile = "self_hosted"
prometheus = { enabled = true, endpoint = "0.0.0.0:9090" }
otel = { endpoint = "http://localhost:4317" }
logs = { level = "info", format = "json" }

[credential]
backend = "postgres"
encryption_key = "${NEBULA_ENCRYPTION_KEY}"

[api]
port = 5678
jwt_secret = "${NEBULA_JWT_SECRET}"

[webhook]
enabled = true
queue_backend = "postgres"

# NO [billing] section
```

### nebula-cloud.toml
```toml
[storage]
backend = "postgres"
url = "${DATABASE_URL}"
read_replica = "${DATABASE_READ_URL}"

[telemetry]
profile = "cloud"
otel = { endpoint = "${OTEL_ENDPOINT}" }
sentry = { dsn = "${SENTRY_DSN}" }

[credential]
backend = "postgres"
encryption = "kms"
kms_key_id = "${KMS_KEY_ID}"

[api]
port = 5678
jwt_secret = "${JWT_SECRET}"

[webhook]
enabled = true
queue_backend = "postgres"

[billing]
enabled = true
endpoint = "${BILLING_API_URL}"
clickhouse = { url = "${CLICKHOUSE_URL}" }
```

---

## 5. CI: Both Binaries Must Compile

```yaml
# .github/workflows/ci.yml
jobs:
  check-desktop:
    runs-on: ubuntu-latest
    steps:
      - run: cargo check -p nebula-desktop

  check-server:
    runs-on: ubuntu-latest
    steps:
      - run: cargo check -p nebula-server

  # If a library crate change breaks either binary → CI fails
  # This catches accidental mode-specific dependencies
```

A contributor adding `use crate::auth::*` inside `nebula-engine` → desktop build fails → PR blocked. The dependency tree IS the mode enforcement.

---

## 6. How Contributors Know What's Mode-Specific

| Question | Answer |
|----------|--------|
| "Is this code cloud-only?" | Is it in `apps/cloud/`? Then yes. Is it in `crates/`? Then no. |
| "Will my change break desktop?" | CI builds both. If desktop fails, fix it. |
| "Can I use auth subsystem code in nebula-engine?" | No. Engine is in `crates/` (mode-independent). Auth is binary-level. |
| "Where do I add billing logic?" | `apps/cloud/src/billing.rs`. Never in a library crate. |

Simple rules:
- **`crates/`** = mode-independent. NEVER references deployment-specific concepts.
- **`apps/desktop/`** = desktop-specific wiring.
- **`apps/server/`** = server-specific wiring.
- **`apps/cloud/`** = cloud-specific extensions.

---

## 7. Why NOT a DeploymentMode Enum

| Approach | Problem |
|----------|---------|
| `enum DeploymentMode { Local, Server, Cloud }` | Leaks into every crate. `if mode == Cloud` in 50 files (n8n's mistake). |
| `#[cfg(feature = "cloud")]` in library crates | Untested combinations. Feature flag explosion (Windmill's mistake). |
| Runtime license check | Good for enterprise gating, but Nebula's differences are structural (API exists or doesn't), not feature-level. |

**Our approach: binary-level separation + config + DI. Zero mode awareness in library crates. The type system enforces boundaries — if a crate isn't in your Cargo.toml, it doesn't exist.**

---

## 8. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Binary targets | `apps/desktop/` only | + `apps/server/` + `apps/cloud/` |
| Mode concept | Implicit | Explicit: 3 binary targets, documented |
| Config | Single format | Per-binary config files |
| CI | Desktop only | Desktop + server both checked |
| Contributor guidance | None | Clear rules: `crates/` = mode-free |

---

## 9. Not In Scope

- Enterprise license key system (v2 — when EE features exist)
- Runtime feature toggles within server binary (use config, not flags)
- Mobile binary target (Phase 3+)
- Edge/WASM binary target (Phase 3+)
