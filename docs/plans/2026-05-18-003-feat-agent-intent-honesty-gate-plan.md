# Agent Structural-Budget Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the deterministic structural-budget tier of ADR-0083 — a bash Stop/SubagentStop guard that blocks oversized / file-sprawling / duplicate-symbol / blob turns the green D10 gate cannot see, with a `// budget-justified:` escape and an audit log.

**Architecture:** New `command` hook `.claude/hooks/intent-gate.sh`, ordered after `stop-gate.sh` in the `Stop` array and added to a new `SubagentStop` entry. It reuses `_lib.sh` plumbing (turn-state, `deny`/`allow`, `git_common_dir`). It is fully deterministic (git + bash, no model), so it is unit-tested by the existing `.claude/hooks/test/run.sh` harness. The confidence-gated semantic LLM tier of ADR-0083 is a **separate sequenced follow-up plan** (its mechanism — native `prompt`/`agent` hook vs headless — needs a spike and is not deterministically TDD-able); it is not dropped, only sequenced.

**Tech Stack:** Bash, `jq`, `git`, Claude Code hooks (`Stop`/`SubagentStop` `command` type), `task hooks:test`, `convco`/`lefthook`.

---

## Scope

**In scope (this plan, shippable on its own):** deterministic structural-budget tier; inoculation + abstention lines in the current implementation/producer subagents; `settings.json` wiring; `task hooks:test` proofs; CLAUDE.md / QUALITY_GATES.md / ADR-README doc updates; slim ADR-0083 to the lean decision.

**Out of scope (separate follow-up plan, `docs/plans/`, no new ADR):** the semantic LLM tier (prompt/agent judge, `intent-reviewer.md`, grounded rubric). Tracked in ADR-0083 §Follow-up and `project_adr0083_intent_gate` memory. Precise per-fn AST complexity (this plan ships the deterministic blob proxy described in Task 4).

## File Structure

- Create: `.claude/hooks/intent-gate.sh` — the deterministic gate (one responsibility: structural-budget verdict on the turn diff).
- Modify: `.claude/settings.json` — wire the hook into `Stop` (after `stop-gate.sh`) and a new `SubagentStop` entry.
- Modify: `.claude/hooks/test/run.sh` — append proof cases after the `# HOOKMARK` line.
- Modify: `.claude/agents/implement-worker.md`, `.claude/agents/loop-producer.md` — add the inoculation + abstention-as-success paragraph.
- Modify: `CLAUDE.md` — Enforced Discipline table row + D10 prose note.
- Modify: `docs/QUALITY_GATES.md` — note diff-scoped enforcement reconciliation.
- Modify: `docs/adr/README.md` — thematic index "Agent harness" row → 0083.
- Modify: `docs/adr/0083-agent-intent-honesty-gate.md` — slim to lean decision (final task).

Verification entrypoint: `task hooks:test` (`bash .claude/hooks/test/run.sh`). Per-crate Rust build is not involved — these are bash/JSON artifacts; lefthook skips fmt/clippy/deny for them.

---

### Task 1: Create intent-gate.sh skeleton (pre-filter + loop bound + log), default-allow

**Files:**
- Create: `.claude/hooks/intent-gate.sh`
- Test: `.claude/hooks/test/run.sh` (append after `# HOOKMARK`)

- [ ] **Step 1: Write the skeleton hook**

Create `.claude/hooks/intent-gate.sh`:

```bash
#!/usr/bin/env bash
# Layer-2 deterministic structural-budget gate (ADR-0083). Runs AFTER
# stop-gate.sh (C). Pure git+bash, no model. Blocking convention from _lib.sh:
# deny() => stderr + exit 2 (turn continues); allow() => exit 0.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.stop_hook_active')" = "true" ] && allow   # loop guard
have_jq || allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
TS_PATH="$(turn_state_path "$sid" "$cwd")"
st="$(load_state "$TS_PATH")"

# Audit log: every verdict. value-free reasons only.
ig_log() { # $1=verdict $2=reason
  local d f; d="$(git_common_dir "$cwd")/.nebula-guard"
  mkdir -p "$d" 2>/dev/null || return 0
  f="$d/intent-log-${sid:-unknown}.jsonl"
  printf '{"v":"%s","r":"%s"}\n' "$1" "$2" >>"$f" 2>/dev/null || true
}

# Loop counter lives in the RAW turn-state file (load_state projects it away;
# turn-reset.sh rewrites the file fresh at A0 so it is naturally per-turn).
ig_attempts() { jq -r '.intent_attempts // 0' "$TS_PATH" 2>/dev/null || echo 0; }
ig_bump() {
  have_jq || return 0; [ -f "$TS_PATH" ] || return 0
  local t; t="$(jq -c '.intent_attempts = ((.intent_attempts // 0) + 1)' "$TS_PATH" 2>/dev/null)" \
    && printf '%s' "$t" >"$TS_PATH" 2>/dev/null || true
}

# Pre-filter: C (stop-gate) owns broken code. If the turn touched lib crates
# but recorded no green gate, C will block — do not double-judge.
impl_n="$(printf '%s' "$st" | jq -r '.impl_files_edited | length' 2>/dev/null || echo 0)"
green_n="$(printf '%s' "$st" | jq -r '.gate_green | length' 2>/dev/null || echo 0)"
if [ "${impl_n:-0}" -gt 0 ] && [ "${green_n:-0}" -eq 0 ]; then
  ig_log allow "c-owns-broken"; allow
fi

# Loop bound: after N=2 denies this turn, allow + log escalation (never trap
# the human; the log is the review surface).
attempts="$(ig_attempts)"
if [ "${attempts:-0}" -ge 2 ]; then
  ig_log escalate "loop-bound-after-2"; allow
fi

ig_log allow "skeleton-default"
allow
```

- [ ] **Step 2: Append the failing tests after `# HOOKMARK`**

In `.claude/hooks/test/run.sh`, immediately AFTER the line `# Per-hook cases are appended by later tasks below this line. # HOOKMARK`, add:

```bash
# E intent-gate (ADR-0083 deterministic structural-budget tier)
egate() { printf '%s' "$1" | bash "$HERE/intent-gate.sh" >/dev/null 2>&1; echo $?; }
E_SID="e-skel"; E_P="$(turn_state_path "$E_SID" "$PWD")"; mkdir -p "$(dirname "$E_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$E_P"
chk "E loop-guard allows"   0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":true}')"
chk "E default allows"      0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":["crates/engine/src/x.rs"],"gate_green":[],"turn_base":""}' >"$E_P"
chk "E defers to C broken"  0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":2}' >"$E_P"
chk "E loop-bound allows"   0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
```

- [ ] **Step 3: Run the harness to verify the new cases pass**

Run: `task hooks:test`
Expected: output includes `ok   - E loop-guard allows`, `ok   - E default allows`, `ok   - E defers to C broken`, `ok   - E loop-bound allows`, and final line `ALL GUARD TESTS PASSED`.

- [ ] **Step 4: Commit**

```bash
git add .claude/hooks/intent-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): add intent-gate.sh skeleton (ADR-0083 deterministic tier)"
```

---

### Task 2: Net-LoC budget block with `// budget-justified:` escape

**Files:**
- Modify: `.claude/hooks/intent-gate.sh`
- Test: `.claude/hooks/test/run.sh`

- [ ] **Step 1: Append the failing tests after the Task-1 block**

In `.claude/hooks/test/run.sh`, after the Task-1 `E` cases, add:

```bash
# E net-LoC budget (starting cap 400; // budget-justified: escapes)
EB_DIR="$(mktemp -d)"
( cd "$EB_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/eb/src && { for i in $(seq 1 450); do echo "// line $i"; done; } > crates/eb/src/big.rs )
EB_SID="e-bud"; EB_P="$(turn_state_path "$EB_SID" "$EB_DIR")"; mkdir -p "$(dirname "$EB_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EB_P"
chk "E blocks >400 net-LoC" 2 "$(egate '{"session_id":"'"$EB_SID"'","cwd":"'"$EB_DIR"'","stop_hook_active":false}')"
( cd "$EB_DIR" && printf '// budget-justified: generated table\n' >> crates/eb/src/big.rs )
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EB_P"
chk "E budget-justified escapes" 0 "$(egate '{"session_id":"'"$EB_SID"'","cwd":"'"$EB_DIR"'","stop_hook_active":false}')"
rm -rf "$EB_DIR"
# Regression: an untracked code file whose lines start with `+` must still be
# counted (the `+ ` space sentinel in ig_added_lines). Without it sed makes
# `++…` which the `^\+([^+]|$)` count rejects → silent undercount → escapes.
EBP_DIR="$(mktemp -d)"
( cd "$EBP_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && { for i in $(seq 1 450); do echo "+marker $i"; done; } > plus.sh )
EBP_SID="e-bud-plus"; EBP_P="$(turn_state_path "$EBP_SID" "$EBP_DIR")"; mkdir -p "$(dirname "$EBP_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$EBP_P"
chk "E counts +-prefixed untracked" 2 "$(egate '{"session_id":"'"$EBP_SID"'","cwd":"'"$EBP_DIR"'","stop_hook_active":false}')"
rm -rf "$EBP_DIR"
```

