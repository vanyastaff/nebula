# Nebula-resource Redesign Cascade — Log

**Started:** 2026-04-24
**Worktree:** `.worktrees/nebula/vigilant-mahavira-629d10`
**Branch:** `claude/vigilant-mahavira-629d10`
**Orchestrator:** main session (flat coordination — no recursive orchestrator dispatch)
**Input prompt:** Hands-off Redesign Cascade Orchestrator Prompt (user-provided, paste-in-session confirmed 2026-04-24)

## Budget

- Hard stop: 5 days agent-work equivalent OR first irresoluble blocker
- Consensus: max 3 rounds per protocol before escalation
- Spike: max 2 iterations
- Checkpoint review: max 2 rounds per CP

## Initial agent roster (read from `.claude/agents/`)

- architect — drafts long-form design docs with checkpoint cadence
- devops — CI/cargo deny/MSRV/benches/build infra
- dx-tester — newcomer API ergonomics smoke-test
- orchestrator — consensus protocol picker (NOT invoked — main session coordinates flat)
- rust-senior — idiomatic patterns, safety, perf, correctness
- security-lead — credential encryption, auth, sandboxing, input validation
- spec-auditor — document structural integrity (cross-refs, drift, bookkeeping)
- tech-lead — priority calls, trade-offs, cross-crate coordination

## Input reference artefacts verified

| Path | Status |
|---|---|
| `crates/resource/src/` | present — module tree includes recovery/, runtime/, topology/, topology_tag.rs |
| `crates/resource/docs/*.md` | present — Architecture, Pooling, adapters, README, api-reference, events, recovery |
| `docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md` | present (CP3) |
| `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` | present (CP5) |
| `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` | present |
| `docs/superpowers/specs/2026-04-24-credential-3agent-consensus-session.md` | **NOT FOUND** — referenced in prompt but absent |
| `docs/adr/0035-phantom-shim-capability-pattern.md` | present (amended) |
| `docs/tracking/credential-concerns-register.md` | present |

## Timeline

### 2026-04-24 T+0 — Cascade start

- User confirmed session-level launch
- Initial recon: path verification complete
- Phase tracking tasks created (9 tasks — Phase 0 through Phase 8)
- CASCADE_LOG.md initialized

### 2026-04-24 T+~15min — Phase 0 gate PASSED

- rust-senior audit complete: `phase-0-code-audit.md` (1010 L, commit d6cee19f, 86 tool uses, ~9 min)
- devops audit complete: `phase-0-manifest-audit.md` (336 L, commit d6cee19f, 55 tool uses, ~4 min)
- Consolidation `01-current-state.md` written
- Audits were consistent — no architect mediation needed
- Gate: PASSES

**Phase 0 findings (summary — detailed files lost in filesystem event; see soft-escalation below):**

1. **Code is entirely v2.** Zero v1 symbols (`HookRegistry`, `QuarantineManager`, `HealthChecker`, `HealthPipeline`, `EventBus`, `HealthStage`, `ConnectivityStage`, `AutoScaler`, `Poison`, `DependencyGraph`, etc.) exist in `src/`. `docs/Architecture.md` describes vanished v1 module map.
2. **`runtime/` + `topology/` are paired layers, NOT duplicates** — `topology/` = trait+config, `runtime/` = instance state. Each of 7 topologies has a pair.
3. **`TopologyTag` is a concrete runtime `#[non_exhaustive] enum`, NOT a phantom-type tag** (brief was wrong). Stored at runtime on `ResourceGuard`. Zero `PhantomData<TopologyTag<...>>` anywhere.
4. **`deny.toml` has zero layer-enforcement rule for `nebula-resource`** despite 5 consumers spanning business (action/sdk/plugin) and exec (engine/sandbox) tiers.
5. **Resource crate has `features = { default = [] }`** and zero flags despite pulling heavy deps (nebula-telemetry, nebula-metrics, nebula-resilience).
6. **No `benches/` directory, not in CodSpeed shard** — no perf gate for runtime-critical crate.
7. **No external `nebula-resource-*` adapter crates** anywhere — `adapters.md` is purely aspirational.
8. **Doc drift at multiple layers** — Architecture.md = v1, README case-mismatch filenames, adapters.md API signatures out-of-date, events.md variant count wrong (lists 7, actual 10).
9. **🔴 panic surface identified** — `Manager::on_credential_refreshed` / `on_credential_revoked` `todo!()`-panic (manager.rs:1378, 1400). (Phase 1 later corrected threat characterization — see below.)

### 2026-04-24 T+~35min — Phase 1 gate PASSED (easily)

- 4 parallel agents dispatched and completed:
  - dx-tester ~680s, 85 tools, 18 severity rows
  - security-lead ~575s, 72 tools, 22 findings
  - rust-senior ~698s, 94 tools, 24-row matrix
  - tech-lead ~429s, 65 tools, 7 sections + priority preview
- Consolidation `02-pain-enumeration.md` written (canonical Phase 1 deliverable)
- `01-current-state.md` §3.1 **corrected** per Phase 1 security-lead finding

