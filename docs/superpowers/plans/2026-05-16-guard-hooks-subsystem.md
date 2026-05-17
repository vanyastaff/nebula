# Guard Hooks Subsystem Implementation Plan (bash + jq — D9)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Harness-enforced, evasion-resistant guard hooks (bash + jq) so the agent cannot weaken tests, suppress lints, bypass lefthook, or claim "done" without a verified gate.

**Architecture (D10):** POSIX bash hooks under `.claude/hooks/` sharing `_lib.sh`, wired in committed `.claude/settings.json` (`command`-type). `jq` parses stdin. The no-cheat guarantee is **structural**, not parser-based: **B** (edit-guard, hard-deny) + **A2** (`record.sh` — records a green gate only for a canonical *clean* invocation; lint-suppressed/masked/redirected/echoed runs never count) + **C** (Stop-gate, hard-block) + lefthook/CI. **Hook A is a fail-OPEN advisory tripwire** (blatant literals only), not a security boundary — 5 adversarial rounds proved a hand-rolled bash shell-parser on a boundary is an un-winnable arms race, so the boundary was relocated to the oracle.

**Tech Stack:** bash 5 (git-bash on Windows — already required by lefthook), jq 1.8, git, Taskfile. No Node, no build step.

**Plan series (Plan 1 of 4 — spec `docs/superpowers/specs/2026-05-16-agent-discipline-and-curation-design.md`, decision D9):** 1=this; 2=D8 doc-canon inversion; 3=skill+subagent curation (G/H); 4=lefthook granularity (F)+`nebula-pitfalls` (E).

**Supersedes the Node `.mjs` draft.** Task 1 removes the obsolete `.claude/hooks/*.mjs` (commits `53707567`, `f275b4da`) and replaces them with `.claude/hooks/`.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `.claude/hooks/_lib.sh` | Shared: stdin read, jq extract, turn-state path/load/save, `crate_of`/`is_lib_rust`, `deny`/`allow` (no shell parser — D10) |
| `.claude/hooks/turn-reset.sh` | A0 `UserPromptSubmit`: reset turn-state |
| `.claude/hooks/bash-deny.sh` | A `PreToolUse/Bash`: fail-OPEN advisory tripwire — blatant no-verify / fmt --all / force-push only (D10; not a boundary) |
| `.claude/hooks/record.sh` | A2 `PostToolUse/Bash`: record green clippy/nextest per crate |
| `.claude/hooks/edit-guard.sh` | B `PreToolUse/Edit\|Write\|MultiEdit`: cheat/costyl + test-weakening |
| `.claude/hooks/stop-gate.sh` | C `Stop`: block done without recorded green gate |
| `.claude/hooks/fmt.sh` | D `PostToolUse/Edit\|Write\|MultiEdit`: format touched file |
| `.claude/hooks/test/run.sh` | bash assertion harness (`task hooks:test`) |
| `.claude/settings.json` | Committed wiring + `$schema` + curated permissions |
| `Taskfile.yml` | `task hooks:test` |
| `CLAUDE.md` | "Enforced Discipline" rule→guard map |

All hooks: `set -uo pipefail`; source `_lib.sh`; end with `allow` (exit 0); never exceed ~2 s.

---

### Task 1: Remove obsolete `.mjs` + create `.claude/hooks/_lib.sh`

**Files:**
- Delete: `.claude/hooks/guard-lib.mjs`, `.claude/hooks/__tests__/guard-lib.test.mjs`
- Create: `.claude/hooks/_lib.sh`, `.claude/hooks/test/run.sh`

- [ ] **Step 1: Remove the superseded Node scaffolding**

```bash
git rm .claude/hooks/guard-lib.mjs .claude/hooks/__tests__/guard-lib.test.mjs
rmdir .claude/hooks/__tests__ .claude/hooks 2>/dev/null || true
```

- [ ] **Step 2: Write the failing test harness `.claude/hooks/test/run.sh`**