- [ ] **Step 2: Run to verify the new cases FAIL**

Run: `task hooks:test`
Expected: `FAIL - E blocks >400 net-LoC (expected[2] got[0])` (skeleton always allows).

- [ ] **Step 3: Implement the net-LoC check**

In `.claude/hooks/intent-gate.sh`, replace the final two lines (`ig_log allow "skeleton-default"` and `allow`) with:

```bash
# Turn diff scope: committed-this-turn (turn_base..HEAD) + working tree +
# staged + UNTRACKED. Agents typically leave new files unstaged, so a diff-
# only view misses them; stop-gate.sh (C) uses the same `git status -u`
# ground truth. Code files only.
tb="$(printf '%s' "$st" | jq -r '.turn_base // empty' 2>/dev/null)"
CODE_RE='\.(rs|toml|sh|md)$'

# Unified added-content stream: a `+++ <path>` header per file then each added
# line prefixed `+`. Tracked deltas from `git diff --unified=0`; every
# untracked code file is wholly added. blob / dup / budget all consume this.
ig_added_lines() {
  { [ -n "$tb" ] && git -C "$cwd" diff --unified=0 "$tb"..HEAD 2>/dev/null; \
    git -C "$cwd" diff --unified=0 2>/dev/null; \
    git -C "$cwd" diff --unified=0 --cached 2>/dev/null; } \
  | grep -E '^(\+\+\+ |\+)'
  while IFS= read -r uf; do
    [ -n "$uf" ] || continue
    printf '+++ %s\n' "$uf"
    # `+ ` (space sentinel) not `+`: a source line that itself starts with `+`
    # would become `++…` and be miscounted as a header by `^\+([^+]|$)`.
    sed 's/^/+ /' "$cwd/$uf" 2>/dev/null
  done < <(git -C "$cwd" ls-files --others --exclude-standard 2>/dev/null \
            | grep -E "$CODE_RE" || true)
}

# net = added − deleted. added = stream added lines minus `+++ ` headers;
# deleted = numstat deletions on tracked changes (untracked delete nothing).
added="$(ig_added_lines | grep -cE '^\+([^+]|$)')"
deleted=0
while read -r _a d _; do
  [[ "$d" =~ ^[0-9]+$ ]] && deleted=$((deleted + d))
done < <( { [ -n "$tb" ] && git -C "$cwd" diff --numstat "$tb"..HEAD 2>/dev/null; \
            git -C "$cwd" diff --numstat 2>/dev/null; \
            git -C "$cwd" diff --numstat --cached 2>/dev/null; } \
          | grep -E "$CODE_RE" || true )
net=$((added - deleted))

# Net-negative (cleanup / deletion) is always allowed — positive constraint.
if [ "$net" -lt 0 ]; then ig_log allow "net-negative"; allow; fi

# Escape token: `// budget-justified:` on any added line this turn.
budget_justified() { ig_added_lines | grep -qE '//[[:space:]]*budget-justified:'; }

