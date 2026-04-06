# nebula-cli — Design Spec

## Goal

Developer-facing CLI for workflow management, action development, plugin authoring, and local execution. Separate binary from desktop app — for terminal-first developers.

## Philosophy

- **Developer tool, not end-user tool.** Desktop app serves non-technical users. CLI serves Rust developers and DevOps.
- **Server-optional.** CLI can run workflows locally (embedded engine) OR against a remote server (API client).
- **Convention over configuration.** `nebula run workflow.yaml` just works.
- **Composable.** Output is JSON by default. Pipes to `jq`, scripts, CI/CD.

---

## 1. Command Structure

```
nebula
├── run <workflow>             Execute a workflow (local or remote)
├── validate <workflow>        Validate workflow definition
├── workflows
│   ├── list                   List workflows (remote)
│   ├── get <id>               Get workflow details
│   ├── create <file>          Create from YAML/JSON
│   ├── update <id> <file>     Update workflow
│   ├── delete <id>            Delete workflow
│   └── activate <id>          Activate trigger
├── executions
│   ├── list                   List executions
│   ├── get <id>               Get execution state + outputs
│   ├── cancel <id>            Cancel running execution
│   ├── logs <id>              Stream execution logs
│   └── rerun <id> [--node]    Re-run execution or single node
├── actions
│   ├── list                   List registered actions
│   ├── info <key>             Action metadata + parameters
│   └── test <key>             Test action with sample input
├── credentials
│   ├── list                   List credentials (redacted)
│   ├── create <type>          Interactive credential creation
│   ├── test <id>              Test credential
│   └── delete <id>            Delete credential
├── plugins
│   ├── list                   List loaded plugins
│   ├── info <key>             Plugin details
│   └── new <name>             Scaffold new plugin project
├── dev
│   ├── init                   Initialize nebula project in current directory
│   ├── action new <name>      Scaffold new action
│   ├── plugin new <name>      Scaffold new plugin (same as plugins new)
│   └── benchmark              Run performance benchmarks
├── server
│   ├── start                  Start API server
│   ├── status                 Check server health
│   └── migrate                Run database migrations
├── config
│   ├── show                   Show resolved configuration
│   ├── validate               Validate config file
│   └── init                   Generate default config file
└── version                    Show version info
```

---

## 2. Key Workflows

### Run workflow locally (no server)

```bash
# From YAML file — embedded engine, SQLite storage
$ nebula run workflow.yaml --input '{"url": "https://example.com"}'

# Output: JSON execution result
{
  "execution_id": "ex_abc123",
  "status": "completed",
  "duration_ms": 1420,
  "outputs": {
    "nd_fetch": { "status": 200, "body": "..." },
    "nd_transform": { "result": "..." }
  }
}

# Stream mode — show progress as nodes complete
$ nebula run workflow.yaml --input '{"url": "..."}' --stream
[12:04:01] ▶ Node "Fetch" started
[12:04:02] ✅ Node "Fetch" completed (450ms)
[12:04:02] ▶ Node "Transform" started
[12:04:02] ✅ Node "Transform" completed (12ms)
[12:04:02] ✅ Workflow completed (462ms)
```

### Run against remote server

```bash
# Configure remote
$ nebula config init --remote https://nebula.example.com --api-key nbl_sk_...

# Run on remote server
$ nebula run workflow.yaml --remote
```

### Scaffold new action

```bash
$ nebula dev action new http-request
Created: plugins/http-request/
├── Cargo.toml
├── src/
│   └── lib.rs          # #[derive(Action, Parameters)] skeleton
├── tests/
│   └── integration.rs  # TestContextBuilder example
└── nebula-plugin.toml  # Plugin manifest
```