```bash
#!/usr/bin/env bash
# .claude/hooks/test/run.sh — guard-hook test harness. Exit 1 if any case fails.
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
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' "$(load_state "$LS_T")"; rm -f "$LS_T"
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

Run: `bash .claude/hooks/test/run.sh`
Expected: FAIL — `_lib.sh` not found / function missing (non-zero exit).

- [ ] **Step 4: Implement `.claude/hooks/_lib.sh`**

```bash
# .claude/hooks/_lib.sh — shared helpers for Nebula guard hooks. Source, don't exec.
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
load_state() { # $1=path -> always {impl_files_edited:[...],gate_green:[...],turn_base:".."}
  local d='{"impl_files_edited":[],"gate_green":[],"turn_base":""}'
  if [ -f "$1" ] && have_jq && jq -e . "$1" >/dev/null 2>&1; then
    jq -c '{impl_files_edited:(if (.impl_files_edited|type)=="array" then .impl_files_edited else [] end),gate_green:(if (.gate_green|type)=="array" then .gate_green else [] end),turn_base:(if (.turn_base|type)=="string" then .turn_base else "" end)}' "$1" 2>/dev/null || printf '%s' "$d"
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

Run: `bash .claude/hooks/test/run.sh`
Expected: PASS — every `_lib` line `ok`, ends `ALL GUARD TESTS PASSED`, exit 0.

- [ ] **Step 6: Commit**

```bash
git add -A .claude/hooks
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): bash guard-lib + test harness; drop Node .mjs draft (D9)"
```
Expected lefthook: `typos` runs (pass); fmt-check/clippy/taplo/cargo-deny skip (no `.rs`/`.toml`); `convco` passes.

---

### Task 2: A0 — `.claude/hooks/turn-reset.sh` (`UserPromptSubmit`)

**Files:** Create `.claude/hooks/turn-reset.sh`; modify `.claude/hooks/test/run.sh`.

- [ ] **Step 1: Add failing test case** — insert ABOVE the `# HOOKMARK` line in `run.sh`:

```bash
# A0 turn-reset
TS_SID="t-a0"; TS_P="$(turn_state_path "$TS_SID" "$PWD")"
mkdir -p "$(dirname "$TS_P")"; printf '{"impl_files_edited":["x.rs"],"gate_green":["engine"]}' >"$TS_P"
printf '{"session_id":"%s","cwd":"%s"}' "$TS_SID" "$PWD" | bash "$HERE/turn-reset.sh"
chk "A0 clears impl" "[]" "$(jq -c '.impl_files_edited' "$TS_P")"
chk "A0 clears gate" "[]" "$(jq -c '.gate_green' "$TS_P")"
```

- [ ] **Step 2: Run** `bash .claude/hooks/test/run.sh` → FAIL (`turn-reset.sh` missing).

- [ ] **Step 3: Implement `.claude/hooks/turn-reset.sh`**

```bash
#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"
# Spec §4.C: record base HEAD so C can catch crate changes COMMITTED mid-turn
# (git status alone goes clean after a commit). --verify -q stays SILENT and
# exits non-zero on an unborn branch (zero commits): plain `rev-parse HEAD`
# would print the literal "HEAD" to stdout there, making turn_base non-empty so
# C runs a vacuous HEAD..HEAD diff. Empty turn_base => C skips the diff arm and
# degrades to git-status + B-union (the intended no-commits behavior).
tb="$(git -C "$cwd" rev-parse --verify -q HEAD 2>/dev/null || true)"
save_state "$p" "$(printf '{"session":"%s","started_at":"%s","impl_files_edited":[],"gate_green":[],"turn_base":"%s"}' "${sid:-unknown}" "$(date -u +%FT%TZ)" "$tb")"
allow
```

- [ ] **Step 4: Run** `bash .claude/hooks/test/run.sh` → PASS (A0 lines `ok`).

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/turn-reset.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A0 UserPromptSubmit turn-reset hook"
```

---

### Task 3: A — `.claude/hooks/bash-deny.sh` (`PreToolUse/Bash`, fail-OPEN advisory tripwire — D10)

**Files:** Create `.claude/hooks/bash-deny.sh`; modify `run.sh`.

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

- [ ] **Step 3: Implement `.claude/hooks/bash-deny.sh`**

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
git add .claude/hooks/bash-deny.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A fail-open PreToolUse advisory tripwire (D10)"
```

---

### Task 4: A2 — `.claude/hooks/record.sh` (`PostToolUse/Bash`)

> **D10 design (grounded in verified harness facts):** `PostToolUse` fires
> ONLY for exit-0 Bash and `tool_response` is a structured object
> (`exit_code`/`success`/`stdout`). A2 records green via an **allowlist of the
> canonical CLEAN gate form** — reject any chaining/masking/redirect/comment
> (`|| && ; | $( \` > < #`), any suppression (`-A`/`--allow`/`--cap-lints`/
> `RUSTFLAGS=`), or a non-`cargo`/`task` argv0 (`echo`/`grep`…). Non-clean ⇒
> not recorded ⇒ C blocks (fail-safe, finite, no arms race). This is the
> load-bearing no-cheat layer; C (Task 6) is its only consumer.

**Files:** Create `.claude/hooks/record.sh`; modify `run.sh`.

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
rr 'cargo clippy -p nebula-zzz -- -D warnings\ntrue'
chk "A2 rejects newline-joined (C-NL)" '["aaa","engine"]' "$(jq -c '.gate_green' "$R_P")"
rr 'cargo clippy -p nebula-core -- -D warnings'
chk "A2 records clean clippy" '["aaa","core","engine"]' "$(jq -c '.gate_green' "$R_P")"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `.claude/hooks/record.sh`**

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
  *'||'*|*'&&'*|*';'*|*'|'*|*'`'*|*'$('*|*'>'*|*'<'*|*'#'*|*$'\n'*|*$'\r'*|*$'\t'*) allow ;;
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
git add .claude/hooks/record.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A2 PostToolUse gate-green recorder"
```

---

### Task 5: B — `.claude/hooks/edit-guard.sh` (`PreToolUse/Edit|Write|MultiEdit`)

> **Known limitation:** B inspects incoming text (`Write.content` / `Edit.new_string` / `MultiEdit.edits[].new_string`). Inline `#[cfg(test)]` in a lib file can cause a false negative for the unwrap rule (clippy at the gate is the backstop). Test-weakening compares `old_string` vs `new_string` assert counts.

