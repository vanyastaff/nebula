# Windmill — Peer Research (Rust workflow engine)

> Седьмой в серии peer-research. Разбор **Windmill** (`windmill-labs/windmill`)
> — Rust backend + Svelte frontend, visual + code, self-hosted OSS.
> **Архитектурно самый близкий peer Nebula'у:** тот же язык, те же problem
> shapes. Их grabli = те же, на которые Nebula наступит.

> **⚠️ Scope reminder.** Всё содержимое ниже — описание **Windmill** как
> peer-research subject. Nebula-mitigation — в correlation-table и Quick wins
> секциях отдельно.

## Метаданные исследования

- **Последняя сверка:** 2026-04-20
- **Репо:** `windmill-labs/windmill`
  - Stars: 16,274; Forks: 927; Open issues: 704; Repo size: ~390MB
  - Лицензия: AGPLv3 + proprietary `enterprise` feature flag
- **Источники:** GitHub issues, DeepWiki, backend/Cargo.toml analysis,
  community.windmill.dev, Reddit/HN
- **Провенанс:** «hot» = 5+ повторов; «confirmed» = в коде или reproduced

---

## Executive Summary — топ-5 pain areas

1. **PostgreSQL as job queue работает, но leaks everywhere.**
   `SELECT FOR UPDATE SKIP LOCKED` polling + `SLEEP_QUEUE` tuning,
   connection-pool timeouts во время flow updates
   ([#6408](https://github.com/windmill-labs/windmill/issues/6408)),
   slow под HTTP-trigger load
   ([#6434](https://github.com/windmill-labs/windmill/issues/6434)),
   duplicate dependency jobs из same root
   ([#6055](https://github.com/windmill-labs/windmill/issues/6055) —
   tagged across 50+ releases, unresolved). Community open на Kafka/Redis
   alternatives ([#173](https://github.com/windmill-labs/windmill/issues/173),
   **с 2022 года!**).

2. **Worker binary — monster.** `deno_core` + `v8` + Python + Bun + Go +
   nsjail glue + 25+ cargo features (см. `backend/Cargo.toml`: `all_languages`,
   `deno_core`, `kafka`, `sqs_trigger`, `nats`, `otel`, `dind`, `postgres_trigger`,
   `mqtt_trigger`, `gcp_trigger`, `websocket`, `smtp`, `native_trigger`, `mcp`,
   `bedrock`, `parquet`, `tantivy`, `embedding`). Compile-time pain:
   Windows cross-compile broken
   ([#1836](https://github.com/windmill-labs/windmill/issues/1836),
   [#3089](https://github.com/windmill-labs/windmill/issues/3089)),
   tokio_unstable breaks compile
   ([#3284](https://github.com/windmill-labs/windmill/issues/3284)),
   jemalloc fails on ARM64 page sizes
   ([#4422](https://github.com/windmill-labs/windmill/issues/4422)),
   SIGILL на arm64 k8s
   ([#8174](https://github.com/windmill-labs/windmill/issues/8174)).

3. **Editor performance at scale.** Flow editor unresponsive после testing
   ([#3374](https://github.com/windmill-labs/windmill/issues/3374)).
   Memory leak switching между scripts
   ([#5822](https://github.com/windmill-labs/windmill/issues/5822)).
   High memory/CPU на bun imports
   ([#3306](https://github.com/windmill-labs/windmill/issues/3306)).
   «Too many tiny JS assets»
   ([#8671](https://github.com/windmill-labs/windmill/issues/8671)).
   QuickJS expression eval fails с large payloads
   ([#8073](https://github.com/windmill-labs/windmill/issues/8073)).

4. **Worker lifecycle brittle.** Workers disappear из UI
   ([#6718](https://github.com/windmill-labs/windmill/issues/6718)),
   liveness DEAD
   ([#4907](https://github.com/windmill-labs/windmill/issues/4907)),
   can't be deleted
   ([#5519](https://github.com/windmill-labs/windmill/issues/5519)),
   init script «not finished if forked»
   ([#7029](https://github.com/windmill-labs/windmill/issues/7029)),
   memory spike на piptar upload
   ([#5968](https://github.com/windmill-labs/windmill/issues/5968)),
   http connection broke в k8s несмотря на stable pods
   ([#8469](https://github.com/windmill-labs/windmill/issues/8469)).

5. **Git-sync — killer feature и killer footgun.** `wmill sync push`
   silently deleted flow который юзер не хотел deleting
   ([#8468](https://github.com/windmill-labs/windmill/issues/8468)).
   RLS violation на save
   ([#7680](https://github.com/windmill-labs/windmill/issues/7680)).
   Git Sync не honoring wmill.yml
   ([#3848](https://github.com/windmill-labs/windmill/issues/3848)).
   OAuth missing для git sync
   ([#5228](https://github.com/windmill-labs/windmill/issues/5228)).

---

## Architectural Overview

Windmill — Rust (backend) + Svelte (frontend) workflow platform.

**Backend crates** (`backend/`):
- `windmill-api` (Axum HTTP), `windmill-worker` (executor),
  `windmill-queue` (job lifecycle), `windmill-common` (shared types),
  `windmill-audit`
- Per-language parsers: `windmill-parser-ts`, `windmill-parser-py`,
  `windmill-parser-rust`, `windmill-parser-go`, `windmill-parser-bash`
- Split API workspace: `windmill-api-assets`, `windmill-api-auth`,
  `windmill-api-configs`, `windmill-api-flows`, `windmill-api-embeddings`,
  `windmill-api-debug`, etc.

**Runtime choices:**
- `tokio 1.46` (`features = ["full","tracing","time"]`)
- `sqlx` с `runtime-tokio-rustls` + `postgres` + compile-time query verification
  (`.sqlx/` dir committed)
- Queue = `v2_job_queue` PostgreSQL table
- Workers poll с `SELECT FOR UPDATE SKIP LOCKED`;
  `LISTEN/NOTIFY` (`notify_queue`, `notify_insert_on_completed_job`)
  used для completion fan-out, не dispatch

**Script execution:**
- One process per job, под `/tmp/windmill/{job_id}`
- Per-language executor modules: `python_executor.rs`, `deno_executor.rs`,
  `bun_executor.rs`, `go_executor.rs`, `bash_executor.rs`, `rust_executor.rs`,
  `csharp_executor.rs`, `java_executor.rs`, `php_executor.rs`, `nu_executor.rs`
- Sandboxing: `nsjail` на Linux, Job Objects на Windows
- JS engine in-process — `deno_core` (rusty_v8)
- Python deps через `uv` с results cached в `pip_resolution_cache` table +
  local `/tmp/windmill/cache/`; S3-sync — EE

**Flow model:** OpenFlow spec — `FlowModule` — discriminated union:
`RawScript | PathScript | PathFlow | ForloopFlow | WhileloopFlow | BranchOne | BranchAll | Identity | AiAgent`. Suspend/resume через
`wmill.getResumeUrls(approver)`.

---

## Ось 1 — COVERAGE

**Script runtimes** (confirmed в backend/Cargo.toml): Python (uv),
TypeScript (Deno + Bun), JavaScript, Go, Bash, PowerShell, SQL (inline),
Rust, C#, Java, PHP, Nu. Нет native Ruby; R requested
([#3695](https://github.com/windmill-labs/windmill/issues/3695)), Nim closed
([#7324](https://github.com/windmill-labs/windmill/issues/7324)).

**Triggers:** Schedule (cron с optional seconds), HTTP routes, Webhooks,
Kafka, SQS, NATS, MQTT, GCP PubSub, WebSocket, SMTP, Postgres, native.
Many trigger-kinds живут за EE features. MQTT был wontfix long
([#1966](https://github.com/windmill-labs/windmill/issues/1966) closed
wontfix 2025-02) before later adding.

**Resources (~= Nebula credentials):** strongly typed с schema inference
через WASM parsers — Python `db: dict` param annotated «postgresql type»
generates `format: "resource-postgresql"`. Resources referenced by path
(`$res:f/folder/my_postgres`). Variables — simpler encrypted strings.

**Apps (UI builder):** Svelte-based drag-drop; internal store, **НЕ flow engine**.
Separate lifecycle.

**AI:** «Windmill Copilot» — inline code completion (Monaco InlineCompletionsProvider),
chat handler, prompt-to-script generation. Providers: OpenAI, Anthropic,
Azure OpenAI, AWS Bedrock, custom AI resource. **Per-language system prompts**
embed Windmill conventions (как `main` called, как resources resolve).

**EE-only:**
- Prometheus metrics (`METRICS_ADDR`)
- SAML (`enterprise_saml`)
- Stripe
- Embeddings
- Parquet (`upload_file_internal` explicitly stubbed в OSS —
  `job_helpers_oss.rs`)
- Tantivy full-text job/log search
- License management
- S3 storage > 20 files / > 50MB
- Multiplayer real-time collab
- Several trigger kinds
- S3 dep-cache sync между workers

«Intentional security vulnerability в CE»
([#7619](https://github.com/windmill-labs/windmill/issues/7619))
generated notable community friction.

**Coverage gaps / requests:**
- Rust shebangs ([#5713](https://github.com/windmill-labs/windmill/issues/5713))
- jsr support для bun ([#4592](https://github.com/windmill-labs/windmill/issues/4592))
- Native DB-triggered events ([#2684](https://github.com/windmill-labs/windmill/issues/2684))
- Per-flow env vars ([#6832](https://github.com/windmill-labs/windmill/issues/6832),
  [#4486](https://github.com/windmill-labs/windmill/issues/4486))
- Unit testing + manual dep mgmt ([#5384](https://github.com/windmill-labs/windmill/issues/5384))

---

## Ось 2 — ERGONOMICS / UNIVERSALITY

### Rust compile pain

- **Windows build broken, actively abandoned**
  ([#1836](https://github.com/windmill-labs/windmill/issues/1836),
  [#3089](https://github.com/windmill-labs/windmill/issues/3089)); gave up
  cross-platform backend и ship Linux binaries + Job Objects shim.
- `tokio_unstable` fragile
  ([#3284](https://github.com/windmill-labs/windmill/issues/3284)).
- Community asks sccache для user Rust jobs
  ([#4369](https://github.com/windmill-labs/windmill/issues/4369)).
- Dev setup docs poor enough to file as bug
  ([#2584](https://github.com/windmill-labs/windmill/issues/2584)).
- Offline install blocked
  ([#6537](https://github.com/windmill-labs/windmill/issues/6537),
  [#1993](https://github.com/windmill-labs/windmill/issues/1993)).

### Runtime quirks

- Bun ignores `BUN_TLS_CA_FILE` в sandbox
  ([#7018](https://github.com/windmill-labs/windmill/issues/7018))
- SIGILL на arm64 k8s
  ([#8174](https://github.com/windmill-labs/windmill/issues/8174))
- OTEL trace proxy breaks bun но не node
  ([#8254](https://github.com/windmill-labs/windmill/issues/8254))
- Bun all-jobs-fail на upgrade
  ([#5552](https://github.com/windmill-labs/windmill/issues/5552))
- Custom/corporate CAs recurring pain
  ([#1564](https://github.com/windmill-labs/windmill/issues/1564))
- `jemalloc` unsupported system page size на RPi5
  ([#4422](https://github.com/windmill-labs/windmill/issues/4422))

### Flow UX

- Editor unresponsive after action test
  ([#3374](https://github.com/windmill-labs/windmill/issues/3374))
- Memory leak switching scripts
  ([#5822](https://github.com/windmill-labs/windmill/issues/5822))
- Resume modal misaligned в branches
  ([#7710](https://github.com/windmill-labs/windmill/issues/7710))
- Renaming step to «result» breaks editor
  ([#7139](https://github.com/windmill-labs/windmill/issues/7139))
- «date» flow input ignored
  ([#7670](https://github.com/windmill-labs/windmill/issues/7670))

### Script versioning

Git-native — powerful но:
1. Couples devs и ops
2. `wmill sync` can unexpectedly delete
   ([#8468](https://github.com/windmill-labs/windmill/issues/8468))
3. RLS can block save
   ([#7680](https://github.com/windmill-labs/windmill/issues/7680))

### Workers / queue

- Polling interval `SLEEP_QUEUE` must be tuned
  ([#2907](https://github.com/windmill-labs/windmill/issues/2907))
- Workers disappear в UI
  ([#6718](https://github.com/windmill-labs/windmill/issues/6718))
- Liveness DEAD
  ([#4907](https://github.com/windmill-labs/windmill/issues/4907))
- Init-script detection fails на fork
  ([#7029](https://github.com/windmill-labs/windmill/issues/7029))
- Piptar memory spike
  ([#5968](https://github.com/windmill-labs/windmill/issues/5968))
- WORKER_TAGS ignored для native worker
  ([#4681](https://github.com/windmill-labs/windmill/issues/4681))
- Dedicated workers need rework
  ([#5337](https://github.com/windmill-labs/windmill/issues/5337))

### Self-host friction

AGPL backend + proprietary EE compile flag confuses deployers
([#4514](https://github.com/windmill-labs/windmill/issues/4514),
[#5014](https://github.com/windmill-labs/windmill/issues/5014) «open letter
to Windmill team»). Docker cannot mount files из Worker host
([#4669](https://github.com/windmill-labs/windmill/issues/4669)).

---

## Ось 3 — BUGS / PROBLEMS

### State durability

- Duplicate dep jobs scheduled из same root — dragged across 50+ releases
  ([#6055](https://github.com/windmill-labs/windmill/issues/6055))
- Invalid flow status triggers error handler
  ([#6536](https://github.com/windmill-labs/windmill/issues/6536))
- Workspace error handler + parent_job
  ([#7311](https://github.com/windmill-labs/windmill/issues/7311))
- `delete_workspace` FK на raw_script_temp
  ([#8751](https://github.com/windmill-labs/windmill/issues/8751))

### Performance/load

- Slow HTTP GET trigger под k6
  ([#6434](https://github.com/windmill-labs/windmill/issues/6434))
- Rare slowdown в benchmarks
  ([#5346](https://github.com/windmill-labs/windmill/issues/5346))
- Resource-hog when idle (fixed,
  [#4680](https://github.com/windmill-labs/windmill/issues/4680))

### Integrations broken

- Database Manager 500 after 600s
  ([#8261](https://github.com/windmill-labs/windmill/issues/8261))
- Postgres 5s add-connection failure
  ([#6879](https://github.com/windmill-labs/windmill/issues/6879))
- Supabase username parsed as db
  ([#5911](https://github.com/windmill-labs/windmill/issues/5911))
- mssql float rounding на inline scripts
  ([#7403](https://github.com/windmill-labs/windmill/issues/7403))
- Parquet→CSV error
  ([#4870](https://github.com/windmill-labs/windmill/issues/4870))
- OAuth broken в nextcloud-triggers
  ([#8470](https://github.com/windmill-labs/windmill/issues/8470))

### Security

- Saved inputs missing created_by ownership check, CWE-639
  ([#8037](https://github.com/windmill-labs/windmill/issues/8037), closed)

---

## Rust-Specific Lessons — **САМОЕ ЦЕННОЕ для Nebula**

Windmill — closest prior art which Nebula building — same language, same
shape of problem. What they learned the hard way:

1. **Feature flags — only way «single backend binary» scales past 10 runtimes.**
   Windmill `backend/Cargo.toml` ships 25+ cargo features и uses
   `#[cfg(feature = "enterprise")]` / `#[cfg(feature = "parquet")]` liberally.
   **Cost:** mono-binary build time explodes (см. [#4369](https://github.com/windmill-labs/windmill/issues/4369) на sccache).

2. **`tokio` + `sqlx` (rustls) — stable default для Rust workflow backends.**
   Windmill на `tokio 1.46` с `full,tracing,time` и `sqlx` с
   `runtime-tokio-rustls,postgres,macros,migrate`. Они **commit `.sqlx/`
   offline-query cache** так CI builds без live DB. Не используют diesel.
   Не используют `async-std` или `smol`.

3. **`tokio_unstable` — trap.**
   [#3284](https://github.com/windmill-labs/windmill/issues/3284) sat open
   for years — enabling `tokio_unstable` breaks their compile. Nebula should
   avoid unless specific feature (task_dump, task instrumentation) essential.

4. **Cross-platform не free.** Windmill effectively gave up Windows backend
   compilation ([#1836](https://github.com/windmill-labs/windmill/issues/1836)
   open since 2023-12). Их sandbox — Linux-only через nsjail; Windows uses
   Job Objects as 2nd-class path.

5. **jemalloc — known footgun на ARM.**
   [#4422](https://github.com/windmill-labs/windmill/issues/4422) —
   unsupported page size на RPi5 ARM64 потому что jemalloc assumes 4KiB.
   Either don't default-enable jemalloc, или gate by `cfg(target_arch)`.

6. **Postgres queue scales surprisingly far — потом abruptly hits wall.**
   Windmill at 16k stars still uses Postgres. Works fine until HTTP-trigger
   load ([#6434](https://github.com/windmill-labs/windmill/issues/6434))
   или tens-of-thousands of queue rows where `SLEEP_QUEUE` + SKIP LOCKED
   contention bite. They advertise «Kafka/Redis in future» since 2022
   ([#173](https://github.com/windmill-labs/windmill/issues/173)) и never
   shipped. **Lesson:** Postgres-as-queue — fine L2 default. **Абстрагировать
   queue за trait с day 1** — единственное что позволяет add Redis/NATS
   later без re-plumbing every call site.

7. **`deno_core` + `v8` in-process — standard для JS expression eval — и
   QuickJS as «small» fallback has fundamental limits.** Windmill uses
   QuickJS для step input transforms и already has
   [#8073](https://github.com/windmill-labs/windmill/issues/8073): large
   payloads break expression evaluation. If Nebula's expression crate
   ambitious, QuickJS alone не enough at scale.

8. **WASM parsers в frontend для schema inference — clever и portable.**
   Windmill compiles `windmill-parser-py`, `windmill-parser-ts` etc.
   to WASM так editor can do static analysis без backend round-trip.
   **Nebula's validator/expression crates could compile to WASM для same UX win.**

9. **Process-per-job IS durability boundary.** Каждый worker creates
   `/tmp/windmill/{job_id}`, runs под nsjail, captures stdout в DB. No
   shared mutable in-process state. Это lets them restart workers at will.
   **Nebula's sandbox crate should preserve this invariant.**

10. **Big workspace + compile-time SQL checks → slow CI.** Windmill mitigates
    с single big `Cargo.lock`, heavy feature gating, и CI using `sqlx prepare`
    cache. **Nebula's many small crates may compile *faster individually*
    но *slower in aggregate*.**

---

## Что Windmill делает ПРАВИЛЬНО — patterns worth stealing

1. **Script-first, flows compose atomic scripts.** Instead of n8n's
   500-node ecosystem of mega-wrappers, Windmill flow — DAG из tiny scripts.
   Cleaner mental model для determinism, testing, version control.
   **Nebula's Action-as-atomic-unit mirrors this; resist pressure to
   build «mega-Actions».**

2. **Resources typed через schema inference, не hand-authored JSON schemas.**
   Python param `db: MyPostgres` → schema `format: "resource-postgresql"`
   comes из parsing signature. **Nebula's credential schema can follow
   same path: derive из Rust types, don't hand-maintain parallel JSON.**

3. **Suspend/resume as explicit primitive (`wmill.getResumeUrls`).**
   Approval gates — не hack on top of scheduler — это first-class flow
   module. **Nebula should expose suspend as engine primitive.**

4. **Discriminated union для flow modules.**
   `RawScript | PathScript | PathFlow | ForloopFlow | WhileloopFlow | BranchOne | BranchAll | Identity | AiAgent`
   — one enum, all control flow. Maps cleanly to Rust `enum`.

5. **Committed `.sqlx/` cache** makes CI reproducible без database.

6. **Per-language executor modules** в `windmill-worker` — clean boundary,
   each executor owns dep-resolve, lock, install, execute, capture.

7. **AI с per-language system prompts embedding platform conventions.**
   Windmill's Copilot knows как `main` called на each runtime. Nebula's
   AI integration should likewise be platform-aware, не generic code-gen.

---

## Correlation Table

| # | Windmill issue | Theme | Ось | Nebula relevance |
|---|---|---|---|---|
| 1 | [#173](https://github.com/windmill-labs/windmill/issues/173) Kafka/Redis queue | Postgres-queue scale ceiling | 1,2 | Abstract queue trait с day 1 |
| 2 | [#6055](https://github.com/windmill-labs/windmill/issues/6055) duplicate dep jobs | Idempotency в dispatch | 3 | Engine dispatch must be idempotent |
| 3 | [#6434](https://github.com/windmill-labs/windmill/issues/6434) HTTP trigger под load | Trigger→queue path perf | 2,3 | Webhook path latency matters |
| 4 | [#3284](https://github.com/windmill-labs/windmill/issues/3284) tokio_unstable breaks | Async runtime risk | 2 | Don't adopt unstable tokio |
| 5 | [#4422](https://github.com/windmill-labs/windmill/issues/4422) jemalloc ARM | Allocator portability | 2 | Gate jemalloc by arch |
| 6 | [#1836](https://github.com/windmill-labs/windmill/issues/1836) Windows compile | Cross-platform cost | 2 | Pick OS posture explicitly |
| 7 | [#8073](https://github.com/windmill-labs/windmill/issues/8073) QuickJS large payloads | Expression eval limits | 3 | Expression crate size-bound tests |
| 8 | [#8468](https://github.com/windmill-labs/windmill/issues/8468) git sync deleted flow | Git-sync footgun | 3 | Sync needs dry-run + confirm |
| 9 | [#6718](https://github.com/windmill-labs/windmill/issues/6718), [#4907](https://github.com/windmill-labs/windmill/issues/4907) worker liveness | Worker heartbeat flake | 3 | Worker health must be observable |
| 10 | [#7619](https://github.com/windmill-labs/windmill/issues/7619) intentional CE vuln | OSS/EE trust | 1 | Don't seed CE с deliberate vulns |
| 11 | [#3374](https://github.com/windmill-labs/windmill/issues/3374) editor unresponsive | Canvas perf | 3 | Visual builder needs virt scroll |
| 12 | [#5822](https://github.com/windmill-labs/windmill/issues/5822) editor memleak | Long sessions | 3 | Event-listener hygiene |
| 13 | [#2584](https://github.com/windmill-labs/windmill/issues/2584) dev setup docs | DX | 2 | Dev-setup tested в CI |
| 14 | [#6537](https://github.com/windmill-labs/windmill/issues/6537) offline deploy | Air-gap | 2 | Dep bundle story с day 1 |
| 15 | [#1564](https://github.com/windmill-labs/windmill/issues/1564) corporate CAs | Enterprise self-host | 2 | rustls custom-CA path |
| 16 | [#6408](https://github.com/windmill-labs/windmill/issues/6408) pool timeout flow update | sqlx pool sizing | 2,3 | Pool tuning docs |
| 17 | [#8037](https://github.com/windmill-labs/windmill/issues/8037) CWE-639 ownership | Authz | 3 | Every mutation checks ownership |
| 18 | [#5014](https://github.com/windmill-labs/windmill/issues/5014) open letter | OSS/EE posture | 1 | Dual-license comms clarity |
| 19 | [#5337](https://github.com/windmill-labs/windmill/issues/5337) rework dedicated workers | Worker types proliferate | 1,2 | Worker roles fixed early |
| 20 | [#6879](https://github.com/windmill-labs/windmill/issues/6879) Postgres 5s fail | Connection test UX | 2,3 | Connection test с progress |

---

## Quick Wins для Nebula (10)

1. **Commit `.sqlx/` offline query cache.** CI builds без live DB.
   Cheap; Windmill-proven.
2. **Abstract queue behind trait (`Dispatcher`) даже если PG — only impl.**
   Windmill «Kafka/Redis soon» 4 years. Don't repeat.
3. **Gate jemalloc behind `cfg(target_arch = "x86_64")`** или
   `cfg(not(target_arch = "aarch64"))`.
   [#4422](https://github.com/windmill-labs/windmill/issues/4422) —
   100% avoidable bug.
4. **Model flow control as `FlowModule`-style enum с start.**
   Discriminated union maps to Rust `enum` naturally; DAG code gets clean match arms.
5. **Make suspend/resume first-class engine primitive с URL-scheme**, не
   per-action flag. Windmill `getResumeUrls` pattern clean.
6. **Inference-driven schema для credentials/resources.** Derive из Rust
   types; do not hand-author parallel JSON Schema.
7. **One process per job — durability boundary.** Keep worker stateless
   между jobs. Enables worker-restart-as-recovery.
8. **Per-runtime cargo features, aggressive `#[cfg(feature = ...)]` gating.**
   Prevents «worker ships every language» если deploy uses только one.
9. **Avoid `tokio_unstable` в default builds.**
   [#3284](https://github.com/windmill-labs/windmill/issues/3284).
   If needed, gate by feature.
10. **Git-sync must have dry-run + diff-preview.**
    [#8468](https://github.com/windmill-labs/windmill/issues/8468) —
    silent deletion из `push` brutal. **Nebula's equivalent (any «sync
    workflow to disk/VCS») must show что будет destroyed before touching.**

---

## Sources

- **Repo:** https://github.com/windmill-labs/windmill
  (stars 16,274; forks 927; 704 open issues; AGPLv3 + proprietary `enterprise` flag)
- **`backend/Cargo.toml`, `backend/Cargo.lock`** (crate list, tokio/sqlx versions)
- **DeepWiki** `windmill-labs/windmill` Overview + crate-level wiki
- **Issues** referenced в §1/§3/§6/§7 (все links verified 2026-04-20)
- **Windmill OpenFlow spec** (referenced в `FlowBuilder.svelte` + FlowModule types)
- **Windmill `README.md`** (mentions future Kafka/Redis, nsjail optional, deno_core)
- **`LICENSE` + `NOTICE`** files на CE/EE boundary
- **`.sqlx/` directory** — evidence of committed offline query cache
- **`job_helpers_oss.rs`** — evidence что Parquet helpers stubbed в OSS, present в EE
