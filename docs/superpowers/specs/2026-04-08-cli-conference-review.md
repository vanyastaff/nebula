# CLI Conference Review — 2026-04-08

> 5 agents (SDK User, DX Tester, Tech Lead, DevOps Engineer, UX Designer) reviewed Nebula CLI against Temporal, Prefect, Airflow, Conductor, n8n, gh, docker, kubectl.

## DX Score: 6/10

Foundations solid. Validate+run path works. Gaps in action testing, exit codes, plugin loading, and workflow verbosity.

---

## Consensus Findings (agreed by 4-5/5 agents)

### P0 — Must fix now

| # | Finding | Agents | Effort |
|---|---------|--------|--------|
| 1 | **BUG: `nebula run` exits 0 on workflow failure.** CI cannot trust exit codes. `run.rs` does not inspect `result.status`. | 5/5 | 1h |
| 2 | **Richer exit codes:** 0=success, 1=general error, 2=validation failed, 3=timeout, 5=parse/file error | 4/5 | 2h |
| 3 | **TTY-aware format:** default to `text` in terminal, `json` when piped. Like docker/kubectl/gh. | 4/5 | 1h |
| 4 | **Add `--quiet` / `-q` global flag.** Essential for CI scripting. | 5/5 | 1h |

### P1 — High impact, do next

| # | Finding | Agents | Effort |
|---|---------|--------|--------|
| 5 | **`--dry-run` on `run`.** Every ops tool has it (terraform plan, kubectl --dry-run). Validate + show execution plan. | 5/5 | 3h |
| 6 | **`--input-file <path>` flag.** Complex JSON without shell escaping. Support `-` for stdin. | 3/5 | 30m |
| 7 | **`--format` on ALL commands.** `actions info` and `config show` lack it. Consistency. | 4/5 | 2h |
| 8 | **`nebula action test <key> --input '{}'`.** Test single action without workflow file. Killer feature for plugin devs. | 3/5 | 3h |
| 9 | **Reduce workflow YAML boilerplate.** Auto-generate id, owner_id, timestamps for local `run`. Minimal should be ~7 lines, not 19. | 3/5 | 4h |
| 10 | **`validate` supports directory/glob.** `nebula validate workflows/` for CI lint step. | 2/5 | 2h |

### P2 — Polish

| # | Finding | Agents | Effort |
|---|---------|--------|--------|
| 11 | **Remove `version` subcommand.** Redundant with `--version`. | 3/5 | 5m |
| 12 | **Fix `actions` vs `action` plurality.** Pick one (prefer singular: `action list`). | 2/5 | 30m |
| 13 | **Flatten `dev action new` → `dev new-action` or `dev scaffold action`.** 3 levels too deep. | 2/5 | 30m |
| 14 | **Add `--color auto\|always\|never` global flag.** Color status in text output. | 2/5 | 3h |
| 15 | **Fix `--stream` flag.** Currently prints one line. Either implement real streaming or remove. | 1/5 | 4h |
| 16 | **`dev init --with-config`.** Also generate `nebula.toml` during project init. | 1/5 | 30m |

---

## Critical Bugs Found

### BUG 1: Exit code 0 on workflow failure
```rust
// run.rs — current (broken)
let result = engine.execute_workflow(&definition, input, budget).await?;
// prints result, returns Ok(()) regardless of result.status
```
A workflow with Failed status exits 0. Every CI pipeline silently passes broken workflows.

### BUG 2: `--stream` flag is a lie
Described as "stream node progress" but only prints `"Executing..."` once before execution. No actual streaming of node completion events.

### BUG 3: `dev action new` generates broken Cargo.toml
Uses `nebula-action = { version = "0.1" }` — fails with `no matching package` outside the workspace. Needs `path = "..."` or a note about workspace requirement.

### BUG 4: Env vars not wired in config
`config.rs` documents "4. Environment variables (NEBULA_*)" but `ConfigBuilder` never adds `ConfigSource::Env`. The promise is broken.

---

## What's Better Than Competitors

| Feature | Advantage |
|---------|-----------|
| `nebula validate` as first-class command | No competitor has standalone offline validation |
| `nebula dev action new` scaffolding | Temporal/Prefect have zero scaffolding |
| Human-friendly duration parsing (`30s`, `5m`) | Better than raw seconds |
| JSON stdout / logs stderr separation | Unix-correct, pipeable |
| Config layering (global → project → env → flags) | Well-designed precedence |
| `--stream` flag concept | Forward-thinking (needs implementation) |

## What All Competitors Have That We Lack

| Feature | Temporal | Prefect | Airflow | Conductor | Nebula |
|---------|----------|---------|---------|-----------|--------|
| Execution history query | `workflow list/describe` | `flow-run ls/inspect` | `dags list-runs` | `workflow status` | **Missing** |
| Local dev server | `server start-dev` | `flow serve` | `standalone` | `server start` | **Missing** |
| Credential management CLI | `env set` | `profile create` | `connections add/list` | `config` | **Missing** |
| Named env profiles | `env set prod` | `profile create prod` | N/A | `config` profiles | **Stub only** |
| Dry-run / test mode | `workflow execute` | `flow serve` | `dags test` | `--sync` | **Missing** |
| Health check command | `cluster health` | `server health` | `db check` | `/health` | **Missing** |
| Bulk/directory operations | N/A | `deploy` | `dags list` | `workflow search` | **Missing** |

---

## Proposed Priority Roadmap

### Sprint 1: CI-Ready (1 day)
- [ ] Fix exit codes (P0 #1, #2)
- [ ] TTY-aware format default (P0 #3)
- [ ] `--quiet` flag (P0 #4)
- [ ] Remove `version` subcommand (P2 #11)

### Sprint 2: Developer Experience (2 days)
- [ ] `--input-file` flag (P1 #6)
- [ ] `--dry-run` on run (P1 #5)
- [ ] `nebula action test` command (P1 #8)
- [ ] `--format` on all commands (P1 #7)
- [ ] Fix scaffold Cargo.toml (BUG #3)

### Sprint 3: Workflow Authoring (1-2 days)
- [ ] Reduce YAML boilerplate for local run (P1 #9)
- [ ] Directory validation (P1 #10)
- [ ] Fix `--stream` or remove (P2 #15)
- [ ] `dev init --with-config` (P2 #16)

### Sprint 4: Polish (1 day)
- [ ] Singular `action` naming (P2 #12)
- [ ] Flatten dev nesting (P2 #13)
- [ ] Color output (P2 #14)
- [ ] Wire env vars in config (BUG #4)

### Future (needs server binary)
- `nebula health` command
- Execution history query
- Named environment profiles
- Remote mode (`--remote <url>`)
- `nebula dev serve` (local dev server)

---

## Key Quotes

> "The single most important fix. CI pipelines cannot trust exit codes." — DevOps

> "The path from 'I created an action' to 'I ran a workflow that uses it' does not exist." — DX Tester

> "`nebula action test` is the highest-impact change for plugin developers." — SDK User

> "Keep `run` and `validate` top-level. Do not add a `workflow` prefix. Frequency wins." — UX Designer

> "The tight coupling to 9 crates is the biggest risk, but wait for the second consumer." — Tech Lead