**Files:** Create `.claude/hooks/edit-guard.sh`; modify `run.sh`.

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
# D11/CRIT-1: src file whose payload has inline #[cfg(test)] is STILL recorded
C1_SID="b-crit1"; C1_P="$(turn_state_path "$C1_SID" "$PWD")"
mkdir -p "$(dirname "$C1_P")"; printf '{"impl_files_edited":[],"gate_green":[]}' >"$C1_P"
bdeny "$(W 'crates/zzz/src/m.rs' 'pub fn f()->u8{0}\n#[cfg(test)]\nmod t{}' "$C1_SID")" >/dev/null
chk "B records src w/ inline test (C-1)" 'true' "$(jq -r '.impl_files_edited|index("crates/zzz/src/m.rs")|type=="number"' "$C1_P" 2>/dev/null)"
# D11/IMPORTANT-2: two escapes, one justification -> deny
chk "B per-occurrence justified (I-2)" 2 "$(bdeny "$(W 'crates/engine/src/q.rs' '// guard-justified: a\n#[allow(x)]\n#[allow(y)]\nfn f(){}')")"
# D11/CRIT-2: Write overwrite of an EXISTING test file dropping asserts + impl edited -> deny
CW_SID="b-write"; CW_P="$(turn_state_path "$CW_SID" "$PWD")"
mkdir -p "$(dirname "$CW_P")"; printf '{"impl_files_edited":["crates/engine/src/x.rs"],"gate_green":[]}' >"$CW_P"
CW_F="$(mktemp -d)/zt.rs"; printf '#[test]\nfn t(){ assert_eq!(run(),1); assert!(ok()); }\n' >"$CW_F"
CW_J="$(printf '{"tool_name":"Write","tool_input":{"file_path":"%s","content":"#[test]\\nfn t(){ let _=run(); }"},"cwd":"%s","session_id":"%s"}' "$CW_F" "$PWD" "$CW_SID")"
chk "B denies Write-weaken test (C-2)" 2 "$(printf '%s' "$CW_J" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; echo $?)"
rm -rf "$(dirname "$CW_F")"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `.claude/hooks/edit-guard.sh`**

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
# D11/CRIT-1: record the impl edit by FILE PATH (is_lib_rust) UNCONDITIONALLY —
# never gated by payload #[cfg(test)] markers (C corroborates from git ground
# truth; this is the belt). The payload-test signal below gates ONLY the
# sub-checks where clippy genuinely backstops.
if is_lib_rust "$file"; then
  st="$(printf '%s' "$st" | jq -c --arg f "$nf" '.impl_files_edited = (.impl_files_edited + [$f] | unique)')"
  save_state "$p" "$st"
