# EXECUTIVE BRIEF — Nebula vs 27 Rust Workflow/Orchestration Projects

**Дата**: 2026-04-26 · **Объём исходного материала**: ~140K слов (27 architecture.md) · **Срез**: 21 архитектурная ось, deep-dive по A3/A4/A5/A11/A12/A21.

---

## TL;DR (одна страница)

**Nebula держит две архитектурные позиции, на которых вся индустрия позади**: (1) credential subsystem — **0 из 27 конкурентов** имеют сравнимую глубину; (2) TypeDAG L1-L4 + 5-kinded sealed action taxonomy — **0 из 27** дотягиваются до этого уровня type safety. Plugin sandbox + Plugin Fund commercial model уникальны как **спецификация**, но не как реализация — **никто не отгрузил настоящую capability-based изоляцию плагинов** (включая Nebula).

**Главная угроза**: AI/LLM перешёл в **expected feature**. 7 из 27 проектов отгружают first-class AI integration в Q2 2026 (z8run/runtara/tianshu/rayclaw/cloudllm/aofctl/orchestral). Defensive bet Nebula ("AI = generic actions + plugin LLM client + Surge") архитектурно корректен, но требует двух конкретных core-engine инвестиций в ближайшие 6-12 недель, иначе AI-story Nebula отстанет на 12-18 месяцев.

**Концентрат рекомендаций (приоритезированно)**:
1. ⭐⭐⭐ **`nebula-mcp` binary** — 1-2 недели — competitive parity с flowlang/runtara; делает Nebula AI-controllable извне.
2. ⭐⭐⭐ **Replay-safe LLM history events** — 2-4 недели — durable AI workflows из коробки; защищает durability promise при добавлении LLM плагинов.
3. ⭐⭐⭐ **MVP capability enforcement в WASM sandbox** — 4-8 недель — превращает spec в реальный moat; z8run показал, что декларированные-но-не-enforced capabilities хуже отсутствия capabilities.
4. ⭐⭐ **`Supervisor` primitive в nebula-resilience** — 1-2 недели — закрывает дыру в plugin crash recovery (заимствуется из aofctl).
5. ⭐⭐ **Audit log для credential read operations** (не только refresh) — 1-2 недели — SOC 2 prerequisite.

Всё остальное — пуш в EXECUTIVE_BRIEF.md второго порядка или backlog.

---

## 1. Стратегический ландшафт (где Nebula стоит)

### 1.1 Карта 27 проектов

Из 27 проектов **22 не являются полноценными workflow engines в смысле Nebula** — они либо узкоспециальные (durable-lambda-core, raftoral), либо библиотеки одного абстрактного слоя (orka, dagx, runner_q), либо вообще из другого домена (kotoba-workflow, fluxus, ebi_bpmn, aqueducts-utils, dataflow-rs). Это объясняет почему 0 из 27 имеют credential layer и 0 из 27 имеют resource lifecycle: они **не ставят перед собой задачу платформы оркестрации**, только executor / library.

**Прямые конкуренты** (полноценные orchestration platforms): **z8run, runtara-core, tianshu, acts, aofctl, orchestral**. 6 проектов. Из них 5 имеют first-class AI; ни один не имеет ничего похожего на credential subsystem; 1 (z8run) имеет уязвимость в credential vault (нет user_id).

### 1.2 Уникальные моаты Nebula (подтверждены данными)

| Ось | Nebula | Industry | Кому ещё близко |
|-----|--------|----------|-----------------|
| **A4 Credentials** | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol blanket adapter | 0/27 имеют credential subsystem | никому |
| **A5 Resources** | 4-scope, ReloadOutcome, generation tracking, on_credential_refresh | 0/27 имеют resource lifecycle | никому |
| **A17 Type safety** | sealed/GAT/HRTB/typestate/Validated<T> | большинство type-erased через `serde_json::Value` | dagx (typestate cycle prevention в одной оси) |
| **A14 Multi-tenancy** | nebula-tenant 3 isolation modes + RBAC + SSO/SCIM план | 0/27 имеют multi-tenancy | никому |
| **A13 3-mode deployment** | desktop / self-hosted / cloud из одной кодовой базы | 0/27 имеют 3-mode | никому |
| **A20 Plugin Fund commercial model** | royalties to plugin authors | 0/27 имеют commercial model для plugin authors | никому |