NET_CAP=400
if [ "$net" -gt "$NET_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "net-loc-over-cap"
  deny "Turn net +$net LoC exceeds the structural budget ($NET_CAP). Split the change into reviewable commits, delete dead code, or add a \`// budget-justified: <reason>\` line to an intentional large addition (e.g. generated/table data). (ADR-0083 structural-budget tier; large diffs are a top review-rejection cause.)"
fi

ig_log allow "within-budget"
allow
```

- [ ] **Step 4: Run to verify the new cases PASS**

Run: `task hooks:test`
Expected: `ok   - E blocks >400 net-LoC`, `ok   - E budget-justified escapes`, all Task-1 `E` cases still `ok`, final `ALL GUARD TESTS PASSED`.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/intent-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): intent-gate net-LoC budget with budget-justified escape"
```

---

### Task 3: New-file budget block

**Files:**
- Modify: `.claude/hooks/intent-gate.sh`
- Test: `.claude/hooks/test/run.sh`

- [ ] **Step 1: Append the failing tests**

After the Task-2 `E` cases in `.claude/hooks/test/run.sh`:

```bash
# E new-file budget (cap 5)
EF_DIR="$(mktemp -d)"
( cd "$EF_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/ef/src && for i in 1 2 3 4 5 6; do echo "fn f${i}(){}" > "crates/ef/src/m${i}.rs"; done )
EF_SID="e-nf"; EF_P="$(turn_state_path "$EF_SID" "$EF_DIR")"; mkdir -p "$(dirname "$EF_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EF_P"
chk "E blocks >5 new files" 2 "$(egate '{"session_id":"'"$EF_SID"'","cwd":"'"$EF_DIR"'","stop_hook_active":false}')"
rm -rf "$EF_DIR"
```

- [ ] **Step 2: Run to verify FAIL**

Run: `task hooks:test`
Expected: `FAIL - E blocks >5 new files (expected[2] got[0])`.

- [ ] **Step 3: Implement the new-file check**

In `.claude/hooks/intent-gate.sh`, immediately BEFORE the `NET_CAP=400` line, add:

```bash
# New-file budget (ToF is the 2nd strongest decay predictor). ls-files
# --others lists individual files even inside a brand-new directory (which
# `git status --porcelain` would collapse to the dir).
new_files() {
  { [ -n "$tb" ] && git -C "$cwd" diff --name-only --diff-filter=A "$tb"..HEAD 2>/dev/null; \
    git -C "$cwd" diff --name-only --diff-filter=A --cached 2>/dev/null; \
    git -C "$cwd" ls-files --others --exclude-standard 2>/dev/null; } \
  | grep -E "$CODE_RE" | sort -u | grep -c . || true
}
NF_CAP=5
nf="$(new_files)"
if [ "${nf:-0}" -gt "$NF_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "new-file-over-cap"
  deny "Turn adds $nf new code files (cap $NF_CAP). Consolidate into existing modules, or add a \`// budget-justified: <reason>\` line. (ADR-0083; file-count predicts architectural decay.)"
fi
```

- [ ] **Step 4: Run to verify PASS**

Run: `task hooks:test`
Expected: `ok   - E blocks >5 new files`; all prior `E` cases still `ok`; `ALL GUARD TESTS PASSED`.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/intent-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): intent-gate new-file budget"
```

---

### Task 4: Large-blob proxy block (deterministic per-addition length)

This is the deterministic proxy for the ADR's per-changed-fn complexity check (the precise per-fn AST check is deferred to the semantic-tier follow-up plan). It flags a single added contiguous run longer than the `clippy.toml` `too-many-lines-threshold = 100` in one file.

**Files:**
- Modify: `.claude/hooks/intent-gate.sh`
- Test: `.claude/hooks/test/run.sh`

- [ ] **Step 1: Append the failing tests**

After the Task-3 `E` cases:

```bash
# E large-blob proxy (single added run > 100 lines in one code file)
EL_DIR="$(mktemp -d)"
( cd "$EL_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/el/src && { echo 'fn big(){'; for i in $(seq 1 130); do echo "  let v$i=$i;"; done; echo '}'; } > crates/el/src/f.rs )
EL_SID="e-blob"; EL_P="$(turn_state_path "$EL_SID" "$EL_DIR")"; mkdir -p "$(dirname "$EL_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EL_P"
chk "E blocks >100-line blob" 2 "$(egate '{"session_id":"'"$EL_SID"'","cwd":"'"$EL_DIR"'","stop_hook_active":false}')"
rm -rf "$EL_DIR"
```

- [ ] **Step 2: Run to verify FAIL**

Run: `task hooks:test`
Expected: `FAIL - E blocks >100-line blob (expected[2] got[0])`.

- [ ] **Step 3: Implement the blob proxy**

In `.claude/hooks/intent-gate.sh`, immediately BEFORE the `# New-file budget` comment added in Task 3, add:

```bash
# Large-blob proxy for per-fn complexity (clippy.toml too-many-lines = 100).
# Longest run of consecutive added lines within one file, consuming the
# shared ig_added_lines stream (untracked-aware, per-file via the `+++ `
# header reset — uniform with the net-LoC / dup-symbol consumers).
BLOB_CAP=100
longest_added_run() {
  ig_added_lines | awk '
      /^\+\+\+ /      { run=0; next }
      /^\+/           { run++; if (run>max) max=run; next }
      { run=0 }
      END             { print max+0 }'
}
blob="$(longest_added_run)"
if [ "${blob:-0}" -gt "$BLOB_CAP" ] && ! budget_justified; then
  ig_bump
  ig_log block "blob-over-cap"
  deny "Turn adds a $blob-line contiguous block in a single file (cap $BLOB_CAP, the clippy.toml too-many-lines threshold). Decompose into smaller functions, or add a \`// budget-justified: <reason>\` line for intentional generated/table code. (ADR-0083 structural-budget tier.)"
fi
```

- [ ] **Step 4: Run to verify PASS**

Run: `task hooks:test`
Expected: `ok   - E blocks >100-line blob`; all prior `E` cases still `ok`; `ALL GUARD TESTS PASSED`.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/intent-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): intent-gate large-blob complexity proxy"
```

---

### Task 5: Duplicate public-symbol heuristic block

**Files:**
- Modify: `.claude/hooks/intent-gate.sh`
- Test: `.claude/hooks/test/run.sh`

- [ ] **Step 1: Append the failing tests**

After the Task-4 `E` cases:

```bash
# E duplicate public symbol (new `pub fn NAME` colliding with existing one)
ED_DIR="$(mktemp -d)"
( cd "$ED_DIR" && git init -q && mkdir -p crates/a/src crates/b/src \
  && echo 'pub fn parse_token() {}' > crates/a/src/lib.rs \
  && git add -A && git -c user.email=t@t -c user.name=t commit -qm base \
  && echo 'pub fn parse_token() {}' > crates/b/src/lib.rs && git add -A )