fi
is_test=0
[[ "$nf" =~ /(tests|benches)/ ]] && is_test=1
printf '%s' "$added" | grep -qE '#\[(cfg\(test\)|test)\]' && is_test=1
jcount=$(printf '%s' "$added" | grep -cE '//[[:space:]]*guard-justified:' || true)
ecount=$(printf '%s' "$added" | grep -oE '#\[[[:space:]]*allow[[:space:]]*\(|(^|[^A-Za-z_])(todo!|unimplemented!|unreachable!)[[:space:]]*\(' | wc -l | tr -d ' ')

if is_lib_rust "$file" && [ "$is_test" -eq 0 ]; then
  if printf '%s' "$added" | grep -qE '\.[[:space:]]*unwrap[[:space:]]*(::<[^>]*>)?[[:space:]]*\(\)|\.[[:space:]]*expect[[:space:]]*\(|(^|[^A-Za-z_])panic![[:space:]]*\(|(Option|Result)[[:space:]]*::[[:space:]]*(unwrap|expect)[[:space:]]*\('; then
    [ "${jcount:-0}" -ge 1 ] || deny "New unwrap()/expect()/panic!() in library code is forbidden (AGENTS.md). Use a typed thiserror variant, or justify with '// guard-justified: <reason>'."
  fi
  # D11/IMPORTANT-2: per-occurrence — N escapes need >= N justifications.
  [ "${ecount:-0}" -gt "${jcount:-0}" ] \
    && deny "allow/todo!/unimplemented!/unreachable! is a path-of-least-work escape — each needs its own '// guard-justified: <reason>' ($ecount escape(s), $jcount justification(s))."
  printf '%s' "$added" | grep -qE '//[[:space:]]*(TODO|FIXME|HACK|XXX)\b|TODO\([A-Z]+-?[0-9]|(^|[^A-Za-z])Phase[[:space:]][A-Z]\b' \
    && deny "TODO/FIXME/HACK/plan-id comments must not land in committed code."
  printf '%s' "$added" | grep -qE 'let[[:space:]]+_[[:space:]]*=[[:space:]]*([A-Za-z0-9_.]*[._])?(transition|send|write|commit|flush|lock|spawn)[A-Za-z0-9_]*[[:space:]]*\(' \
    && deny "let _ = <call> silently swallows a Result/must-use. Handle the error explicitly."
fi

