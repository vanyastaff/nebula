# nebula-config Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Every Nebula service and runtime component needs configuration: file paths, database URLs, feature flags, timeouts, and environment-specific overrides. Loading from multiple sources (defaults, file, env) with clear precedence and validation prevents production misconfigurations and allows the same binary to run in dev and prod.

**nebula-config is the unified configuration system for Nebula services and runtime components.**

It answers: *How do services load, merge, validate, and optionally hot-reload configuration from defaults, files, and environment without each crate reinventing the wheel?*

```
ConfigBuilder
    ↓
sources: defaults < file < env < inline (documented precedence)
    ↓
load + merge → validate (optional validator integration) → Config
    ↓
get<T>(path), optional watcher/hot-reload
```

This is the config contract: deterministic precedence, typed access, validation gate, and optional reload without restart.

---

## User Stories

### Story 1 — Service Author Loads Config at Startup (P1)

A service (engine, API, worker) needs database URL, log level, and timeouts. It uses ConfigBuilder to add defaults, then file and env. After load, it calls `config.get::<String>("database.url")` and `config.get::<u64>("timeouts.http_ms")`. Missing required keys or invalid types fail at load or at get.

**Acceptance**:
- ConfigBuilder supports defaults, file (JSON/TOML/YAML/INI/properties), env, inline
- Precedence is documented and tested: defaults < file < env < inline
- get<T> with path-based access; serde-based conversion; clear error on missing or type mismatch
- Validation pipeline (e.g. ConfigValidator with nebula-validator) can reject invalid config before use

### Story 2 — Operator Overrides via Environment (P1)

In production, the operator sets DATABASE_URL and LOG_LEVEL via environment variables without editing files. The same binary and image work in every environment.

**Acceptance**:
- Env source maps env vars to config paths (e.g. DATABASE_URL → database.url or configurable mapping)
- Env overrides file and defaults per documented precedence
- No secrets in logs; optional redaction for sensitive paths

### Story 3 — Hot-Reload Without Restart (P2)

A long-running service wants to change log level or feature flags without restart. Config supports a watcher and optional reload callback; after reload, new values are visible to next get() or to a subscribed component.

**Acceptance**:
- Watcher detects file change (or explicit reload API)
- Reload is gated by validator: invalid config keeps last-known-good
- Atomic activation so no partial state; document reload semantics for concurrent get()

### Story 4 — Multiple Crates Use Config Without Conflict (P2)

Engine, runtime, API, and credential each need their own config subtree. Config supports path namespacing so each crate reads under its key (e.g. "engine.workers", "api.port"). Merge and precedence are global; usage is scoped by path.

**Acceptance**:
- Single Config instance can hold multiple logical sections
- Each crate documents its paths and required keys in its docs
- No crate-specific types in nebula-config; only generic get and path rules

---

## Core Principles

### I. Deterministic Precedence

**Precedence order (defaults < file < env < inline) is documented and tested. Same inputs ⇒ same resolved config.**

**Rationale**: Operators and developers need to predict which value wins. Non-determinism causes "works on my machine" and production incidents.

**Rules**:
- Precedence is fixed and documented in API and README
- Contract tests lock precedence behavior
- Additive sources (e.g. multiple files) have defined merge order

### II. Validation Before Use

**Invalid config should be rejected at load (or at reload), not at first get(). Optional integration with nebula-validator.**

**Rationale**: Failing fast at startup is better than failing in the middle of a workflow. Validation pipeline allows schema-like checks and clear error messages.

**Rules**:
- ConfigBuilder or load() can accept a validator
- If validation fails, load returns Err; no partial Config
- Reload: invalid config does not replace current; last-known-good retained

### III. Typed Access with Clear Errors

**get<T>(path) returns Result or Option; path not found and type mismatch are distinct.**

**Rationale**: Callers need to know whether the key is missing or wrong type. Ambiguous errors make debugging hard.

**Rules**:
- Path-based access; stable path semantics (e.g. "a.b.c" for nested)
- Error type distinguishes missing key vs conversion error
- Document path format and any reserved keys

### IV. No Business-Domain Semantics in Config Crate

**Config loads, merges, and serves values. It does not define what "database" or "workflow" config means.**

**Rationale**: Domain semantics (e.g. "workflow.max_nodes") belong in engine/workflow crates. Config is the mechanism, not the policy.

**Rules**:
- No workflow-, engine-, or credential-specific types in config crate
- Each consumer crate documents its own config paths and semantics
- Config crate only provides loader, merge, validation hook, and get

### V. Hot-Reload Is Safe and Observable

**If hot-reload is supported, it must not apply invalid config and should be observable (e.g. logging or callback).**

**Rationale**: Silent reload with bad config can cause undefined behavior. Last-known-good and validator gate prevent that.

**Rules**:
- Reload runs through same validation as initial load
- On validation failure, retain current config and report error (log or callback)
- Document concurrency: get() during reload sees old or new consistently (define once)

### VI. Remote and Database Sources Are Optional/Planned

**Current production path is defaults + file + env. Remote (HTTP) or database-backed config can be added as optional sources with same precedence contract.**

**Rationale**: Many deployments need only file + env. Remote/database sources increase complexity and are documented as future or optional.

**Rules**:
- Document which sources are stable and which are experimental or placeholder
- New sources must follow same merge and validation rules

---

## Production Vision

### The config system in an n8n-class fleet

