# Tianshu-rs — Architectural Issues

## Issue count

Total open issues: 2. Total closed issues: 0. This project is pre-adoption — no external bug reports yet.

Tier 2 requirement: ≥3 cited issues for projects with >100 closed issues. Tianshu has 0 closed issues, so this gate does not apply.

---

## Issue #8 — feat(workflow_engine): recursive sub-process depth limiting

**URL:** https://github.com/Desicool/Tianshu-rs/issues/8
**State:** Open
**Reactions:** 0 (self-filed issue)

**Summary:** Proposes adding `max_depth` to `WorkflowContext` (default 3) to prevent unbounded recursive sub-process spawning. `spawn_child()` would return `SpawnResult::Spawned(handle)` or `SpawnResult::DepthLimitReached { request_id, situation }`. When depth is exceeded, the situation is persisted and the workflow can return `Waiting` on a `depth_approval` poll — allowing a human-in-the-loop to approve increasing the depth limit.

**Architectural relevance:** This is a correctness/safety concern for multi-agent workflows. Without depth limiting, a workflow spawning children that spawn children can recurse unboundedly, exhausting database connections and memory. The proposed fix requires a **breaking API change** (`Result<ChildHandle>` → `Result<SpawnResult>`). This is a direct consequence of the project's decision to provide sub-workflow spawning without safety guards.

---

## Issue #2 — feat: add workflow examples — ReAct, plan-and-execute, tool orchestration, multi-agent swarm, conversation agent

**URL:** https://github.com/Desicool/Tianshu-rs/issues/2
**State:** Open
**Reactions:** 0 (self-filed issue)

**Summary:** Proposes adding five workflow examples covering common agent patterns: ReAct (reason+act loop), plan-and-execute, tool orchestration pipeline, multi-agent swarm (leader/worker with session-state coordination), and conversation agent (stateful multi-turn with per-turn checkpointing).

**Architectural relevance:** The absence of these examples indicates the LLM agent patterns (while supported by the engine) have not been validated end-to-end by the author. The multi-agent swarm example would exercise the session-state coordination mechanism (known to have a last-write-wins race condition). The plan-and-execute example would test the sub-workflow spawning + depth limiting scenario from issue #8.