**Стратегический вывод**: моаты Nebula — **enterprise-ready features** (credentials, multi-tenancy, RBAC). Это **продаваемые** в регулируемые отрасли: финансы, healthcare, government. Marketing collateral систематически недосчитывает A4 как differentiator — все конкуренты имеют либо ноль, либо уязвимость.

### 1.3 Где индустрия впереди (или равна)

| Ось | Лидер(ы) | Nebula | Gap |
|-----|----------|--------|-----|
| **A21 AI/LLM** | runtara-core (first-class AiAgent + MCP), tianshu (LangGraph-alt), aofctl (5 fleet modes), cloudllm (7 modes) | none yet | **shipping vs planning** |
| **A19 Testing** | duroxide (generic Provider validation suite — 20 modules) | nebula-testing crate | duroxide's design сильнее — заслуживает заимствования |
| **A6 Resilience (Supervisor)** | aofctl (`Supervisor` primitive) | retry/CB/bulkhead/timeout/hedging — но нет агент-crash recovery | gap для plugin/agent crashes |

### 1.4 Convergent-patterns observed (3+ независимых реализаций)

Когда 3 проекта независимо доходят до одного паттерна, это **сильный индустриальный сигнал**:

1. **Conversation memory + compaction strategy enum** — runtara (SlidingWindow/Summarize), tianshu (2 strategies), cloudllm (Trim/SelfCompression/NoveltyAware). **Сигнал**: для plugin LLM client минимум — 3-стратегий enum.

2. **Provider/Backend trait pattern** — duroxide (Provider trait), raftoral (RocksDB column families abstraction), runner_q (storage backend), durable-lambda-core (DurableBackend trait + MockBackend), cloudllm (ClientWrapper). **Сигнал**: trait-based isolation для тестируемости через generic validation suite — заимствуется duroxide-style.

3. **MCP integration** (6 проектов) — runtara-core (rmcp 1.2 native), flowlang (flowmcp binary), rayclaw, aofctl, orchestral, cloudllm. **Сигнал**: workflow engines становятся AI-controllable через standardized protocol — это core platform concern, не plugin-level.

---

## 2. AI/LLM landscape (самый стратегический срез)

### 2.1 Шесть архитектурных паттернов наблюдаемых сегодня

| Паттерн | Шипится в | Подходит ли Nebula |
|---------|-----------|---------------------|
| **Node-based AI** (LLM как built-in node) | z8run (10 nodes) | ✓ через generic actions + plugin LLM client (current bet) |
| **First-class step + MCP server** (AiAgent step + платформа AI-controllable) | runtara-core | partial — `nebula-mcp` нужен core-level |
| **Coroutine-replay durability** (`ctx.step()` checkpoint) | tianshu | ✗ конфликтует с DAG model — не для Nebula |
| **LLM-as-scheduler / planner** (LLM решает что вызвать) | rayclaw, orchestral | ✗ конфликтует с typed-DAG philosophy |
| **Multi-agent fleet coordination** (Hierarchical/Peer/Swarm/Pipeline/Tiered) | aofctl, cloudllm | ✗ Surge concern, не engine concern |
| **Replay-safe LLM events** (LlmRequested/LlmCompleted в history) | duroxide proposal, raftoral close-fit | ⭐ **defensive bet — Nebula должна забрать** |

### 2.2 Зачем Nebula нужны (1) MCP server и (2) replay-safe LLM events