ED_SID="e-dup"; ED_P="$(turn_state_path "$ED_SID" "$ED_DIR")"; mkdir -p "$(dirname "$ED_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$ED_P"
chk "E blocks dup pub symbol" 2 "$(egate '{"session_id":"'"$ED_SID"'","cwd":"'"$ED_DIR"'","stop_hook_active":false}')"
rm -rf "$ED_DIR"
# E skiplisted idiomatic name (pub fn new) must NOT block even when duplicated
EDN_DIR="$(mktemp -d)"
( cd "$EDN_DIR" && git init -q && mkdir -p crates/x/src crates/y/src \
  && echo 'pub fn new() {}' > crates/x/src/lib.rs \
  && git add -A && git -c user.email=t@t -c user.name=t commit -qm base \
  && echo 'pub fn new() {}' > crates/y/src/lib.rs && git add -A )
EDN_SID="e-dup-new"; EDN_P="$(turn_state_path "$EDN_SID" "$EDN_DIR")"; mkdir -p "$(dirname "$EDN_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$EDN_P"
chk "E skiplist allows dup new" 0 "$(egate '{"session_id":"'"$EDN_SID"'","cwd":"'"$EDN_DIR"'","stop_hook_active":false}')"
rm -rf "$EDN_DIR"
```

- [ ] **Step 2: Run to verify FAIL**

Run: `task hooks:test`
Expected: `FAIL - E blocks dup pub symbol (expected[2] got[0])`.

- [ ] **Step 3: Implement the duplicate-symbol heuristic**

In `.claude/hooks/intent-gate.sh`, immediately BEFORE the `# Large-blob proxy` comment from Task 4, add:

```bash
# Duplicate public-symbol heuristic: a NEW `pub fn|struct|trait NAME` whose
# NAME already exists (same kind) elsewhere in crates/*/src — the "47 date
# formatters" pattern. Added lines via ig_added_lines (untracked included).
# Idiomatic, legitimately-repeated names (constructors + trait/accessor
# boilerplate) are skipped: they saturate any Rust workspace (`pub fn new`
# is in hundreds of files) and are never the duplicate-utility smell, so
# flagging them would invert the gate's signal. `pub async fn` is out of
# scope by design (the smell is plain `pub fn`); widening it would enlarge
# the false-positive surface this skiplist exists to contain.
dup_symbol() {
  local added kind name hit
  added="$(ig_added_lines | grep -E '^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' || true)"
  [ -n "$added" ] || return 1
  while IFS= read -r line; do
    kind="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait).*/\1/')"
    [ -n "$kind" ] || continue
    name="$(printf '%s' "$line" | sed -E 's/^\+[[:space:]]*pub[[:space:]]+(fn|struct|trait)[[:space:]]+([A-Za-z_][A-Za-z0-9_]*).*/\2/')"
    [ -n "$name" ] || continue
    case "$name" in
      new|default|len|is_empty|build|builder|from|try_from|into|iter|iter_mut|next|poll|fmt|clone|eq|hash|drop|deref|get|set|id|name|kind|value) continue ;;
    esac
    hit="$(grep -rEl --include='*.rs' "(^|[^A-Za-z_])pub[[:space:]]+$kind[[:space:]]+$name([^A-Za-z0-9_]|$)" "$cwd"/crates/*/src 2>/dev/null | wc -l | tr -d ' ')"
    [ "${hit:-0}" -ge 2 ] && { printf '%s %s' "$kind" "$name"; return 0; }
  done <<< "$added"
  return 1
}
if d="$(dup_symbol)" && ! budget_justified; then
  ig_bump
  ig_log block "duplicate-symbol"
  deny "New public \`$d\` collides with an existing workspace symbol of the same kind. Reuse the existing one (search crates/*/src), or add a \`// budget-justified: <reason>\` line if the duplication is intentional. (ADR-0083; agents that do not search the codebase re-implement existing utilities.)"
fi
```

- [ ] **Step 4: Run to verify PASS**

Run: `task hooks:test`
Expected: `ok   - E blocks dup pub symbol`; all prior `E` cases still `ok`; `ALL GUARD TESTS PASSED`.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/intent-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): intent-gate duplicate public-symbol heuristic"
```

---

### Task 6: Abstention-as-success + clean-turn allow proofs

No new code — proves the existing pre-filter already treats a justified empty-diff turn and a small clean turn as success (FixedBench abstention).

**Files:**
- Test: `.claude/hooks/test/run.sh`

- [ ] **Step 1: Append the proofs**

After the Task-5 `E` cases:

```bash
# E abstention-as-success: empty diff + nothing edited => allow (not a block)
EA_DIR="$(mktemp -d)"; ( cd "$EA_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty )
EA_SID="e-abs"; EA_P="$(turn_state_path "$EA_SID" "$EA_DIR")"; mkdir -p "$(dirname "$EA_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EA_P"
chk "E abstention allowed" 0 "$(egate '{"session_id":"'"$EA_SID"'","cwd":"'"$EA_DIR"'","stop_hook_active":false}')"
# E small clean turn (<400 net, <=5 files, no blob, no dup) => allow
( cd "$EA_DIR" && mkdir -p crates/ok/src && printf 'pub fn small(){}\n' > crates/ok/src/lib.rs )
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EA_P"
chk "E small clean allowed" 0 "$(egate '{"session_id":"'"$EA_SID"'","cwd":"'"$EA_DIR"'","stop_hook_active":false}')"
rm -rf "$EA_DIR"
# E boundary: exactly NF_CAP (5) new files is ALLOWED (guard is -gt, not -ge)
EC_DIR="$(mktemp -d)"; ( cd "$EC_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/ec/src && for i in 1 2 3 4 5; do echo "fn f${i}(){}" > "crates/ec/src/m${i}.rs"; done )
EC_SID="e-cap"; EC_P="$(turn_state_path "$EC_SID" "$EC_DIR")"; mkdir -p "$(dirname "$EC_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$EC_P"
chk "E exactly-5 files allowed" 0 "$(egate '{"session_id":"'"$EC_SID"'","cwd":"'"$EC_DIR"'","stop_hook_active":false}')"
rm -rf "$EC_DIR"
```

- [ ] **Step 2: Run to verify PASS**

Run: `task hooks:test`
Expected: `ok   - E abstention allowed`, `ok   - E small clean allowed`, `ok   - E exactly-5 files allowed`, `ALL GUARD TESTS PASSED`.

- [ ] **Step 3: Commit**

```bash
git add .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "test(ci): intent-gate abstention + clean-turn proofs"
```

---

### Task 7: Wire intent-gate into settings.json (Stop + SubagentStop)

**Files:**
- Modify: `.claude/settings.json`

- [ ] **Step 1: Add the hook wiring**

In `.claude/settings.json`, replace the `"Stop"` block:

```json
    "Stop": [
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/stop-gate.sh\"" } ] }
    ]
