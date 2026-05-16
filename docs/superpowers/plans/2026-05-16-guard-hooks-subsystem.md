# Guard Hooks Subsystem Implementation Plan (bash + jq — D9)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Harness-enforced, evasion-resistant guard hooks (bash + jq) so the agent cannot weaken tests, suppress lints, bypass lefthook, or claim "done" without a verified gate.

**Architecture (D10):** POSIX bash hooks under `scripts/guard/` sharing `_lib.sh`, wired in committed `.claude/settings.json` (`command`-type). `jq` parses stdin. The no-cheat guarantee is **structural**, not parser-based: **B** (edit-guard, hard-deny) + **A2** (`record.sh` — records a green gate only for a canonical *clean* invocation; lint-suppressed/masked/redirected/echoed runs never count) + **C** (Stop-gate, hard-block) + lefthook/CI. **Hook A is a fail-OPEN advisory tripwire** (blatant literals only), not a security boundary — 5 adversarial rounds proved a hand-rolled bash shell-parser on a boundary is an un-winnable arms race, so the boundary was relocated to the oracle.

**Tech Stack:** bash 5 (git-bash on Windows — already required by lefthook), jq 1.8, git, Taskfile. No Node, no build step.

**Plan series (Plan 1 of 4 — spec `docs/superpowers/specs/2026-05-16-agent-discipline-and-curation-design.md`, decision D9):** 1=this; 2=D8 doc-canon inversion; 3=skill+subagent curation (G/H); 4=lefthook granularity (F)+`nebula-pitfalls` (E).

**Supersedes the Node `.mjs` draft.** Task 1 removes the obsolete `.claude/hooks/*.mjs` (commits `53707567`, `f275b4da`) and replaces them with `scripts/guard/`.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `scripts/guard/_lib.sh` | Shared: stdin read, jq extract, turn-state path/load/save, `crate_of`/`is_lib_rust`, `deny`/`allow` (no shell parser — D10) |
| `scripts/guard/turn-reset.sh` | A0 `UserPromptSubmit`: reset turn-state |
| `scripts/guard/bash-deny.sh` | A `PreToolUse/Bash`: fail-OPEN advisory tripwire — blatant no-verify / fmt --all / force-push only (D10; not a boundary) |
| `scripts/guard/record.sh` | A2 `PostToolUse/Bash`: record green clippy/nextest per crate |
| `scripts/guard/edit-guard.sh` | B `PreToolUse/Edit\|Write\|MultiEdit`: cheat/costyl + test-weakening |
| `scripts/guard/stop-gate.sh` | C `Stop`: block done without recorded green gate |
| `scripts/guard/fmt.sh` | D `PostToolUse/Edit\|Write\|MultiEdit`: format touched file |
| `scripts/guard/test/run.sh` | bash assertion harness (`task hooks:test`) |
| `.claude/settings.json` | Committed wiring + `$schema` + curated permissions |
| `Taskfile.yml` | `task hooks:test` |
| `CLAUDE.md` | "Enforced Discipline" rule→guard map |

All hooks: `set -uo pipefail`; source `_lib.sh`; end with `allow` (exit 0); never exceed ~2 s.

---

### Task 1: Remove obsolete `.mjs` + create `scripts/guard/_lib.sh`

**Files:**
- Delete: `.claude/hooks/guard-lib.mjs`, `.claude/hooks/__tests__/guard-lib.test.mjs`
- Create: `scripts/guard/_lib.sh`, `scripts/guard/test/run.sh`

- [ ] **Step 1: Remove the superseded Node scaffolding**

```bash
git rm .claude/hooks/guard-lib.mjs .claude/hooks/__tests__/guard-lib.test.mjs
rmdir .claude/hooks/__tests__ .claude/hooks 2>/dev/null || true
```

- [ ] **Step 2: Write the failing test harness `scripts/guard/test/run.sh`**