# D11/CRIT-2,3: test-weaken covers Edit/MultiEdit AND Write, on a /tests|benches/
# path OR a lib file with inline #[cfg(test)]. Only when impl changed this turn.
impl_n="$(printf '%s' "$st" | jq -r '.impl_files_edited | length')"
if [ "${impl_n:-0}" -gt 0 ]; then
  assert_count() { printf '%s' "$1" | grep -oE '\bassert[A-Za-z_]*!|#\[(test|should_panic)\]' | wc -l | tr -d ' '; }
  is_testish=0
  [[ "$nf" =~ /(tests|benches)/ ]] && is_testish=1
  printf '%s' "$added" | grep -qE '#\[(cfg\(test\)|test)\]' && is_testish=1
  if [ "$is_testish" -eq 1 ]; then
    case "$tool" in
      Edit) o="$(jqg '.tool_input.old_string')"; n="$(jqg '.tool_input.new_string')";;
      MultiEdit) o="$(jqg '.tool_input.edits[].old_string')"; n="$(jqg '.tool_input.edits[].new_string')";;
      Write) o="$( __f="${file//\\//}"; [ -f "$__f" ] && cat -- "$__f" || printf '' )"; n="$added";;
    esac
    weak=0
    [ "$(assert_count "$o")" -gt "$(assert_count "$n")" ] && weak=1
    printf '%s' "$n" | grep -qE 'assert!\([[:space:]]*(true|1[[:space:]]*==[[:space:]]*1)[[:space:]]*\)|#\[ignore\]' && weak=1
    [ "$weak" -eq 1 ] && deny "Weakening a test (fewer asserts/#[test], assert!(true)/tautology/#[ignore]) while impl changed this turn is blocked. Fix the logic, not the test."
  fi
fi
allow
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/edit-guard.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): B PreToolUse edit anti-cheat guard"
```

---

### Task 6: C — `.claude/hooks/stop-gate.sh` (`Stop`)

**Files:** Create `.claude/hooks/stop-gate.sh`; modify `run.sh`.

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
# D11: git ground-truth derivation (independent of turn-state recording)
CG_DIR="$(mktemp -d)"; ( cd "$CG_DIR" && git init -q && mkdir -p crates/zzz/src && echo 'fn f(){}' > crates/zzz/src/a.rs )
CG_SID="c-git"; CG_P="$(turn_state_path "$CG_SID" "$CG_DIR")"; mkdir -p "$(dirname "$CG_P")"
printf '{"impl_files_edited":[],"gate_green":[]}' >"$CG_P"
chk "C blocks via git diff" 2 "$(cstop '{"session_id":"'"$CG_SID"'","cwd":"'"$CG_DIR"'","stop_hook_active":false}')"
printf '{"impl_files_edited":[],"gate_green":["zzz"]}' >"$CG_P"
chk "C allows git+green"   0 "$(cstop '{"session_id":"'"$CG_SID"'","cwd":"'"$CG_DIR"'","stop_hook_active":false}')"
# Renamed src file (git mv) must still be detected: the git-status rename arrow is stripped so C checks the NEW path
( cd "$CG_DIR" && git add -A && git -c user.email=t@t -c user.name=t commit -qm x && mkdir -p crates/yyy/src && git mv crates/zzz/src/a.rs crates/yyy/src/b.rs )
printf '{"impl_files_edited":[],"gate_green":[]}' >"$CG_P"
chk "C detects renamed src (#2)" 2 "$(cstop '{"session_id":"'"$CG_SID"'","cwd":"'"$CG_DIR"'","stop_hook_active":false}')"
# CRITICAL-1: a src path with a SPACE (git C-quotes it w/o -z) must still be detected
SP_DIR="$(mktemp -d)"; ( cd "$SP_DIR" && git init -q && mkdir -p "crates/sp/src" && echo 'fn f(){}' > "crates/sp/src/a b.rs" )
SP_SID="c-sp"; SP_P="$(turn_state_path "$SP_SID" "$SP_DIR")"; mkdir -p "$(dirname "$SP_P")"
printf '{"impl_files_edited":[],"gate_green":[]}' >"$SP_P"
chk "C detects space-in-path (C-1)" 2 "$(cstop '{"session_id":"'"$SP_SID"'","cwd":"'"$SP_DIR"'","stop_hook_active":false}')"
rm -rf "$SP_DIR"
# Spec §4.C: a crate change COMMITTED mid-turn, B-bypassed, must still DENY
# via turn_base..HEAD (git status is clean after the commit; B never saw it)
TB_DIR="$(mktemp -d)"
( cd "$TB_DIR" && git init -q && mkdir -p crates/tb/src && echo 'fn a(){}' > crates/tb/src/x.rs && git add -A && git -c user.email=t@t -c user.name=t commit -qm base )
TB_BASE="$(git -C "$TB_DIR" rev-parse HEAD)"
( cd "$TB_DIR" && echo 'fn a(){ 1 }' > crates/tb/src/x.rs && git add -A && git -c user.email=t@t -c user.name=t commit -qm change )
TB_SID="c-tb"; TB_P="$(turn_state_path "$TB_SID" "$TB_DIR")"; mkdir -p "$(dirname "$TB_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"%s"}' "$TB_BASE" >"$TB_P"
chk "C catches committed-this-turn (§4.C)" 2 "$(cstop '{"session_id":"'"$TB_SID"'","cwd":"'"$TB_DIR"'","stop_hook_active":false}')"
rm -rf "$TB_DIR"
# §4.C edge: A0 on an unborn branch (zero commits) must record EMPTY turn_base,
# not the literal "HEAD". Plain `rev-parse HEAD` echoes "HEAD" to stdout there,
# making turn_base non-empty so C's [ -n "$tb" ] guard runs a vacuous
# HEAD..HEAD diff and a first-ever B-bypassed commit escapes. --verify -q must
# yield "" so C correctly skips the diff arm.
UB_DIR="$(mktemp -d)"; ( cd "$UB_DIR" && git init -q )
UB_SID="c-ub"; UB_P="$(turn_state_path "$UB_SID" "$UB_DIR")"; mkdir -p "$(dirname "$UB_P")"
printf '{"session_id":"%s","cwd":"%s"}' "$UB_SID" "$UB_DIR" | bash "$HERE/turn-reset.sh"
chk "A0 unborn branch => empty turn_base (§4.C)" '""' "$(jq -c '.turn_base' "$UB_P")"
rm -rf "$UB_DIR"
rm -rf "$CG_DIR"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `.claude/hooks/stop-gate.sh`**

```bash
#!/usr/bin/env bash
# D11: touched-crate set from git GROUND TRUTH (not solely B's recording).
# `git` here is read-only and triggers no tools — Stop-hook-safe. Over-
# detection is the safe direction (more crates must be green, never fewer).
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
[ "$(jqg '.stop_hook_active')" = "true" ] && allow   # loop guard (deadlock-safe)
have_jq || allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
st="$(load_state "$(turn_state_path "$sid" "$cwd")")"
printf '%s' "$st" | jq -e '.gate_green | index("*workspace*")' >/dev/null 2>&1 && allow
declare -A touched=()
_consider() {  # $1=path -> record its crate if it is a crate src .rs
  local p="${1%$'\r'}"  # jq -r emits CRLF on git-bash; CR is never a path char
  printf '%s' "$p" | tr '\\' '/' | grep -qE '(^|/)crates/[^/]+/src/.*\.rs$' || return 0
  local c; c="$(crate_of "$p")"; [ -n "$c" ] && touched[$c]=1
}
# git ground truth: NUL-delimited, UNQUOTED paths (-z) — no quoting/sed-arrow
# pitfalls. Rename/Copy records are `XY new\0old`; gate BOTH paths + deletions.
while IFS= read -r -d '' rec; do
  [ -n "$rec" ] || continue
  xy="${rec:0:2}"; pth="${rec:3}"
  case "$xy" in
    R*|C*) _consider "$pth"; IFS= read -r -d '' old && _consider "$old" ;;
    *)     _consider "$pth" ;;
  esac