```

with:

```json
    "Stop": [
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/stop-gate.sh\"" } ] },
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/intent-gate.sh\"" } ] }
    ],
    "SubagentStop": [
      { "matcher": "implement-worker", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/intent-gate.sh\"" } ] }
    ]
```

- [ ] **Step 2: Validate JSON**

Run: `jq -e . .claude/settings.json >/dev/null && echo OK`
Expected: `OK`

- [ ] **Step 3: Verify the harness still green**

Run: `task hooks:test`
Expected: final line `ALL GUARD TESTS PASSED`.

- [ ] **Step 4: Commit**

```bash
git add .claude/settings.json
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): wire intent-gate into Stop + implement-worker SubagentStop"
```

---

### Task 8: Inoculation + abstention lines in implementation/producer subagents

**Files:**
- Modify: `.claude/agents/implement-worker.md`
- Modify: `.claude/agents/loop-producer.md`

- [ ] **Step 1: Append the inoculation block to `implement-worker.md`**

At the end of `.claude/agents/implement-worker.md`, append:

```markdown

## Integrity (ADR-0083)

If a task is hard, implement it correctly — do not make the gate pass by
fitting code to the tests or adding unrequested scaffolding. Surfacing that
this is tempting is acceptable; doing it is not. Concluding that no change is
needed (issue already fixed, request already satisfied) is a fully successful
outcome — say so explicitly with the reason; it is not a failure.
```

- [ ] **Step 2: Append the same block to `loop-producer.md`**

At the end of `.claude/agents/loop-producer.md`, append the identical `## Integrity (ADR-0083)` section from Step 1.

- [ ] **Step 3: Verify markdown is intact**

Run: `tail -n 8 .claude/agents/implement-worker.md`
Expected: shows the `## Integrity (ADR-0083)` heading and paragraph.

- [ ] **Step 4: Commit**

```bash
git add .claude/agents/implement-worker.md .claude/agents/loop-producer.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "feat(ci): inoculation + abstention lines in worker/producer agents"
```

---

### Task 9: CLAUDE.md Enforced Discipline row + QUALITY_GATES note + ADR index

**Files:**
- Modify: `CLAUDE.md`
- Modify: `docs/QUALITY_GATES.md`
- Modify: `docs/adr/README.md`

- [ ] **Step 1: Add the Enforced Discipline table row**

In `CLAUDE.md`, in the "Enforced Discipline (guard hooks)" table, after the row
`| Cannot end a turn with impl changed but no green clippy+nextest | stop-gate.sh |`
add:

```markdown
| Layer-2: turn diff over structural budget (net-LoC / new-files / blob / dup symbol) | `intent-gate.sh` (deterministic, ADR-0083; `// budget-justified:` escape) |
```

- [ ] **Step 2: Note Layer-2 is an addition above D10**

In `CLAUDE.md`, in the paragraph that defines D10 ("The no-cheat guarantee is structural (D10): B ... + C ... + lefthook/CI."), append this sentence:

```markdown
`intent-gate.sh` (Layer-2, ADR-0083) is a deterministic structural-budget
**addition above** D10 — it does not alter the D10 core; `stop-gate.sh` still
runs first and remains the guarantee.
```

- [ ] **Step 3: Add the QUALITY_GATES.md reconciliation note**

At the end of `docs/QUALITY_GATES.md`, append:

```markdown

## Diff-scoped structural budget (ADR-0083)

