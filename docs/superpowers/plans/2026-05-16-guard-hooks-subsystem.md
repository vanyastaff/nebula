# Guard Hooks Subsystem Implementation Plan (bash + jq — D9)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Harness-enforced, evasion-resistant guard hooks (bash + jq) so the agent cannot weaken tests, suppress lints, bypass lefthook, or claim "done" without a verified gate.

**Architecture:** Six POSIX bash hooks under `scripts/guard/` sharing `_lib.sh`, wired in committed `.claude/settings.json` (`command`-type). Blocking = `exit 2` + stderr. `jq` parses stdin. Hooks fail **open** on internal error EXCEPT `bash-deny.sh` (hook A) which fails **closed**: any command it cannot confidently normalize (`$(`, backticks, `${`, `;`, newline, unbalanced quotes, no resolvable argv0) ⇒ **deny**. Conservative-and-fail-closed beats a clever tokenizer that can be evaded.

**Tech Stack:** bash 5 (git-bash on Windows — already required by lefthook), jq 1.8, git, Taskfile. No Node, no build step.

**Plan series (Plan 1 of 4 — spec `docs/superpowers/specs/2026-05-16-agent-discipline-and-curation-design.md`, decision D9):** 1=this; 2=D8 doc-canon inversion; 3=skill+subagent curation (G/H); 4=lefthook granularity (F)+`nebula-pitfalls` (E).

**Supersedes the Node `.mjs` draft.** Task 1 removes the obsolete `.claude/hooks/*.mjs` (commits `53707567`, `f275b4da`) and replaces them with `scripts/guard/`.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `scripts/guard/_lib.sh` | Shared: stdin read, jq extract, fail-closed `normalize_argv0`, turn-state path/load/save, `crate_of`/`is_lib_rust`, `deny`/`allow` |
| `scripts/guard/turn-reset.sh` | A0 `UserPromptSubmit`: reset turn-state |
| `scripts/guard/bash-deny.sh` | A `PreToolUse/Bash`: fail-closed deny (no-verify, clippy -A, fmt --all, force-push) |
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
chk "normalize_argv0 strips env+wrappers" cargo "$(normalize_argv0 'FOO=1 env BAR=2 sudo cargo clippy -- -D warnings')"
chk "normalize_argv0 unwraps timeout value" cargo "$(normalize_argv0 'timeout 600 cargo clippy -- -D warnings')"
chk "normalize_argv0 unwraps sudo -u value" cargo "$(normalize_argv0 'sudo -u root cargo build')"
chk "normalize_argv0 nice -n value" cargo "$(normalize_argv0 'nice -n 10 cargo nextest run')"
chk "normalize_argv0 fail-closed on subshell" UNPARSEABLE "$(normalize_argv0 'cargo $(echo test)')"
chk "normalize_argv0 fail-closed on chaining" UNPARSEABLE "$(normalize_argv0 'cargo test; rm -rf x')"
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
load_state() { # $1=path
  if [ -f "$1" ] && have_jq && jq -e . "$1" >/dev/null 2>&1; then cat "$1"
  else printf '{"impl_files_edited":[],"gate_green":[]}'; fi
}
save_state() { mkdir -p "$(dirname "$1")" 2>/dev/null && printf '%s' "$2" >"$1" 2>/dev/null || true; }

