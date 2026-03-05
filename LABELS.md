# Labels & Issue Organization

This document defines the label system for triaging, organizing, and tracking issues in Nebula.

---

## Label Hierarchy

We use a hierarchical labeling system: `category:value`. This keeps labels organized and searchable.

---

## Categories

### 1. Type — What Kind of Work?

**Exactly one per issue.**

| Label | Color | Use Case |
|-------|-------|----------|
| `type:bug` | `#d73a49` (red) | Something is broken or not working as intended |
| `type:feature` | `#1f883d` (green) | New capability or functionality |
| `type:enhancement` | `#f9f0c9` (yellow) | Improve existing feature (not a new feature) |
| `type:docs` | `#d4c5f9` (purple) | Documentation only; no code changes |
| `type:chore` | `#cfd3d7` (gray) | Build, CI, dependency updates, refactoring |
| `type:question` | `#ffd700` (gold) | User question (convert to discussion if appropriate) |

**How to pick:**
- Bug = code doesn't match spec OR behaves unexpectedly
- Feature = entirely new capability
- Enhancement = make existing feature better (faster, more options, cleaner API)
- Docs = only documentation changes
- Chore = maintenance with no user-facing impact

---

### 2. Area — Which Part of the Project?

**Use multiple as needed.**

| Label | Color | Scope |
|-------|-------|-------|
| `area:action` | `#0075ca` (blue) | Action trait, action execution, contracts |
| `area:engine` | `#0075ca` | DAG scheduler, orchestration, execution planning |
| `area:runtime` | `#0075ca` | Task runner, isolation, work queue, execution |
| `area:storage` | `#0075ca` | KV storage abstraction, PostgreSQL adapter, persistence |
| `area:credential` | `#0075ca` | Credential management, encryption, rotation, injection |
| `area:resource` | `#0075ca` | Resource lifecycle, pooling, connection management |
| `area:api` | `#0075ca` | REST API, WebSocket, HTTP server, endpoints |
| `area:workflow` | `#0075ca` | Workflow definition, DAG model, schema, validation |
| `area:execution` | `#0075ca` | Execution state machine, lifecycle, status tracking |
| `area:plugin` | `#0075ca` | Plugin system, SDKs, third-party integrations |
| `area:telemetry` | `#0075ca` | Logging, metrics, tracing, observability |
| `area:testing` | `#0075ca` | Test infrastructure, benchmarks, test utilities |
| `area:ui` | `#0075ca` | Desktop app (Tauri), web UI, visual editor |
| `area:docs` | `#0075ca` | Documentation, READMEs, guides, architecture docs |
| `area:infra` | `#0075ca` | CI/CD, deployment, Docker, Kubernetes, build system |
| `area:performance` | `#0075ca` | Speed, memory, throughput, optimization |

**How to pick:**
- Use all areas that are affected by the issue
- If unsure, pick the most directly affected area
- Multiple areas is OK (e.g., `area:engine` + `area:runtime` for DAG execution)

---

### 3. Priority — How Urgent?

**Pick one; optional for low-priority items (defaults to `priority:p2`).**

| Label | Color | Meaning |
|-------|-------|---------|
| `priority:p0` | `#ff0000` (red) | **Critical** — System broken, data loss risk, security vuln. Fix immediately. |
| `priority:p1` | `#ff6600` (orange) | **Important** — Major feature broken. Schedule for next sprint. |
| `priority:p2` | `#ffdd00` (yellow) | **Normal** — Nice-to-have, add to backlog. Most issues are P2. |
| `priority:p3` | `#cccccc` (gray) | **Low** — Future consideration, nice-to-have, polish. |

**Usage notes:**
- P0/P1 should be rare (5–10% of issues)
- Most bugs are P1 or P2
- Enhancement ideas are usually P2 or P3
- Security issues are P0

---

### 4. Difficulty — How Hard Is It?

**Pick one; helps match issues to contributors.**

| Label | Color | Effort |
|-------|-------|--------|
| `difficulty:good-first-issue` | `#7057ff` (blue) | Perfect for newcomers; 2–4 hours; minimal context needed |
| `difficulty:medium` | `#7057ff` | Moderate challenge; 2–5 days; some architecture knowledge needed |
| `difficulty:hard` | `#7057ff` | Complex; 1–2+ weeks; expert knowledge required |

**How to pick:**
- Good first issue = isolated, self-contained, clear acceptance criteria
- Medium = requires understanding 2–3 crates, some refactoring, good tests
- Hard = touches core architecture, multiple crates, design questions, OR takes > 1 week