The `cognitive_complexity` / `too_many_lines` workspace `allow` stays — flipping
them on 36 crates is thousands of legacy warnings. New code is instead held to
the `clippy.toml` thresholds **diff-scoped** by `.claude/hooks/intent-gate.sh`
(net-LoC, new-file, large-blob proxy, duplicate-symbol), with a
`// budget-justified:` escape. Legacy is grandfathered; the separate legacy
burn-down workstream reconciles it crate-by-crate.
```

- [ ] **Step 4: Add the ADR thematic-index row**

In `docs/adr/README.md`, in the "Thematic index (agents start here)" table, add a row:

```markdown
| **Agent harness** | **0083** | Intent / structural-budget / honesty gate |
```

- [ ] **Step 5: Verify**

Run: `grep -n "intent-gate.sh" CLAUDE.md && grep -n "ADR-0083" docs/QUALITY_GATES.md && grep -n "Agent harness" docs/adr/README.md`
Expected: a match line from each of the three files.

- [ ] **Step 6: Commit**

```bash
git add CLAUDE.md docs/QUALITY_GATES.md docs/adr/README.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "docs: record Layer-2 intent-gate in CLAUDE.md/QUALITY_GATES/ADR-index"
```

---

### Task 10: Slim ADR-0083 to the lean decision

Per ADR-0083 §"Documentation discipline": after the plan exists, the design detail moves into this plan; the ADR keeps Context → Decision → Consequences and points here. This task is deletion-only in the ADR body (net-negative — the gate allows it).

**Files:**
- Modify: `docs/adr/0083-agent-intent-honesty-gate.md`

- [ ] **Step 1: Remove the heavy design body**

In `docs/adr/0083-agent-intent-honesty-gate.md`, delete the entire `## Design` section (from the `## Design` heading up to but not including `## Consequences`). Replace it with:

```markdown
## Design

The deterministic structural-budget tier is specified, with code and tests, in
[`docs/plans/2026-05-18-003-feat-agent-intent-honesty-gate-plan.md`](../plans/2026-05-18-003-feat-agent-intent-honesty-gate-plan.md).
The confidence-gated semantic LLM tier (grounded rubric, out-of-context
reviewer) is a sequenced follow-up plan in `docs/plans/`. This ADR records the
**decision**; implementation detail is not duplicated here (0082 convention).
```

- [ ] **Step 2: Trim the Context to the decision rationale**

In the `## Context` section, keep the first paragraph (D10 summary), the bullet list of evidence (Volume-Quality / Laziness-Deficit / SlopCodeBench / reward-hacking / action-bias / duplicate-helper), the verified clippy-allow gap paragraph, and the "Five vectors" list. Delete any sentence that restates mechanism (mechanism now lives in the plan). The Context must still answer "why this decision" in under ~250 words.

- [ ] **Step 3: Verify the ADR is lean and still coherent**

Run: `wc -l docs/adr/0083-agent-intent-honesty-gate.md`
Expected: substantially fewer lines than before — the heavy `## Design` body is replaced by a pointer to this plan. Target is "lean where it matters" (~190–200 lines), not a hard line count: the Follow-up-workstream and Documentation-discipline sections are decision content this ADR must retain, so the 0082-style <90 is not applicable here. Decision content is never sacrificed for a line target. The file still has `## Context`, `## Decision`, `## Consequences`, `## Follow-up workstream`, `## Documentation discipline`, `## Supersession`.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0083-agent-intent-honesty-gate.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" \
  commit -m "docs(adr): slim 0083 to the lean decision (detail -> plan)"
```

---

### Task 11: Final verification

- [ ] **Step 1: Full guard harness green**

Run: `task hooks:test`
Expected: final line `ALL GUARD TESTS PASSED`, exit 0. All `E ` cases present and `ok`.

- [ ] **Step 2: settings.json valid + hooks wired**

Run: `jq -e '.hooks.Stop[1].hooks[0].command, .hooks.SubagentStop[0].matcher' .claude/settings.json`
Expected: prints the `intent-gate.sh` command string and `"implement-worker"`.

- [ ] **Step 3: No placeholder left in the gate**

Run: `grep -nE 'TODO|FIXME|TBD|placeholder' .claude/hooks/intent-gate.sh; echo done`
Expected: just `done` (no matches).

- [ ] **Step 4: Branch state summary**

Run: `git -C "$PWD" log --oneline -12`
Expected: the Task 1–10 commits present in order; working tree clean (`git status --porcelain` empty).

---

## Out of scope — sequenced follow-up (separate plan, no ADR)

The ADR-0083 **semantic LLM tier** (confidence-gated grounded-rubric judge, `prompt`/`agent` hook mechanism spike, `.claude/agents/intent-reviewer.md`, precise per-fn AST complexity replacing the Task-4 proxy) is a separate `docs/plans/` plan, sequenced after this one, per the one-ADR program discipline. It is **not dropped** — recorded here, in ADR-0083 §Follow-up, and in the `project_adr0083_intent_gate` memory. Also sequenced after 0083: legacy structural-debt burn-down; AI-Factory removal / `ce-*` default.