```bash
#!/usr/bin/env bash
# scripts/guard/test/run.sh — guard-hook test harness. Exit 1 if any case fails.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$HERE/_lib.sh"
fail=0
chk() { # chk "name" expected actual
  if [ "$2" = "$3" ]; then printf 'ok   - %s\n' "$1"
  else printf 'FAIL - %s (expected[%s] got[%s])\n' "$1" "$2" "$3"; fail=1; fi
}

# --- _lib unit checks ---
# (D10: normalize_argv0/resolve_cmd deleted — hook A is now a fail-open
# substring tripwire that needs no shell parser; nothing else uses them.)
LS_T="$(mktemp)"; printf '{"impl_files_edited":"oops"}' >"$LS_T"
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[]}' "$(load_state "$LS_T")"; rm -f "$LS_T"
chk "crate_of extracts" engine "$(crate_of 'crates/engine/src/engine.rs')"
chk "crate_of windows path" engine "$(crate_of 'crates\\engine\\src\\engine.rs')"
chk "crate_of none" "" "$(crate_of 'README.md')"
is_lib_rust 'crates/engine/src/state.rs'        && chk "is_lib_rust src" 0 0 || chk "is_lib_rust src" 0 1
is_lib_rust 'crates/engine/tests/retry.rs'      && chk "is_lib_rust tests" 1 0 || chk "is_lib_rust tests" 1 1
is_lib_rust 'crates\\engine\\src\\state.rs'     && chk "is_lib_rust win" 0 0 || chk "is_lib_rust win" 0 1

# Per-hook cases are appended by later tasks below this line. # HOOKMARK

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
```

- [ ] **Step 3: Run it to verify it fails**

Run: `bash scripts/guard/test/run.sh`
Expected: FAIL — `_lib.sh` not found / function missing (non-zero exit).

- [ ] **Step 4: Implement `scripts/guard/_lib.sh`**

```bash
# scripts/guard/_lib.sh — shared helpers for Nebula guard hooks. Source, don't exec.
# Blocking convention: deny() => stderr + exit 2. allow() => exit 0.
guard_input=""
read_input() { guard_input="$(cat)"; }
have_jq() { command -v jq >/dev/null 2>&1; }
jqg() { printf '%s' "$guard_input" | jq -r "$1" 2>/dev/null || true; }

deny()  { printf 'guard: %s\n' "$1" >&2; exit 2; }
allow() { exit 0; }

git_common_dir() { # $1=cwd
  local g
  if g="$(git -C "${1:-$PWD}" rev-parse --git-common-dir 2>/dev/null)"; then
    case "$g" in /*|[A-Za-z]:[\\/]*) printf '%s' "$g";; *) printf '%s/%s' "${1:-$PWD}" "$g";; esac
  else
    printf '%s' "${TMPDIR:-/tmp}/nebula-guard"
  fi
}
turn_state_path() { printf '%s/.nebula-guard/turn-%s.json' "$(git_common_dir "${2:-$PWD}")" "${1:-unknown}"; }
load_state() { # $1=path -> always {impl_files_edited:[...],gate_green:[...]}
  local d='{"impl_files_edited":[],"gate_green":[]}'
  if [ -f "$1" ] && have_jq && jq -e . "$1" >/dev/null 2>&1; then
    jq -c '{impl_files_edited:(if (.impl_files_edited|type)=="array" then .impl_files_edited else [] end),gate_green:(if (.gate_green|type)=="array" then .gate_green else [] end)}' "$1" 2>/dev/null || printf '%s' "$d"
  else printf '%s' "$d"; fi
}
save_state() { mkdir -p "$(dirname "$1")" 2>/dev/null && printf '%s' "$2" >"$1" 2>/dev/null || true; }

crate_of() { # $1=path -> crate name or empty
  local p="${1//\\\\//}"; p="${p//\\//}"
  [[ "$p" =~ (^|/)crates/([^/]+)/ ]] && printf '%s' "${BASH_REMATCH[2]}"
}
is_lib_rust() { # $1=path -> return 0 if library rust
  local p="${1//\\\\//}"; p="${p//\\//}"
  [[ "$p" == *.rs ]] || return 1
  [[ "$p" =~ (^|/)crates/[^/]+/src/ ]] || return 1
  [[ "$p" =~ /(tests|benches|examples)/ ]] && return 1
  [[ "$p" =~ /(main|build)\.rs$ ]] && return 1
  return 0
}
# (D10) resolve_cmd / normalize_argv0 intentionally REMOVED. Five adversarial
# rounds proved a hand-rolled bash shell-parser on a security boundary is an
# un-winnable arms race. Hook A no longer parses argv; it is a fail-open
# substring tripwire (Task 3). The no-cheat guarantee is structural:
# B (edit-guard) + A2 (lint-suppression-aware recorder) + C (Stop-gate) + CI.
```

- [ ] **Step 5: Run to verify pass**

Run: `bash scripts/guard/test/run.sh`
Expected: PASS — every `_lib` line `ok`, ends `ALL GUARD TESTS PASSED`, exit 0.

- [ ] **Step 6: Commit**

```bash
git add -A scripts/guard .claude/hooks
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): bash guard-lib + test harness; drop Node .mjs draft (D9)"
```
Expected lefthook: `typos` runs (pass); fmt-check/clippy/taplo/cargo-deny skip (no `.rs`/`.toml`); `convco` passes.