---

### 5. Status — Where Are We?

**Applied by maintainers; changes as work progresses.**

| Label | Color | Meaning |
|-------|-------|---------|
| `status:blocked` | `#fc2929` (red) | Waiting on external dependency or another issue |
| `status:needs-discussion` | `#ffc400` (yellow) | Design question; needs feedback before work starts |
| `status:in-progress` | `#0075ca` (blue) | Someone is actively working on it |
| `status:ready` | `#1f883d` (green) | Approved and ready for work; implementation details clear |
| `status:on-hold` | `#cfd3d7` (gray) | Intentionally paused; will revisit later |

---

### 6. Stage — Development Phase

**Applied by maintainers; reflects current phase.**

| Label | Color | Meaning |
|-------|-------|---------|
| `stage:phase1` | `#cce5ff` (light blue) | Phase 1: Core Foundation |
| `stage:phase2` | `#cce5ff` | Phase 2: Execution Engine |
| `stage:phase3` | `#cce5ff` | Phase 3: Credential System |
| `stage:phase4` | `#cce5ff` | Phase 4: Plugin Ecosystem |
| `stage:phase5` | `#cce5ff` | Phase 5: Desktop App & Polish |

See [vision/ROADMAP.md](../vision/ROADMAP.md) for phase definitions.

---

## Example Label Combinations

### Example 1: Critical Bug

```
type:bug
area:credential
priority:p0
status:in-progress
difficulty:hard
```

→ A security bug in credential injection. Critical, someone is working on it.

### Example 2: Enhancement for New Contributors

```
type:enhancement
area:testing
difficulty:good-first-issue
priority:p2
status:ready
```

→ Improve test infrastructure. Perfect for a newcomer who wants to contribute.

### Example 3: Design Question

```
type:feature
area:engine
priority:p1
status:needs-discussion
difficulty:hard
```

→ New DAG scheduling feature. Important but needs design consensus first.

---

## Creating Issues

### Labeling Checklist

Before closing an issue creation form:

- [ ] **Type**: Pick exactly one (`type:*`)
- [ ] **Area**: Pick one or more (`area:*`)
- [ ] **Priority**: Pick one if P0–P1; optional for others
- [ ] **Difficulty**: Pick one if you know
- [ ] **Status**: Leave blank; maintainers will set

---

## Searching by Label

### Quick Searches

Find issues to work on:

```
Good first issues:
https://github.com/vanyastaff/nebula/issues?q=label:difficulty:good-first-issue

Help wanted:
https://github.com/vanyastaff/nebula/issues?q=label:status:ready

Critical bugs:
https://github.com/vanyastaff/nebula/issues?q=label:priority:p0

Engine work (all statuses):
https://github.com/vanyastaff/nebula/issues?q=label:area:engine

Blocked issues:
https://github.com/vanyastaff/nebula/issues?q=label:status:blocked
```

### Advanced Filters

```
# Bugs in runtime, ready for work, good for beginners
type:bug area:runtime status:ready difficulty:good-first-issue

# High-priority enhancements in Phase 2
type:enhancement priority:p0,p1 stage:phase2

# Docs issues that need discussion
type:docs status:needs-discussion
```

---

## Removing Labels

### When to Remove

- **`status:in-progress`** — When someone stops working on it (no activity for 2 weeks)
- **`status:blocked`** — When the blocking issue is resolved
- **`status:ready`** — When work starts (add `status:in-progress`)

### How to Remove

```
/remove-label status:in-progress
```

(If your repo has a bot; otherwise, maintainers will do it)

---

## FAQ

**Q: Can an issue have multiple type labels?**
A: No. Pick the primary type. If it's both a bug and an enhancement, it's probably two separate issues.

**Q: Should every issue have a difficulty label?**
A: No. Only if you can estimate the work. Maintainers can add it later.

**Q: What if I disagree with a label?**
A: Comment on the issue explaining why. Maintainers are happy to reconsider.

**Q: Are labels mandatory?**
A: Type and at least one area, yes. Others are helpful but optional for contributors.

---

## Maintenance

This label scheme is reviewed quarterly. If you notice:
- Labels that are never used → remove them
- Labels that should be split → propose a change
- Missing categories → open an issue

---

**See Also:**
- [ISSUES.md](ISSUES.md) — How to report bugs and request features
- [WORKFLOW.md](WORKFLOW.md) — Branch and commit conventions
- [vision/ROADMAP.md](../vision/ROADMAP.md) — Phase definitions