**MCP server (`nebula-mcp` binary)**:
- 6 из 27 проектов уже имеют MCP integration
- Это **core platform feature**, не plugin — эксponируется снаружи Claude Desktop / Cursor / Cline / другие AI clients
- Технически дешёвая реализация: `tools/list` + `tools/call` over stdio JSON-RPC, expose registered actions
- Конкретный effort: 1-2 недели
- Strategic return: zero-cost competitive parity + AI-driven workflow construction unlocked

**Replay-safe LLM history events**:
- duroxide's documented proposal: `LlmRequested` / `LlmCompleted` как первоклассные events в execution log
- Гарантирует exactly-once LLM semantics при replay — критично для durable AI workflows (cost runaway risk при наивном replay)
- Поддерживает Nebula's existing append-only execution log architecture (расширение, не передизайн)
- Effort: 2-4 недели для core schema; plugin LLM client потом строится поверх
- Strategic return: durable AI workflows из коробки + защита durability promise при добавлении LLM плагинов

### 2.3 Что AI-strategy Nebula НЕ должна делать

- ✗ **Не отгружать 6-й sealed action kind для AI**. Plugin LLM client + replay-safe events закрывает use case без erosion of таксономии.
- ✗ **Не строить multi-agent fleet modes в core**. cloudllm (7 modes) и aofctl (5 modes) — высокоуровневая оркестрация, должна жить в Surge.
- ✗ **Не приоритезировать RAG primitives**. Только rayclaw имеет (sqlite-vec); большинство deferring к user-code. Plugin-level concern.
- ✗ **Не тратить недели на выбор "правильного" LLM provider abstraction**. Универсальная конвергенция: 3-5 методов (complete, complete_stream, complete_with_tools), 4-9 providers, OpenAI + Anthropic + Ollama-local — минимум.

---

## 3. Plugin sandbox: spec ahead, implementation behind

### 3.1 Industry reality

**Только 1 из 27 проектов** отгрузил настоящую plugin EXEC isolation: **aofctl** через Docker (bollard). Это reasonable для DevOps tool, но heavyweight.

**z8run** попытался WASM + capabilities, но **enforcement layer не работает** — capabilities (network/filesystem/memory_limit_mb) декларируются в манифесте, но Linker не имеет WASI imports. Sandbox запускает WASM, но плагин не может ничего делать (что technically safe, но делает capability declarations meaningless — пользователи могут строить ложное чувство безопасности).

**emergent-engine**: subprocess + git-repo registry; SHA256 verification код существует, но `#[allow(dead_code)]` — НЕ вызывается при install. Issue #25 explicitly tracks missing sandbox.

**acts**: `acts.transform.code` запускает arbitrary JS через QuickJS, `acts.app.shell` — arbitrary shell. **Zero sandboxing.**

### 3.2 Что это значит для Nebula

**Хорошая новость**: spec Nebula plugin-v2 (WASM + capability-based + Plugin Fund commercial) — **architecturally correct и strategically differentiated**. Никто конкурент не догнал.

**Плохая новость**: **никто не догнал, потому что enforcement — это hard work**. z8run отгрузил декларации без enforcement и получил false security. Nebula не должна повторить.

**Конкретная рекомендация**: ship MVP capability enforcement до расширения spec. Минимально жизнеспособный набор:
- `network: deny | allow_list[host]`
- `filesystem: deny | tmp_only | allow_list[path]`
- `memory_limit_mb: u64`
- `wall_time_ms: u64`

Каждое — concrete WASI import implementation в wasmtime Linker. Тест: violations actually trap plugin (не просто log warning). Effort: 4-8 недель. Это **активирует мoat**.

Дополнительно: **MCP subprocess как secondary plugin transport** — для polyglot plugins (Python/Node.js/Go) и LLM tool integrations. WASM transport — для compute-heavy sandboxed plugins. Двойная transport стратегия покрывает оба use case без compromise security model.

---

## 4. Top-N findings (cross-cutting strategic observations)

1. **AI/LLM convergence на MCP**. 6 из 27 проектов имеют MCP integration в одной из форм (server, bridge, или client). Это **самый сильный convergent industry signal** в исследовании. Workflow engines становятся AI-controllable через standardized protocol.