---

### Task 2: A0 — `scripts/guard/turn-reset.sh` (`UserPromptSubmit`)

**Files:** Create `scripts/guard/turn-reset.sh`; modify `scripts/guard/test/run.sh`.

- [ ] **Step 1: Add failing test case** — insert ABOVE the `# HOOKMARK` line in `run.sh`:

```bash
# A0 turn-reset
TS_SID="t-a0"; TS_P="$(turn_state_path "$TS_SID" "$PWD")"
mkdir -p "$(dirname "$TS_P")"; printf '{"impl_files_edited":["x.rs"],"gate_green":["engine"]}' >"$TS_P"
printf '{"session_id":"%s","cwd":"%s"}' "$TS_SID" "$PWD" | bash "$HERE/turn-reset.sh"
chk "A0 clears impl" "[]" "$(jq -c '.impl_files_edited' "$TS_P")"
chk "A0 clears gate" "[]" "$(jq -c '.gate_green' "$TS_P")"
```

- [ ] **Step 2: Run** `bash scripts/guard/test/run.sh` → FAIL (`turn-reset.sh` missing).

- [ ] **Step 3: Implement `scripts/guard/turn-reset.sh`**

```bash
#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"
save_state "$p" "$(printf '{"session":"%s","started_at":"%s","impl_files_edited":[],"gate_green":[]}' "${sid:-unknown}" "$(date -u +%FT%TZ)")"
allow
```

- [ ] **Step 4: Run** `bash scripts/guard/test/run.sh` → PASS (A0 lines `ok`).

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/turn-reset.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A0 UserPromptSubmit turn-reset hook"
```

---

### Task 3: A — `scripts/guard/bash-deny.sh` (`PreToolUse/Bash`, fail-OPEN advisory tripwire — D10)

**Files:** Create `scripts/guard/bash-deny.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# A bash-deny  (D10: fail-OPEN advisory tripwire — NOT a security boundary)
adeny() { printf '%s' "$1" | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?; }
mk() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"},"cwd":"%s"}' "$1" "$PWD"; }
# blatant literal violations -> deny (helpful nudge; substring catches wrappers)
chk "A denies --no-verify"          2 "$(adeny "$(mk 'git commit -m wip --no-verify')")"
chk "A denies cargo fmt --all"      2 "$(adeny "$(mk 'cargo fmt --all')")"
chk "A denies wrapped fmt --all"    2 "$(adeny "$(mk 'timeout 600 cargo fmt --all')")"
chk "A denies git push --force"     2 "$(adeny "$(mk 'git push --force origin main')")"
# benign -> allow
chk "A allows conventional commit"  0 "$(adeny "$(mk 'git commit -m \"feat(x): y\"')")"
chk "A allows gh pr create"         0 "$(adeny "$(mk 'gh pr create --title \"Add X\"')")"
chk "A allows grep literal"         0 "$(adeny "$(mk 'grep -rn \"TODO\" crates/')")"
chk "A allows normal nextest"       0 "$(adeny "$(mk 'cargo nextest run -p nebula-engine')")"
chk "A allows push no force"        0 "$(adeny "$(mk 'git push origin main')")"
# fail-OPEN by design (obfuscation/ambiguity is B/A2/C's job, not A's)
chk "A fail-open on subshell"       0 "$(adeny "$(mk 'cargo \$(echo test)')")"
chk "A fail-open on non-Bash"       0 "$(printf '{"tool_name":"Edit"}' | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?)"
```

- [ ] **Step 2: Run** → FAIL (`bash-deny.sh` missing).

- [ ] **Step 3: Implement `scripts/guard/bash-deny.sh`**

```bash
#!/usr/bin/env bash
# D10: NOT a security boundary — a cheap fail-OPEN advisory tripwire. The real
# no-cheat guarantee is B (edit-guard) + A2 (lint-suppression-aware recorder)
# + C (Stop-gate) + lefthook/CI. Any doubt (no jq / non-Bash / unreadable /
# obfuscated / ambiguous) => allow. No shell parser; substring only.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
have_jq || allow
[ "$(jqg '.tool_name')" = "Bash" ] || allow
c="$(jqg '.tool_input.command')"; [ -n "$c" ] || allow
g() { printf '%s' "$c" | grep -Eq "$1"; }
# Blatant literal violations only. A rare false-deny on doc/message text is
# acceptable (advisory; the agent rewords). Obfuscation is intentionally NOT
# handled — its *outcome* is caught by B/A2/C.
if g 'git[[:space:]]+commit' && g '(--no-verify|--no-gpg-sign|core\.hooksPath=)'; then
  deny "Don't bypass lefthook (--no-verify/--no-gpg-sign/core.hooksPath). Fix what it flags."