In a production Nebula deployment, each process (API, worker, engine) builds Config at startup from defaults, then config file(s), then environment. Validation runs before the process accepts traffic. Optional watcher reloads config on file change; validator ensures only valid config is activated. Operators change behavior via env or config file without code deploy.

```
ConfigSource: Default | File(PathBuf) | FileAuto | Directory | Env | EnvWithPrefix(String)
              | Remote(String) | Database{..} | KeyValue{..} | Inline(String) | CommandLine
    ↓ priority order (source.priority()); lower = overrides later
ConfigBuilder → with_defaults_json / with_source / with_loader / with_validator / with_watcher
    ↓ build() → try_sources (CompositeLoader), merge_json, validator.validate()
Config: Arc<RwLock<Value>>, get<T>(path), reload(), start_watching(), stop_watching()
Loaders: FileLoader (JSON/TOML/YAML/INI/Properties), EnvLoader, CompositeLoader
Validators: NoOpValidator, SchemaValidator, CompositeValidator, FunctionValidator; ConfigValidator impl for Validate<Value>
Watchers: FileWatcher (notify), PollingWatcher, NoOpWatcher; ConfigWatchEvent / ConfigWatchEventType
```

Reload: starts from defaults, loads all non-Default sources concurrently, merges in priority order, validates; on success atomically replaces data; on failure keeps last-known-good. Auto-reload loop optional via `with_auto_reload_interval`; cancelled on Config drop. Metrics/logging: "config_reload_total", "config_reload_errors_total". Remote/Database/KeyValue sources are placeholder (no default loader); production path is defaults + file + env.

### From the archives: cross-cutting and layers

The archive (`docs/crates/config/_archive/`: archive-business-cross.md, archive-crates-dependencies.md, archive-layers-interaction.md) places config in "Cross-Cutting Concerns" and describes it as shared by many crates. Production vision: config remains a library used by every service; precedence and validation are contract-tested in `tests/contract/`; compatibility fixtures in `tests/fixtures/compat/` (e.g. validator_contract_v1.json); hot-reload is optional and safe; domain semantics stay in consuming crates.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Remote/database source implementations | Medium | Placeholder contracts only; production may need HTTP or KV config source |
| Redaction for sensitive paths in logs | Medium | Avoid logging secrets when dumping config or on error |
| Formal contract tests for precedence and reload | High | Partially done; lock all precedence and reload semantics |
| Document interaction with runtime/resource/credential | Medium | INTERACTIONS.md and per-crate path documentation |
| Versioned compatibility fixtures | Low | Already in compat fixtures; keep updated |

---

## Key Decisions

### D-001: Precedence Order Defaults < File < Env < Inline

**Decision**: Fixed order so that operator overrides (env) always win over file and defaults.

**Rationale**: Twelve-factor and ops expectations. Same order everywhere avoids confusion.

**Rejected**: Configurable precedence — would make behavior unpredictable across deployments.

### D-002: Validator Gates Reload

**Decision**: On hot-reload, new config is validated; if invalid, current config is retained and error is reported.

**Rationale**: Prevents bad reload from breaking running process. Last-known-good is a safe default.

**Rejected**: Apply reload anyway and let runtime fail — unacceptable for production.

### D-003: Path-Based Access, No Crate-Specific Types

**Decision**: Generic get<T>(path) with string paths. No "engine config" or "api config" types in config crate.

**Rationale**: Keeps config crate free of domain; each consumer defines its own path layout and types.

**Rejected**: Config structs per crate in config crate — would create dependency and coupling.

### D-004: Optional nebula-validator Integration

**Decision**: Config can use a validator (e.g. ConfigValidator bridging nebula-validator) for schema-like validation.

**Rationale**: Reuse of validation combinators and clear error reporting without duplicating logic.

**Rejected**: Inline validation only in config — would duplicate validator crate capabilities.

---

## Open Proposals

### P-001: Sensitive Path Redaction

**Problem**: Logging config or errors might expose secrets.

**Proposal**: Allow marking paths as sensitive; Debug and log output redact those values.

**Impact**: Additive; default redaction list or API to register sensitive paths.

### P-002: Remote and Database Sources

**Problem**: Some deployments want config from HTTP endpoint or database.

**Proposal**: Add ConfigSource::Remote(url) and ConfigSource::Database with same merge and validation semantics. Optional features.

**Impact**: New dependencies and failure modes; document retry and caching.

### P-003: Contract Test Suite and Compatibility Fixtures

**Problem**: Precedence and reload behavior must not regress.

**Proposal**: Formalize contract tests and versioned fixtures (already started); extend to all precedence and reload cases.

**Impact**: Non-breaking; improves stability.

---

## Non-Negotiables

1. **Deterministic precedence** — documented order (defaults < file < env < inline); same inputs ⇒ same config.
2. **Validation before use** — invalid config is rejected at load; reload does not apply invalid config.
3. **Typed get with clear errors** — path-based access; missing vs type error distinct.
4. **No domain types in config crate** — only loader, merge, validation, get; semantics in consumers.
5. **Hot-reload retains last-known-good on validation failure** — no partial or invalid config applied.
6. **Breaking precedence or path semantics = major + MIGRATION.md** — operators depend on current behavior.

---

## Governance

- **PATCH**: Bug fixes, docs, internal refactors. No change to precedence or get/path semantics.
- **MINOR**: Additive only (new source types, new optional APIs). No change to existing precedence or validation behavior.
- **MAJOR**: Breaking changes to precedence, path format, or validation contract. Requires MIGRATION.md.

Every PR must verify: contract tests pass; precedence and reload behavior documented; no domain-specific types in public API.