done < <(git -C "$cwd" status --porcelain -z -u 2>/dev/null)
# Spec §4.C 3rd source: changes COMMITTED this turn (git status goes clean
# after a commit; turn-state isn't reset by commit). turn_base = HEAD at A0.
tb="$(printf '%s' "$st" | jq -r '.turn_base // empty' 2>/dev/null)"
if [ -n "$tb" ]; then
  while IFS= read -r -d '' f; do [ -n "$f" ] && _consider "$f"; done \
    < <(git -C "$cwd" diff --name-only -z "$tb"..HEAD 2>/dev/null)
fi
# corroborating B-union (turn-state recording — NEVER git-only; D11/constraint 1)
while IFS= read -r f; do [ -n "$f" ] && _consider "$f"; done < <(printf '%s' "$st" | jq -r '.impl_files_edited[]?' 2>/dev/null)
(( ${#touched[@]} == 0 )) && allow
missing=""
for c in "${!touched[@]}"; do
  printf '%s' "$st" | jq -e --arg c "$c" '.gate_green | index($c)' >/dev/null 2>&1 || missing="$missing $c"
done
[ -z "$missing" ] && allow
deny "You changed crate(s)$missing but never showed a clean clippy + nextest green for them. Run \`cargo clippy -p nebula-<crate> -- -D warnings\` and \`cargo nextest run -p nebula-<crate>\` (or \`task dev:check\`) before claiming done. (Touched set = git diff ground truth; weakening tests cannot help — A2 records green only for a clean gate, CI re-runs.)"
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/stop-gate.sh .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): C Stop falsifiable-finish gate"
```

---

### Task 7: D — `.claude/hooks/fmt.sh` (`PostToolUse/Edit|Write|MultiEdit`)

**Files:** Create `.claude/hooks/fmt.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# D fmt (must always exit 0, never block)
dfmt() { printf '%s' "$1" | bash "$HERE/fmt.sh" >/dev/null 2>&1; echo $?; }
chk "D exits 0 non-rust"  0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"README.md"},"cwd":"'"$PWD"'"}')"
chk "D exits 0 missing rs" 0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"crates/zzz/src/nope.rs"},"cwd":"'"$PWD"'"}')"
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `.claude/hooks/fmt.sh`**