fi
if g '(^|[[:space:]])cargo([[:space:]]|$)' && g '(^|[[:space:]])fmt([[:space:]]|$)' && g '(^|[[:space:]])--all([[:space:]]|$)'; then
  deny "cargo fmt --all trips Windows os-error-206 / false green. Use bash scripts/pre-commit-fmt-check.sh or cargo fmt -p <crate>."
fi
if g 'git[[:space:]]+push' && g '(--force([[:space:]]|=|$)|--force-with-lease|(^|[[:space:]])-f([[:space:]]|$))' && [ "${NEBULA_ALLOW_FORCE:-}" != "1" ]; then
  deny "Force-push to shared history blocked (AGENTS.md). Set NEBULA_ALLOW_FORCE=1 to override."
fi
allow
```

- [ ] **Step 4: Run** → PASS (all A lines `ok`).

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/bash-deny.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A fail-open PreToolUse advisory tripwire (D10)"
```

---

### Task 4: A2 — `scripts/guard/record.sh` (`PostToolUse/Bash`)

> **D10 design (grounded in verified harness facts):** `PostToolUse` fires
> ONLY for exit-0 Bash and `tool_response` is a structured object
> (`exit_code`/`success`/`stdout`). A2 records green via an **allowlist of the
> canonical CLEAN gate form** — reject any chaining/masking/redirect/comment
> (`|| && ; | $( \` > < #`), any suppression (`-A`/`--allow`/`--cap-lints`/
> `RUSTFLAGS=`), or a non-`cargo`/`task` argv0 (`echo`/`grep`…). Non-clean ⇒
> not recorded ⇒ C blocks (fail-safe, finite, no arms race). This is the
> load-bearing no-cheat layer; C (Task 6) is its only consumer.

**Files:** Create `scripts/guard/record.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# A2 record (D10: canonical-clean-form allowlist; structured tool_response;
# gate_green is jq `unique` => sorted)
R_SID="t-a2"; R_P="$(turn_state_path "$R_SID" "$PWD")"
mkdir -p "$(dirname "$R_P")"; printf '{"impl_files_edited":[],"gate_green":[]}' >"$R_P"
rr() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"},"tool_response":{"exit_code":%s,"success":%s,"stdout":"ok","stderr":""},"session_id":"%s","cwd":"%s"}' "$1" "${2:-0}" "${3:-true}" "$R_SID" "$PWD" | bash "$HERE/record.sh"; }
rr 'cargo nextest run -p nebula-engine'
chk "A2 records clean nextest" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'echo cargo clippy -p nebula-core -- -D warnings'
chk "A2 rejects echo (C-1/M-1)" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings || true'
chk "A2 rejects ||true (C-1)" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings 2>/dev/null'
chk "A2 rejects redirect (C-1)" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core --cap-lints allow -- -D warnings'
chk "A2 rejects --cap-lints (I-1)" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings -A clippy::all'
chk "A2 rejects -A suppression" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-aaa -p nebula-bbb -- -D warnings'
chk "A2 multi-p takes first (I-2)" '["aaa","engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings' 1 false
chk "A2 rejects exit!=0" '["aaa","engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings'
chk "A2 records clean clippy" '["aaa","core","engine"]' "$(jq -c '.gate_green' "$R_P")"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `scripts/guard/record.sh`**

```bash
#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.tool_name')" = "Bash" ] || allow
have_jq || allow
cmd="$(jqg '.tool_input.command')"
# Verified harness facts: PostToolUse fires only for exit-0 Bash; tool_response
# is a structured object. Trust the authenticated status; string-shape
# fallback treats a failure token as not-clean. Non-clean => not recorded.
ec="$(jqg '.tool_response.exit_code')"
[ -n "$ec" ] && [ "$ec" != "0" ] && allow
[ "$(jqg '.tool_response.success')" = "false" ] && allow
sresp="$(jqg '.tool_response')"
case "$sresp" in
  *'"exit_code"'*|*'"success"'*) : ;;
  *error*|*FAILED*|*"warning:"*|*"test result: FAILED"*) allow ;;