**Gate verdict:** PASSES easily. 6 🔴 / 9 🟠 / 9 🟡 / 3 🟢 / 1 ✅. Escalation threshold (0 🔴 AND <3 🟠) not even close to triggering.

**Convergent themes (cited by 2+ agents):**
1. 🔴 **Credential×Resource seam is structurally wrong** — primary driver. `Resource::Auth` is dead weight (100% `()` usage); Tech Spec §3.6 designs different shape; reverse-index never populated → silent revocation drop today + latent `todo!()` panic if write added without dispatcher.
2. 🔴 **Daemon topology has no public start path** — `ManagedResource.topology` pub(crate); `Manager::register(daemon)` works but user cannot reach `DaemonRuntime::start()`.
3. 🔴 **Doc surface is broken** — `api-reference.md` ~50% fabrication rate, `adapters.md` 4/7 compile-fail blocks, `dx-eval-real-world.rs` references nonexistent `nebula_resource::Credential`.
4. 🔴 **Drain-abort phase corruption** — `graceful_shutdown::Abort` flips phase back to `Ready` without recording failure; fix helper `ManagedResource::set_failed()` dead-coded. **Standalone-fix PR candidate SF-2.**
5. 🟠 **`manager.rs` surface is the god-object, not the type.** 2101 L file: split file, keep type.
6. 🟠 **Daemon + EventSource are canon-out-of-band** (§3.5 defines resource = pool/SDK client). Extraction recommended.
7. 🟠 **Reserved-but-unused public API** (`AcquireOptions::intent/.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`).

**Standalone-fix PR candidates (outside cascade):**
- **SF-1:** `deny.toml` wrappers rule for nebula-resource (security-lead, mechanical, CI-enforceable)
- **SF-2:** Drain-abort phase corruption — wire `ManagedResource::set_failed()` in `graceful_shutdown::Abort` (rust-senior, one-function fix)

**Phase 0 corrections captured in `01-current-state.md`:**
- `credential_resources` is NEVER written (not "populated at register" as Phase 0 said)
- Tech Spec §3.6 (not §15.7) is the rotation-hook design reference
- Migration scope is in-tree only (no external adapters) — brief's deprecation-window machinery is over-engineered for 5 internal consumers

**5 open questions for Phase 2 co-decision:**
1. `Auth` → `Credential` reshape: drop entirely, make optional (`AuthenticatedResource` sub-trait), or keep current?
2. Topology count: 5 (extract Daemon/EventSource), 6 (merge Service/Transport), or keep 7?
3. `Runtime` vs `Lease`: collapse (9/9 tests prove `Runtime == Lease`), default, or keep separate?
4. `AcquireOptions::intent/.tags`: remove, defer, or wire up this cascade?
5. `manager.rs` split: file-split only (tech-lead's default)?

**Budget usage so far:** ~36 min wall / ~68 min agent-effort. Well inside 5-day envelope.

### 2026-04-24 T+~45min — SOFT ESCALATION: filesystem loss of per-agent findings files

**Symptom:** during Edit of `01-current-state.md` and `CASCADE_LOG.md` after Phase 1 consolidation, Edit tool reported "File does not exist." Filesystem inspection shows only `02-pain-enumeration.md` survived from the draft directory. All other files (CASCADE_LOG.md, 01-current-state.md, phase-0-*.md, phase-1-*-findings.md, scratch/probe-*.md) are missing.

**Hypothesis:** Agent subagents ran in isolated worktrees (per teammate-mode `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`); when those worktrees were cleaned up, the untracked files they created in `docs/superpowers/drafts/` may have been swept. The exact mechanism is unclear — worth investigating post-cascade.

**Recovery:**
- `02-pain-enumeration.md` is the canonical Phase 1 deliverable and survived — **no material loss to Phase 2+ dispatch**.
- `01-current-state.md` reconstructed from orchestrator context with Phase 1 §3.1 correction applied.
- `CASCADE_LOG.md` (this file) reconstructed from context — **Phase 0 + Phase 1 summaries captured above**.
- Per-agent findings files (phase-0-code-audit.md, phase-0-manifest-audit.md, phase-1-*-findings.md) are unrecoverable from this session. Their key findings are captured in `02-pain-enumeration.md`.

**Mitigation going forward:**
- **Commit after every phase gate.** Artefacts go into git history immediately so they survive worktree cleanup.
- Phase 2+ dispatch prompts will emphasize "file path = absolute path in main worktree, not in isolated worktree" — though this may not be sufficient if the cleanup mechanism affects the main tree.
- If loss recurs for a critical artefact, escalate to hard ESCALATION.md rather than soft.

**Next:** Phase 2 — scope narrowing co-decision. architect proposes 2-3 scope options, tech-lead priority-call, security-lead security-gate block. Max 3 rounds.

---

*This log is append-only during cascade. Each phase gate adds an entry. Soft escalations logged prominently; hard escalations also write `ESCALATION.md` at repo root.*
