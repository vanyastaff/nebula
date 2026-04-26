# DeepWiki Findings — acts-next (luminvent/acts)

Note: luminvent/acts is not indexed in DeepWiki ("Repository not found. Visit https://deepwiki.com to index it."). All 7 queries were redirected to yaojianpin/acts (the upstream/parent repo, structurally identical codebase up to the luminvent fork's 3–4 additional commits). Results below accurately describe the forked code because the core architecture is shared.

---

## Query 1 — Core trait hierarchy for actions/nodes/activities

Result: Confirmed. The core trait hierarchy is:
- `ActTask` trait (init/run/next/review/error methods) implemented by `Arc<Task>`, dispatching to `NodeContent` enum variants (Workflow, Branch, Step, Act).
- `ActPackageFn` trait (execute/start methods) for package execution.
- `ActPlugin` trait (on_init method) for system plugins.

DeepWiki noted: "Acts uses Step, Branch, Act to build the workflow" aligns with NodeContent variants. Channel event system uses `type` field indicating "workflow/step/branch/act".

---

## Query 2 — How is workflow state persisted and recovered after crash?

Result: Persisted via pluggable DbCollection backends (MemStore, acts-store-sqlite, acts-store-postgres). Key entities stored: Task (id, kind, name, pid, tid, state, start_time, end_time, data) and Model (id, name, data as YAML, version).

Recovery: `cache.restore()` queries store for in-progress Procs, deserializes NodeTree from `data::Proc.tree` JSON, re-enqueues non-terminal Tasks. MessageStatus updated on action completion for accurate persistent state.

No append-only event log, no replay-based reconstruction — snapshot-based only.

---

## Query 3 — Credential or secret management approach

Result: Via `SecretsVar` implementing `ActUserVar` trait. The `secrets` namespace is exposed as a JavaScript global. Data provided at workflow start time as `Vars::new().with("secrets", Vars::new().with("TOKEN", "my_token"))`. No storage, no encryption, no lifecycle management. `default_data()` returns None — secrets must be injected at runtime by the caller.

---

## Query 4 — How are plugins or extensions implemented?

Result: Plugins implement `ActPlugin` trait, compiled as Rust crates in the same workspace. `EngineBuilder.add_plugin()` registers them; `build().await` calls each plugin's `on_init()`. Statically linked, not dynamically loaded.

Two plugin types: Store plugins (SqliteStore, PostgresStore — register DbCollection instances) and Package plugins (StatePackagePlugin, HTTP, Shell — register packages and message handlers).

---

## Query 6 — How are triggers (webhooks, schedules, external events) modeled?

Result: Triggers are `ActPackage` implementations in `ActPackageCatalog::Event` category. Three built-in: `acts.event.manual`, `acts.event.hook`, `acts.event.chat`. Deployed via `ModelExecutor.deploy_event()` which creates an `Event` store record. `EventExecutor.start(event_id, params)` looks up the event record and calls the package's `start()` method.

Schedule events are roadmap-only (not implemented). No webhook URL allocation, no HMAC verification, no external broker support.

---

## Query 7 — Built-in LLM or AI agent integration?

Result: No built-in LLM integration. `ActPackageCatalog::Ai` enum variant exists as a placeholder comment "AI related for LLMs". `plugins/ai` is roadmap item marked as not yet created. No providers, no abstractions, no API calls implemented.

---

## Query 9 — Known limitations or planned redesigns

Result: From roadmap in README:
- Missing: schedule event package, form plugin, AI plugin, pubsub plugin, observability plugin, database plugin, mail plugin
- Documentation explicitly marked incomplete
- No distributed safety (single-process scheduler, no leader election)
- No model versioning with live migration
- API stability pre-1.0

From upstream yaojianpin/acts issues: Issue #10 (model versioning open), Issue #16 (Cache learning — open question about cache behavior).