esac
# Record green ONLY for a CANONICAL CLEAN gate invocation — an ALLOWLIST of the
# exact clean shape, not a blocklist of evasions. Any chaining/masking/
# redirect/comment, any lint suppression, or a non-cargo/task argv0 => not
# recognized => not recorded => C blocks (fail-safe; agent runs gate plainly).
# Closes echo/||true/2>/dev/null/--cap-lints/RUSTFLAGS/multi-p/grep-of-docs.
case "$cmd" in
  *'||'*|*'&&'*|*';'*|*'|'*|*'`'*|*'$('*|*'>'*|*'<'*|*'#'*) allow ;;
  *' -A'*|*'--allow'*|*'--cap-lints'*|*'RUSTFLAGS='*) allow ;;
esac
core="$(printf '%s' "$cmd" | sed -E 's/^[[:space:]]*([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+)*//')"
is_gate=0
if   [[ "$core" =~ ^cargo([[:space:]]+\+[^[:space:]]+)?[[:space:]]+clippy([[:space:]]|$) ]] && [[ "$core" =~ (^|[[:space:]])-D([[:space:]]|$) ]]; then is_gate=1
elif [[ "$core" =~ ^cargo([[:space:]]+\+[^[:space:]]+)?[[:space:]]+nextest[[:space:]]+run([[:space:]]|$) ]]; then is_gate=1
elif [[ "$core" =~ ^task[[:space:]]+dev:check([[:space:]]|$) ]]; then is_gate=2
fi
[ "$is_gate" = 0 ] && allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"; st="$(load_state "$p")"
if [ "$is_gate" = 2 ]; then
  st="$(printf '%s' "$st" | jq -c '.gate_green = (.gate_green + ["*workspace*"] | unique)')"
else
  crate="$(printf '%s' "$core" | grep -oE -- '-p[[:space:]]+(nebula-)?[A-Za-z0-9_-]+' | head -1 | sed -E 's/^-p[[:space:]]+(nebula-)?//')"
  [ -n "$crate" ] && st="$(printf '%s' "$st" | jq -c --arg c "$crate" '.gate_green = (.gate_green + [$c] | unique)')"
fi
save_state "$p" "$st"
allow
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/record.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A2 PostToolUse gate-green recorder"
```

---

### Task 5: B — `scripts/guard/edit-guard.sh` (`PreToolUse/Edit|Write|MultiEdit`)

> **Known limitation:** B inspects incoming text (`Write.content` / `Edit.new_string` / `MultiEdit.edits[].new_string`). Inline `#[cfg(test)]` in a lib file can cause a false negative for the unwrap rule (clippy at the gate is the backstop). Test-weakening compares `old_string` vs `new_string` assert counts.

**Files:** Create `scripts/guard/edit-guard.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# B edit-guard
bdeny() { printf '%s' "$1" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; echo $?; }
W() { printf '{"tool_name":"Write","tool_input":{"file_path":"%s","content":"%s"},"cwd":"%s","session_id":"%s"}' "$1" "$2" "$PWD" "${3:-b-t}"; }
chk "B denies unwrap in lib"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' 'fn f(){ let x = g().unwrap(); }')")"
chk "B denies bare #[allow]"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' '#[allow(dead_code)]\nfn f(){}')")"
chk "B allows justified allow" 0 "$(bdeny "$(W 'crates/engine/src/state.rs' '// guard-justified: FFI shim\n#[allow(dead_code)]\nfn f(){}')")"
# test weakening while impl edited this turn
BW_SID="b-weaken"; BW_P="$(turn_state_path "$BW_SID" "$PWD")"
mkdir -p "$(dirname "$BW_P")"; printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":[]}' >"$BW_P"
EW='{"tool_name":"Edit","tool_input":{"file_path":"crates/engine/tests/retry.rs","old_string":"assert_eq!(got, want);","new_string":"assert!(true);"},"cwd":"'"$PWD"'","session_id":"'"$BW_SID"'"}'
chk "B denies test-weaken+impl" 2 "$(bdeny "$EW")"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `scripts/guard/edit-guard.sh`**

```bash
#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
tool="$(jqg '.tool_name')"
case "$tool" in Write|Edit|MultiEdit) :;; *) allow;; esac
have_jq || allow
file="$(jqg '.tool_input.file_path')"; [ -n "$file" ] || allow
case "$tool" in
  Write)  added="$(jqg '.tool_input.content')";;
  Edit)   added="$(jqg '.tool_input.new_string')";;
  MultiEdit) added="$(jqg '.tool_input.edits[].new_string')";;
esac
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"; st="$(load_state "$p")"
nf="${file//\\//}"
is_test=0
[[ "$nf" =~ /(tests|benches)/ ]] && is_test=1
printf '%s' "$added" | grep -qE '#\[(cfg\(test\)|test)\]' && is_test=1