2. **Credential subsystem — пустыня индустрии**. Из 27 проектов: 0 имеют сравнимую с Nebula глубину; 1 имеет уязвимость (z8run vault no user_id); 1 имеет toy реализацию (acts 7-line plaintext JS global); остальные 25 — env vars или вообще ничего. Это уникальный moat.

3. **Type erasure — индустриальная норма, не bug**. ~18 из 27 проектов используют `serde_json::Value` или эквивалент для I/O. Только Nebula и dagx имеют типизированные ports на compile-time. Это **позиционирование по сложности vs accessibility**: Nebula выбрала сложность за compile-time safety, индустрия выбрала простоту через type erasure.

4. **Durable execution patterns конвергируют на history replay**. Temporal, duroxide, durable-lambda, raftoral — все история-based. Nebula's frontier-based scheduler — отличная альтернатива, но **проверь, что append-only execution log можно использовать для LLM events** (см. секцию 2.2).

5. **Plugin sandbox — industry weakness, не уникальная проблема Nebula**. Если Nebula отгрузит MVP capability enforcement, **она будет одна из лучших в категории**. Если нет — позиция не лучше z8run.

6. **`async_trait` macro vs RPITIT** — индустрия в transitional моменте. async_trait всё ещё доминирует (acts, dataflow-rs, fluxus, aofctl, runner_q), но durable-lambda-core полностью на RPITIT, duroxide частично. Nebula уже на RPITIT (per CLAUDE.md / idiom currency feedback memory) — это convergent с new direction индустрии.

7. **Non-functional requirements (resilience, observability) — undersold differentiators**. nebula-resilience (retry/CB/bulkhead/timeout/hedging unified) — никто кроме aofctl не близко (и тот только Supervisor). OTel тracing в Nebula vs duroxide's metrics facade pattern — duroxide проще для пользователя, Nebula богаче на distributed traces; **обе модели валидны**, можно reframe маркетинг.

8. **Solo-maintainer reality**. 24 из 27 проектов имеют solo maintainer. Это не специфическая проблема Nebula. Bus factor — universal industry concern.

---

## 5. Threats / Opportunities brief

### 5.1 Threats

| Threat | Severity | Mitigation |
|--------|---------:|------------|
| AI/LLM gap relative to z8run/runtara/tianshu/aofctl widens to 12-18 months | high | ship MCP server (1-2w) + replay-safe LLM events (2-4w) within Q3 |
| Plugin v2 spec остаётся spec без MVP enforcement → credibility risk | medium | scope down to 4 capabilities, ship enforcement + tests Q3-Q4 |
| Solo maintainer bus factor | constant | unchanged from baseline |
| z8run improves on AI lead while fixing security gaps (vault user_id, cron, perf bugs) | medium | none required — z8run still 12-18m behind on infrastructure axes |
| Surge / ACP project rooted in agent-orchestration space could be confused with Nebula's positioning | low | clearer Surge/Nebula boundary in docs |

### 5.2 Opportunities

| Opportunity | Magnitude | Effort |
|-------------|----------:|-------:|
| Marketing pivot — credential subsystem as enterprise-sales hook | high | 1 week of marketing copy + 1 verified-with-security-lead claim sheet |
| Vault adapter pattern (cache+refresh against HashiCorp Vault / AWS Secrets Manager) | medium | 2-4w per first adapter |
| MCP server `nebula-mcp` — zero-cost AI ecosystem entry | high | 1-2w |
| Replay-safe LLM events as durable AI workflows differentiator (duroxide proposed but no one shipped) | high | 2-4w |
| `Supervisor` primitive in nebula-resilience for plugin crash recovery | medium | 1-2w |
| Provider validation suite (duroxide-style) for nebula-testing's resource contracts | medium | 2-3w |
| Lockfile-in-manifest for Plugin Fund prerequisite | medium | 1-2w |

---