Generated `lib.rs`:
```rust
use nebula_action::prelude::*;
use nebula_parameter::prelude::*;

#[derive(Action, Parameters, Deserialize)]
#[action(key = "http.request", name = "HTTP Request")]
pub struct HttpRequest {
    #[param(label = "URL")]
    #[validate(required, url)]
    url: String,

    #[param(default = "GET")]
    method: String,
}

impl StatelessAction for HttpRequest {
    type Input = Self;
    type Output = Value;

    async fn execute(&self, _input: Self, ctx: &ActionContext) -> ActionResult<Value> {
        // TODO: implement
        ActionResult::success(json!({}))
    }
}
```

### Stream execution logs

```bash
$ nebula executions logs ex_abc123 --follow
[12:04:01.000] INFO  engine: Node started node_id=nd_1 action=http.request
[12:04:01.450] INFO  action: HTTP 200 from https://example.com
[12:04:01.451] INFO  engine: Node completed node_id=nd_1 duration=450ms
[12:04:01.452] INFO  engine: Node started node_id=nd_2 action=transform
[12:04:01.464] INFO  engine: Node completed node_id=nd_2 duration=12ms
[12:04:01.465] INFO  engine: Execution completed status=success duration=462ms
```

---

## 3. Technical Implementation

### Crate: `apps/cli/`

```toml
# apps/cli/Cargo.toml
[package]
name = "nebula-cli"
version = "0.1.0"

[[bin]]
name = "nebula"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
nebula-engine = { path = "../../crates/engine" }
nebula-storage = { path = "../../crates/storage", features = ["sqlite"] }
nebula-workflow = { path = "../../crates/workflow" }
nebula-config = { path = "../../crates/config" }
nebula-log = { path = "../../crates/log" }
tokio = { workspace = true, features = ["full"] }
serde_json = { workspace = true }
reqwest = { workspace = true, optional = true }  # for remote mode

[features]
default = ["local"]
local = ["nebula-storage/sqlite"]
remote = ["dep:reqwest"]  # API client for remote server
```

### Local vs Remote

```rust
enum ExecutionTarget {
    /// Embedded engine — run in-process with SQLite
    Local(LocalEngine),
    /// Remote API client — delegate to server
    Remote(ApiClient),
}

impl ExecutionTarget {
    fn from_config(config: &CliConfig) -> Self {
        if config.remote.is_some() {
            Self::Remote(ApiClient::new(&config.remote.unwrap()))
        } else {
            Self::Local(LocalEngine::new(&config.data_dir))
        }
    }
}
```

### Output Formats

```bash
# Default: JSON (for piping)
$ nebula workflows list
[{"id": "wf_abc", "name": "Sync Orders", "active": true}]

# Human-readable table
$ nebula workflows list --format table
ID        NAME          ACTIVE  LAST RUN
wf_abc    Sync Orders   ✅      2 min ago
wf_def    Daily Report  ❌      1 day ago

# Compact (just IDs)
$ nebula workflows list --format ids
wf_abc
wf_def
```

---

## 4. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| CLI | Does not exist | Full CLI binary (`nebula` command) |
| Local execution | Desktop app only | CLI can run workflows in terminal |
| Scaffolding | Manual | `nebula dev action new` generates skeleton |
| Remote management | None (API exists but no client) | CLI as API client |
| CI/CD integration | None | `nebula run` + `nebula validate` for pipelines |

---

## 5. Implementation Phases

| Phase | What |
|-------|------|
| 1 | `nebula run` + `nebula validate` (local, embedded engine) |
| 2 | `nebula workflows/executions` (remote, API client) |
| 3 | `nebula dev action new` + `nebula dev plugin new` (scaffolding) |
| 4 | `nebula server start/migrate` (server management) |
| 5 | `nebula credentials` (interactive creation + test) |
| 6 | `nebula executions logs --follow` (SSE streaming) |

**Phase 1 = minimum viable CLI.**

---

## 6. Not In Scope

- Interactive TUI (terminal UI with panels) — v2
- REPL for expression evaluation — v2
- Plugin marketplace commands (install/publish) — needs WASM ecosystem
- Desktop app integration (CLI and desktop are separate tools)
- Auto-update mechanism