if is_lib_rust "$file" && [ "$is_test" -eq 0 ]; then
  st="$(printf '%s' "$st" | jq -c --arg f "$nf" '.impl_files_edited = (.impl_files_edited + [$f] | unique)')"
  save_state "$p" "$st"
  printf '%s' "$added" | grep -qE '\.unwrap\(\)|\.expect\(|(^|[^A-Za-z_])panic!\(' \
    && deny "New unwrap()/expect()/panic!() in library code is forbidden (AGENTS.md). Use a typed thiserror variant."
  if printf '%s' "$added" | grep -qE '#\[allow\(|(^|[^A-Za-z_])(todo!|unimplemented!|unreachable!)\('; then
    printf '%s' "$added" | grep -qE '//[[:space:]]*guard-justified:' \
      || deny "allow/todo!/unimplemented!/unreachable! is a path-of-least-work escape. Fix it, or add a '// guard-justified: <reason>' line above."
  fi
  printf '%s' "$added" | grep -qE '//[[:space:]]*(TODO|FIXME|HACK|XXX)\b|TODO\([A-Z]+-?[0-9]|(^|[^A-Za-z])Phase[[:space:]][A-Z]\b' \
    && deny "TODO/FIXME/HACK/plan-id comments must not land in committed code."
  printf '%s' "$added" | grep -qE 'let[[:space:]]+_[[:space:]]*=[[:space:]]*[A-Za-z0-9_.]*(transition|send|write|commit|flush|lock|spawn)[A-Za-z0-9_]*\(' \
    && deny "let _ = <call> silently swallows a Result/must-use. Handle the error explicitly."
fi

if { [ "$tool" = Edit ] || [ "$tool" = MultiEdit ]; } && [[ "$nf" =~ /(tests|benches)/ ]]; then
  impl_n="$(printf '%s' "$st" | jq -r '.impl_files_edited | length')"
  if [ "${impl_n:-0}" -gt 0 ]; then
    case "$tool" in
      Edit) olds="$(jqg '.tool_input.old_string')"; news="$(jqg '.tool_input.new_string')";;
      MultiEdit) olds="$(jqg '.tool_input.edits[].old_string')"; news="$(jqg '.tool_input.edits[].new_string')";;
    esac
    oc="$(printf '%s' "$olds" | grep -oE '\bassert[A-Za-z_]*!' | wc -l | tr -d ' ')"
    nc="$(printf '%s' "$news" | grep -oE '\bassert[A-Za-z_]*!' | wc -l | tr -d ' ')"
    weak=0
    [ "${oc:-0}" -gt "${nc:-0}" ] && weak=1
    printf '%s' "$news" | grep -qE 'assert!\([[:space:]]*true[[:space:]]*\)|#\[ignore\]' && weak=1
    [ "$weak" -eq 1 ] && deny "Weakening a test (removed assert / assert!(true) / #[ignore]) while impl changed this turn is blocked. Fix the logic, not the test."
  fi
fi
allow
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/edit-guard.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): B PreToolUse edit anti-cheat guard"
```

---

### Task 6: C — `scripts/guard/stop-gate.sh` (`Stop`)

**Files:** Create `scripts/guard/stop-gate.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# C stop-gate
cstop() { printf '%s' "$1" | bash "$HERE/stop-gate.sh" >/dev/null 2>&1; echo $?; }
C_SID="c-blk"; C_P="$(turn_state_path "$C_SID" "$PWD")"; mkdir -p "$(dirname "$C_P")"
printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":[]}' >"$C_P"
chk "C blocks no-green"  2 "$(cstop '{"session_id":"'"$C_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":["engine"]}' >"$C_P"
chk "C allows green"     0 "$(cstop '{"session_id":"'"$C_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":[]}' >"$C_P"
chk "C no reblock loop"  0 "$(cstop '{"session_id":"'"$C_SID"'","cwd":"'"$PWD"'","stop_hook_active":true}')"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `scripts/guard/stop-gate.sh`**