```bash
#!/usr/bin/env bash
# Format-only, single file, never organize-imports (split-edit safe), never blocks.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
case "$(jqg '.tool_name')" in Write|Edit|MultiEdit) :;; *) allow;; esac
f="$(jqg '.tool_input.file_path')"; f="${f%$'\r'}"  # jq -r CRLF on git-bash
case "$f" in
  *.rs)   rustfmt --edition 2024 "$f" >/dev/null 2>&1 || true;;
  *.toml) command -v taplo >/dev/null 2>&1 && taplo fmt "$f" >/dev/null 2>&1 || true;;
esac
allow
```

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/fmt.sh .claude/hooks/test/run.sh
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
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/turn-reset.sh\"" } ] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/bash-deny.sh\"" } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/edit-guard.sh\"" } ] }
    ],
    "PostToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/record.sh\"" } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/fmt.sh\"" } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/.claude/hooks/stop-gate.sh\"" } ] }
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
      - bash .claude/hooks/test/run.sh
```

- [ ] **Step 2: Verify** — Run: `task hooks:test`
Expected: `ALL GUARD TESTS PASSED`, exit 0.

- [ ] **Step 3: Append to `CLAUDE.md`:**

```markdown
## Enforced Discipline (guard hooks)

Mechanically enforced by `.claude/hooks/*.sh` (committed in `.claude/settings.json`),
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

**Files:** Modify `.claude/hooks/test/run.sh` (append a scenario block before `# HOOKMARK`).

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
git add .claude/hooks/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "test(scripts): guard-hooks integration smoke (cheat denied / clean allowed)"
```

---

## Self-Review

> **SUPERSEDED (point-in-time, pre-D9 draft).** This Self-Review block reflects
> the original Node-`.mjs`, fail-closed-A draft. The authoritative current
> design is the spec's decision rows **D9–D11** + §4 (bash+jq runtime; hook A
> demoted to a fail-OPEN advisory tripwire; `resolve_cmd`/`normalize_argv0`
> deleted; the structural no-cheat guarantee is **C-via-git-diff + A2-clean-gate
> + CI**, with B an early advisory). The Task 1–10 *code* in this plan is
> D11-correct and was implemented + adversarially reviewed accordingly. Read
> this block as history, not as the design.

**1. Spec coverage (spec § → task):** §4 runtime/contract (bash+jq, exit2, fail-open-except-A) → Tasks 1,3 ✓; §4.A0 → T2 ✓; §4.A fail-closed deny set → T3 ✓; §4.A2 record (+limitation) → T4 ✓; §4.B cheat/costyl/test-weaken → T5 ✓; §4.C stop + `stop_hook_active` + side-effect-free → T6 ✓; §4.D fmt-only → T7 ✓; §4 settings wiring + `$schema` + permissions → T8 ✓; §8.1 harness + `task hooks:test` → T1–10 ✓; §8.2 CLAUDE.md map → T9 ✓; §11 cheat-denied/clean-allowed → T10 ✓. D9 (bash, fail-closed A, .claude/hooks) → whole plan ✓. Out of scope (correct): D8, G/H, lefthook-granularity, `nebula-pitfalls`, full permissions cleanup → Plans 2–4.

**2. Placeholder scan:** No TBD/TODO-as-instruction; every step has complete runnable code/commands with expected output. Literal `TODO`/`HACK` appear only as guard regex content.

**3. Consistency:** `_lib.sh` defines `read_input, jqg, have_jq, deny, allow, git_common_dir, turn_state_path, load_state, save_state, crate_of, is_lib_rust, normalize_argv0`; every hook sources `_lib.sh` and uses those exact names. Turn-state shape `{session,started_at,impl_files_edited[],gate_green[]}` written by A0, mutated by A2/B, read by C; `*workspace*` sentinel set by A2 (`task dev:check`) honored by C. `# HOOKMARK` insertion point is stable across Tasks 2–10. Blocking is uniformly `exit 2`; D never blocks.

---

## Execution Handoff

Already chosen: **Subagent-Driven** (superpowers:subagent-driven-development) — fresh implementer per task + spec then code-quality review, in this session, continuous.