## 6. Roadmap implications + recommended ADRs

### 6.1 Q3 2026 (next quarter) — recommended

| Item | Effort | Strategic value |
|------|-------:|-----------------|
| `nebula-mcp` binary (MVP: tools/list + tools/call) | 1-2w | ⭐⭐⭐ |
| Replay-safe LLM history events (schema + engine integration) | 2-4w | ⭐⭐⭐ |
| MVP capability enforcement in WASM sandbox (4 capabilities) | 4-8w | ⭐⭐⭐ |
| `Supervisor` primitive in nebula-resilience | 1-2w | ⭐⭐ |
| Credential audit logging extension (read events, not just refresh) | 1-2w | ⭐⭐ |
| Marketing copy update — credential subsystem advantage | 1w | ⭐⭐ |

Total Q3: ~10-19 weeks (parallelizable). Realistic given solo maintainer: 2-3 of these in Q3, остальные перенести в Q4.

**Приоритезированный shortlist для Q3**: `nebula-mcp` + replay-safe LLM events + Supervisor primitive (~5-8 weeks combined). Capability enforcement переносить в Q4 как 1.5-2 month focused effort.

### 6.2 Recommended ADRs before implementation

1. **ADR — MCP server as first-class platform feature**. Должна Nebula быть AI-controllable через MCP? Где живёт `nebula-mcp` (отдельный binary vs feature flag в `nebula serve`)? Какой subset registered actions exposed? Read-only vs read-write?

2. **ADR — Replay-safe LLM events schema**. Что именно пишется в history? Только request/response, или включая tool-calls? Token counts? Cost? Совместимо ли с existing execution log schema?

3. **ADR — Plugin transport bifurcation: WASM primary + MCP secondary**. Когда какой transport использовать? Capability declaration схожа? Plugin Fund licensing — на какой transport applies?

4. **ADR — Capability enforcement scope (V0)**. Какой минимально жизнеспособный set capabilities для MVP? Какие тестируемые failure modes (negative tests показывающие что violations trap)?

5. **ADR — Credential audit surface для SOC 2 path**. Какие события (create / read / update / refresh / revoke) логируются? Куда? Sampling? Retention?

### 6.3 Q4 2026 + (longer horizon)

- Capability enforcement maturation (8+ capabilities, fine-grained policies)
- Vault adapter (HashiCorp / AWS Secrets Manager)
- Provider validation suite-style test infrastructure для nebula-resource contracts
- MCP subprocess transport как secondary plugin transport (после WASM enforcement отгружен)
- `(name, version)` runtime versioned action dispatch ADR
- Plugin Fund commercial implementation (signing, licensing, registry)

### 6.4 Не делать

- ✗ Не добавлять 6-й sealed action kind для AI (см. §2.3)
- ✗ Не строить multi-agent fleets в core (Surge concern)
- ✗ Не отгружать plugin v2 spec без enforcement (z8run lesson)
- ✗ Не diluteить credential moat lite-mode для desktop (см. A4 axis file)
- ✗ Не replicate emergent's git-repo registry без security model

---

## 7. Reference materials

- `findings/_orchestrator/synthesis/master-matrix.md` — full 28-row × 22-axes matrix
- `findings/_orchestrator/synthesis/axes/A21-ai-llm.md` — 6 patterns deep
- `findings/_orchestrator/synthesis/axes/A11-plugin.md` — BUILD + EXEC analysis
- `findings/_orchestrator/synthesis/axes/A4-credentials.md` — credential subsystem moat
- `findings/_orchestrator/synthesis/axes/A3-action.md` — sealed taxonomy comparison
- `findings/<project>/architecture.md` — 27 per-project deep dives (~140K words total)
- `findings/_orchestrator/completions/<project>.md` — 27 digest summaries

---

**Конец executive brief.** Стратегические решения остаются за tech-lead'ом. Этот документ — карта местности и rate-of-change анализ; не план execution. Каждый roadmap item требует своего ADR review с security-lead / rust-senior input.