```bash
#!/usr/bin/env bash
# Side-effect-free: reads turn-state only; runs no tools (deadlock-safe).
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.stop_hook_active')" = "true" ] && allow   # loop guard
have_jq || allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
st="$(load_state "$(turn_state_path "$sid" "$cwd")")"
mapfile -t files < <(printf '%s' "$st" | jq -r '.impl_files_edited[]?' )
[ "${#files[@]}" -eq 0 ] && allow
printf '%s' "$st" | jq -e '.gate_green | index("*workspace*")' >/dev/null 2>&1 && allow
declare -A seen; missing=""
for f in "${files[@]}"; do
  c="$(crate_of "$f")"; [ -n "$c" ] || continue
  [ -n "${seen[$c]:-}" ] && continue; seen[$c]=1
  if ! printf '%s' "$st" | jq -e --arg c "$c" '.gate_green | index($c)' >/dev/null 2>&1; then
    missing="$missing $c"
  fi
done
[ -z "$missing" ] && allow
deny "You changed crate(s)$missing but never showed clippy + nextest green for them this turn. Run \`cargo clippy -p nebula-<crate> -- -D warnings\` and \`cargo nextest run -p nebula-<crate>\` (or \`task dev:check\`) before claiming done. Weakening tests to get there is blocked by the edit guard."
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/stop-gate.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): C Stop falsifiable-finish gate"
```

---

### Task 7: D — `scripts/guard/fmt.sh` (`PostToolUse/Edit|Write|MultiEdit`)

**Files:** Create `scripts/guard/fmt.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# D fmt (must always exit 0, never block)
dfmt() { printf '%s' "$1" | bash "$HERE/fmt.sh" >/dev/null 2>&1; echo $?; }
chk "D exits 0 non-rust"  0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"README.md"},"cwd":"'"$PWD"'"}')"
chk "D exits 0 missing rs" 0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"crates/zzz/src/nope.rs"},"cwd":"'"$PWD"'"}')"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `scripts/guard/fmt.sh`**

```bash
#!/usr/bin/env bash
# Format-only, single file, never organize-imports (split-edit safe), never blocks.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
case "$(jqg '.tool_name')" in Write|Edit|MultiEdit) :;; *) allow;; esac
f="$(jqg '.tool_input.file_path')"
case "$f" in
  *.rs)   rustfmt --edition 2024 "$f" >/dev/null 2>&1 || true;;
  *.toml) command -v taplo >/dev/null 2>&1 && taplo fmt "$f" >/dev/null 2>&1 || true;;
esac
allow
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/fmt.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): D PostToolUse single-file formatter"
```

---

### Task 8: Wire committed `.claude/settings.json`

**Files:** Create `.claude/settings.json`.

- [ ] **Step 1: Create the file**

```json
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": [
      "Bash(cargo *)",
      "Bash(cargo nextest *)",
      "Bash(task *)",
      "Bash(git *)",
      "Bash(gh *)",
      "Bash(bash scripts/*)",
      "Bash(rustfmt *)",
      "Bash(taplo *)",
      "Bash(jq *)"
    ]
  },
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/turn-reset.sh\"" } ] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/bash-deny.sh\"" } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/edit-guard.sh\"" } ] }
    ],
    "PostToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/record.sh\"" } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/fmt.sh\"" } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard/stop-gate.sh\"" } ] }
    ]
  }
}
```

- [ ] **Step 2: Validate** — Run: `jq -e '.["$schema"] and (.hooks.PreToolUse|length==2) and (.hooks.Stop|length==1)' .claude/settings.json && echo "settings.json OK"`
Expected: `true` then `settings.json OK`.

- [ ] **Step 3: Commit**

```bash
git add .claude/settings.json
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): wire bash guard hooks in committed settings.json"
```

---

### Task 9: `task hooks:test` + CLAUDE.md Enforced-Discipline section

**Files:** Modify `Taskfile.yml`, `CLAUDE.md`.

- [ ] **Step 1: Add to the `tasks:` map in `Taskfile.yml`:**

```yaml
  hooks:test:
    desc: Run guard-hook test harness (bash)
    cmds:
      - bash scripts/guard/test/run.sh
```

- [ ] **Step 2: Verify** — Run: `task hooks:test`
Expected: `ALL GUARD TESTS PASSED`, exit 0.

- [ ] **Step 3: Append to `CLAUDE.md`:**

```markdown
## Enforced Discipline (guard hooks)

Mechanically enforced by `scripts/guard/*.sh` (committed in `.claude/settings.json`),
not advisory. `task hooks:test` proves each guard. **The no-cheat guarantee is
structural (D10): B (edit-guard) + A2 (clean-gate recorder) + C (Stop-gate) +
lefthook/CI.** Hook A is a **fail-open advisory tripwire**, not a security
boundary — it nudges on blatant literals only. Plan 2 makes this file canonical.