crate_of() { # $1=path -> crate name or empty
  local p="${1//\\//}"
  [[ "$p" =~ (^|/)crates/([^/]+)/ ]] && printf '%s' "${BASH_REMATCH[2]}"
}
is_lib_rust() { # $1=path -> return 0 if library rust
  local p="${1//\\//}"
  [[ "$p" == *.rs ]] || return 1
  [[ "$p" =~ (^|/)crates/[^/]+/src/ ]] || return 1
  [[ "$p" =~ /(tests|benches|examples)/ ]] && return 1
  [[ "$p" =~ /(main|build)\.rs$ ]] && return 1
  return 0
}
# Fail-closed: echoes argv0, or "UNPARSEABLE" for anything we cannot safely
# analyze (caller MUST treat UNPARSEABLE as deny).
normalize_argv0() { # $1=raw command
  local c="$1"
  case "$c" in *'$('*|*'`'*|*'${'*|*';'*|*'&&'*|*'||'*|*$'\n'*) printf 'UNPARSEABLE'; return;; esac
  local dq="${c//[^\"]/}" sq="${c//[^\']/}"
  if (( ${#dq} % 2 != 0 || ${#sq} % 2 != 0 )); then printf 'UNPARSEABLE'; return; fi
  local -a t; read -ra t <<< "$c"
  local n=${#t[@]} i=0
  local -A WRAP=([env]=1 [sudo]=1 [nice]=1 [timeout]=1 [watch]=1 [xargs]=1 [command]=1 [stdbuf]=1 [nohup]=1)
  local -A VF=([-u]=1 [-g]=1 [-n]=1 [-C]=1 [-k]=1 [-s]=1 [-S]=1 [-h]=1 [-d]=1 [-o]=1 [-e]=1)
  while (( i < n )); do
    local w="${t[$i]}"
    if [[ "$w" =~ ^[A-Za-z_][A-Za-z0-9_]*= && "$w" != */* ]]; then ((i++)); continue; fi
    local base="${w##*/}"
    if [[ -n "${WRAP[$base]:-}" ]]; then
      ((i++))
      while (( i < n )); do
        local x="${t[$i]}"
        if [[ "$x" == -* ]]; then ((i++)); if [[ -n "${VF[$x]:-}" && $i -lt $n && "${t[$i]}" != -* ]]; then ((i++)); fi; continue; fi
        if [[ "$x" =~ ^[0-9]+(\.[0-9]+)?[smhdKMG]?$ ]]; then ((i++)); continue; fi
        if [[ "$x" =~ ^[A-Za-z_][A-Za-z0-9_]*= && "$x" != */* ]]; then ((i++)); continue; fi
        break
      done
      continue
    fi
    break
  done
  if (( i >= n )); then printf 'UNPARSEABLE'; return; fi
  local a="${t[$i]}"; printf '%s' "${a##*/}"
}
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

### Task 3: A — `scripts/guard/bash-deny.sh` (`PreToolUse/Bash`, fail-closed)

**Files:** Create `scripts/guard/bash-deny.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# A bash-deny  (run hook, capture exit code)
adeny() { printf '%s' "$1" | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?; }
mk() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"},"cwd":"%s"}' "$1" "$PWD"; }
chk "A denies --no-verify (wrapped)" 2 "$(adeny "$(mk 'env X=1 git commit -m wip --no-verify')")"
chk "A denies clippy -A"            2 "$(adeny "$(mk 'cargo clippy -p nebula-engine -- -A clippy::all')")"
chk "A denies cargo fmt --all"      2 "$(adeny "$(mk 'cargo fmt --all')")"
chk "A denies timeout-wrapped fmt --all" 2 "$(adeny "$(mk 'timeout 600 cargo fmt --all')")"
chk "A fail-closed on subshell"     2 "$(adeny "$(mk 'cargo \$(echo test)')")"
chk "A allows normal nextest"       0 "$(adeny "$(mk 'cargo nextest run -p nebula-engine')")"
```

- [ ] **Step 2: Run** → FAIL (`bash-deny.sh` missing).

- [ ] **Step 3: Implement `scripts/guard/bash-deny.sh`**

```bash
#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
have_jq || deny "jq is required by the bash guard and is missing (fail-closed). Install jq."
[ "$(jqg '.tool_name')" = "Bash" ] || allow
cmd="$(jqg '.tool_input.command')"; [ -n "$cmd" ] || allow
argv0="$(normalize_argv0 "$cmd")"
[ "$argv0" = "UNPARSEABLE" ] && deny "Command too complex to verify safely (subshell/chaining/quotes). Run it as a single plain command."
raw="$cmd"
if [ "$argv0" = git ] && [[ "$raw" =~ (^|[[:space:]])commit([[:space:]]|$) ]] \
   && [[ "$raw" =~ (--no-verify|(^|[[:space:]])-n([[:space:]]|$)|--no-gpg-sign|core\.hooksPath=) ]]; then
  deny "Bypassing lefthook is the top-level cheat. Commit without --no-verify/-n/--no-gpg-sign; fix what the hook flags."
fi
if [ "$argv0" = cargo ] && [[ "$raw" =~ (^|[[:space:]])clippy([[:space:]]|$) ]] \
   && [[ "$raw" =~ ([[:space:]]-A[[:space:]]|--allow[[:space:]]|RUSTFLAGS=[^\&]*-A) ]]; then
  deny "Silencing clippy to reach green is cheating the oracle. Fix the lint or add a justified #[allow] in code."
fi
if [ "$argv0" = cargo ] && [[ "$raw" =~ (^|[[:space:]])fmt([[:space:]]|$) ]] \
   && [[ "$raw" =~ ([[:space:]]|^)--all([[:space:]]|$) ]]; then
  deny "cargo fmt --all trips Windows os-error-206 and false green. Use bash scripts/pre-commit-fmt-check.sh or cargo fmt -p <crate>."
fi
if [ "$argv0" = git ] && [[ "$raw" =~ (^|[[:space:]])push([[:space:]]|$) ]] \
   && [[ "$raw" =~ (--force([[:space:]]|=|$)|--force-with-lease|(^|[[:space:]])-f([[:space:]]|$)) ]] \
   && [ "${NEBULA_ALLOW_FORCE:-}" != "1" ]; then
  deny "Force-push to shared history is blocked (AGENTS.md). Set NEBULA_ALLOW_FORCE=1 only if you truly mean it."
fi
allow
```

- [ ] **Step 4: Run** → PASS (all A lines `ok`).

- [ ] **Step 5: Commit**

```bash
git add scripts/guard/bash-deny.sh scripts/guard/test/run.sh
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A fail-closed PreToolUse Bash deny guard"
```

---

### Task 4: A2 — `scripts/guard/record.sh` (`PostToolUse/Bash`)

> **Known limitation:** `PostToolUse` exposes `tool_response` but no guaranteed exit code. A2 records a crate green only when the command is a recognized gate command AND `tool_response` shows no failure token (`error`, `FAILED`, `warning:`, `test result: FAILED`). Heuristic; the Stop gate (Task 6) is the backstop.

**Files:** Create `scripts/guard/record.sh`; modify `run.sh`.

- [ ] **Step 1: Add failing cases** above `# HOOKMARK`:

```bash
# A2 record
R_SID="t-a2"; R_P="$(turn_state_path "$R_SID" "$PWD")"
mkdir -p "$(dirname "$R_P")"; printf '{"impl_files_edited":[],"gate_green":[]}' >"$R_P"
printf '{"tool_name":"Bash","tool_input":{"command":"cargo nextest run -p nebula-engine"},"tool_response":"12 passed","session_id":"%s","cwd":"%s"}' "$R_SID" "$PWD" | bash "$HERE/record.sh"
chk "A2 records green" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
printf '{"tool_name":"Bash","tool_input":{"command":"cargo clippy -p nebula-core -- -D warnings"},"tool_response":"error: aborting","session_id":"%s","cwd":"%s"}' "$R_SID" "$PWD" | bash "$HERE/record.sh"
chk "A2 ignores failed" '["engine"]' "$(jq -c '.gate_green' "$R_P")"
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
cmd="$(jqg '.tool_input.command')"; resp="$(jqg '.tool_response')"
case "$resp" in *error*|*FAILED*|*"warning:"*|*"test result: FAILED"*) allow;; esac
is_gate=0
[[ "$cmd" =~ cargo[[:space:]]+clippy.*-D ]] && is_gate=1
[[ "$cmd" =~ cargo[[:space:]]+nextest[[:space:]]+run ]] && is_gate=1
[[ "$cmd" =~ (^|[[:space:]])task[[:space:]]+dev:check ]] && is_gate=2
[ "$is_gate" = 0 ] && allow
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"; st="$(load_state "$p")"
if [ "$is_gate" = 2 ]; then
  st="$(printf '%s' "$st" | jq -c '.gate_green = (.gate_green + ["*workspace*"] | unique)')"
else
  crate="$(printf '%s' "$cmd" | sed -n 's/.*-p[[:space:]]\{1,\}\(nebula-\)\{0,1\}\([A-Za-z0-9_-]\{1,\}\).*/\2/p')"
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
not advisory. `task hooks:test` proves each guard. Hook A is fail-closed
(un-parseable command ⇒ deny). Plan 2 makes this file canonical.

| Rule | Guard |
|------|-------|
| No `git commit --no-verify` / lefthook bypass | `bash-deny.sh` |
| No clippy `-A`/`--allow`/`RUSTFLAGS` suppression | `bash-deny.sh` |
| No `cargo fmt --all` (Windows 206 / false green) | `bash-deny.sh` |
| Unverifiable/obfuscated shell command | `bash-deny.sh` (fail-closed) |
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