| Rule | Guard |
|------|-------|
| Nudge: blatant `git commit --no-verify` / `cargo fmt --all` / `git push --force` | `bash-deny.sh` (advisory, fail-open) |
| Lint-suppressed clippy never counts as a passing gate | `record.sh` (A2) |
| No `unwrap()/expect()/panic!()` in lib code | `edit-guard.sh` |
| `#[allow]/todo!/unimplemented!/unreachable!` need `// guard-justified:` | `edit-guard.sh` |
| No TODO/FIXME/HACK/plan-id in committed code | `edit-guard.sh` |
| No test-weakening while impl changed same turn | `edit-guard.sh` |
| Cannot end a turn with impl changed but no green clippy+nextest | `stop-gate.sh` |

Escape hatch for discretionary edit rules: a `// guard-justified: <reason>` line
directly above the construct. No escape for lefthook-bypass, lint-suppression,
or no-unwrap.
```

- [ ] **Step 4: Commit**

```bash
git add Taskfile.yml CLAUDE.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): task hooks:test + CLAUDE.md Enforced-Discipline map"
```

---

### Task 10: Integration smoke (acceptance §11)

**Files:** Modify `scripts/guard/test/run.sh` (append a scenario block before `# HOOKMARK`).

- [ ] **Step 1: Add the end-to-end scenario**

```bash
# Integration: cheat path (edit impl then neuter a test) => B denies
S_SID="smoke"; S_P="$(turn_state_path "$S_SID" "$PWD")"; mkdir -p "$(dirname "$S_P")"
printf '{"impl_files_edited":[],"gate_green":[]}' >"$S_P"
printf '{"tool_name":"Write","tool_input":{"file_path":"crates/engine/src/state.rs","content":"pub fn add(a:i32,b:i32)->i32{a+b}"},"cwd":"%s","session_id":"%s"}' "$PWD" "$S_SID" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1 || true
SE='{"tool_name":"Edit","tool_input":{"file_path":"crates/engine/tests/state.rs","old_string":"assert_eq!(add(2,2),4);","new_string":"assert!(true);"},"cwd":"'"$PWD"'","session_id":"'"$S_SID"'"}'
printf '%s' "$SE" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; chk "SMOKE cheat denied" 2 "$?"
# Integration: clean impl edit => allowed
printf '{"impl_files_edited":[],"gate_green":[]}' >"$S_P"
printf '{"tool_name":"Write","tool_input":{"file_path":"crates/engine/src/ok.rs","content":"pub fn add(a: i32, b: i32) -> i32 { a + b }"},"cwd":"%s","session_id":"%s"}' "$PWD" "$S_SID" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; chk "SMOKE clean allowed" 0 "$?"
```

- [ ] **Step 2: Run full suite** — Run: `task hooks:test`
Expected: `ALL GUARD TESTS PASSED`, exit 0.

- [ ] **Step 3: Commit**

```bash
git add scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "test(scripts): guard-hooks integration smoke (cheat denied / clean allowed)"
```

---

## Self-Review

**1. Spec coverage (spec § → task):** §4 runtime/contract (bash+jq, exit2, fail-open-except-A) → Tasks 1,3 ✓; §4.A0 → T2 ✓; §4.A fail-closed deny set → T3 ✓; §4.A2 record (+limitation) → T4 ✓; §4.B cheat/costyl/test-weaken → T5 ✓; §4.C stop + `stop_hook_active` + side-effect-free → T6 ✓; §4.D fmt-only → T7 ✓; §4 settings wiring + `$schema` + permissions → T8 ✓; §8.1 harness + `task hooks:test` → T1–10 ✓; §8.2 CLAUDE.md map → T9 ✓; §11 cheat-denied/clean-allowed → T10 ✓. D9 (bash, fail-closed A, scripts/guard) → whole plan ✓. Out of scope (correct): D8, G/H, lefthook-granularity, `nebula-pitfalls`, full permissions cleanup → Plans 2–4.

**2. Placeholder scan:** No TBD/TODO-as-instruction; every step has complete runnable code/commands with expected output. Literal `TODO`/`HACK` appear only as guard regex content.

**3. Consistency:** `_lib.sh` defines `read_input, jqg, have_jq, deny, allow, git_common_dir, turn_state_path, load_state, save_state, crate_of, is_lib_rust, normalize_argv0`; every hook sources `_lib.sh` and uses those exact names. Turn-state shape `{session,started_at,impl_files_edited[],gate_green[]}` written by A0, mutated by A2/B, read by C; `*workspace*` sentinel set by A2 (`task dev:check`) honored by C. `# HOOKMARK` insertion point is stable across Tasks 2–10. Blocking is uniformly `exit 2`; D never blocks.

---

## Execution Handoff

Already chosen: **Subagent-Driven** (superpowers:subagent-driven-development) — fresh implementer per task + spec then code-quality review, in this session, continuous.
