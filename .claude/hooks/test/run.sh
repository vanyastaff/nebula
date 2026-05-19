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

# Shared fixture for the rebase/squash-merge regression. Simulates
# `git rebase --onto <newbase> <old-turn-base> <branch>` after an unrelated PR
# (crates/up) was squash-merged into the new base, while THIS turn really only
# touched crates/mine. Echoes "<dir>|<old_turn_base_sha>". root/b1 are empty so
# only the non-empty `mine` commit is replayed (no rebase empty-commit ambiguity).
# `-b base` keeps the initial branch distinct from the later `main` branch even
# when the user's init.defaultBranch is "main".
# $1=#lines crates/up/src/u.rs (rebased-in, NOT this turn)
# $2=#lines crates/mine/src/m.rs (this turn's real change)
mk_rebase_repo() {
  local d up_n="$1" mine_n="$2" root b1 i
  d="$(mktemp -d)"
  git -C "$d" init -q -b base 2>/dev/null || git -C "$d" init -q
  git -C "$d" symbolic-ref HEAD refs/heads/base 2>/dev/null || true
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm root --allow-empty
  root="$(git -C "$d" rev-parse HEAD)"
  git -C "$d" checkout -q -b feature
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm b1 --allow-empty
  b1="$(git -C "$d" rev-parse HEAD)"
  mkdir -p "$d/crates/mine/src"
  for i in $(seq 1 "$mine_n"); do echo "// mine $i"; done > "$d/crates/mine/src/m.rs"
  git -C "$d" add -A
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm mine
  git -C "$d" checkout -q -b main "$root"
  mkdir -p "$d/crates/up/src"
  for i in $(seq 1 "$up_n"); do echo "// up $i"; done > "$d/crates/up/src/u.rs"
  git -C "$d" add -A
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm squash-unrelated
  git -C "$d" checkout -q feature
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t rebase --onto main "$b1" feature -q >/dev/null 2>&1
  printf '%s|%s' "$d" "$b1"
}

# Variant covering Codex review #3269664222 (P1): the branch already has a
# pre-turn commit (`crates/old`) when the turn begins; `git rebase --onto main`
# replays BOTH `old` and this turn's `new` onto an unrelated upstream. A0
# stores the patch-id of `old`; effective_turn_base walks the rebased branch
# and matches that patch-id against the new line, recovering the rewritten
# `old'` as the effective base so the diff stays scoped to `crates/new` only.
# Echoes "<dir>|<pre_turn_base_sha>|<patch_id_of_old>|<rewritten_old_sha>".
# $1=#lines crates/up/src/u.rs   $2=#lines crates/old/src/o.rs (pre-turn)
# $3=#lines crates/new/src/n.rs (this turn)
mk_rebase_repo_preturn() {
  local d up_n="$1" old_n="$2" new_n="$3" root pre_tb pid rewritten i
  d="$(mktemp -d)"
  git -C "$d" init -q -b base 2>/dev/null || git -C "$d" init -q
  git -C "$d" symbolic-ref HEAD refs/heads/base 2>/dev/null || true
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm root --allow-empty
  root="$(git -C "$d" rev-parse HEAD)"
  git -C "$d" checkout -q -b feature
  mkdir -p "$d/crates/old/src"
  for i in $(seq 1 "$old_n"); do echo "// old $i"; done > "$d/crates/old/src/o.rs"
  git -C "$d" add -A
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm old
  pre_tb="$(git -C "$d" rev-parse HEAD)"
  pid="$(git -C "$d" show "$pre_tb" 2>/dev/null | git patch-id --stable 2>/dev/null | awk 'NF>0{print $1; exit}')"
  mkdir -p "$d/crates/new/src"
  for i in $(seq 1 "$new_n"); do echo "// new $i"; done > "$d/crates/new/src/n.rs"
  git -C "$d" add -A
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm new
  git -C "$d" checkout -q -b main "$root"
  mkdir -p "$d/crates/up/src"
  for i in $(seq 1 "$up_n"); do echo "// up $i"; done > "$d/crates/up/src/u.rs"
  git -C "$d" add -A
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm squash-unrelated
  git -C "$d" checkout -q feature
  git -C "$d" -c commit.gpgsign=false -c user.email=t@t -c user.name=t rebase --onto main "$root" feature -q >/dev/null 2>&1
  rewritten="$(git -C "$d" rev-list --reverse main..feature 2>/dev/null | head -n 1)"
  printf '%s|%s|%s|%s' "$d" "$pre_tb" "$pid" "$rewritten"
}

# --- _lib unit checks ---
LS_T="$(mktemp)"; printf '{"impl_files_edited":"oops"}' >"$LS_T"
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[],"turn_base":"","turn_base_patch_ids":[]}' "$(load_state "$LS_T")"; rm -f "$LS_T"
chk "crate_of extracts" engine "$(crate_of 'crates/engine/src/engine.rs')"
chk "crate_of windows path" engine "$(crate_of 'crates\\engine\\src\\engine.rs')"
chk "crate_of none" "" "$(crate_of 'README.md')"
is_lib_rust 'crates/engine/src/state.rs'        && chk "is_lib_rust src" 0 0 || chk "is_lib_rust src" 0 1
is_lib_rust 'crates/engine/tests/retry.rs'      && chk "is_lib_rust tests" 1 0 || chk "is_lib_rust tests" 1 1
is_lib_rust 'crates\\engine\\src\\state.rs'     && chk "is_lib_rust win" 0 0 || chk "is_lib_rust win" 0 1
# effective_turn_base: ancestor passthrough / empty / repin-on-history-rewrite
ETB_D="$(mktemp -d)"
git -C "$ETB_D" init -q -b base 2>/dev/null || git -C "$ETB_D" init -q
git -C "$ETB_D" symbolic-ref HEAD refs/heads/base 2>/dev/null || true
git -C "$ETB_D" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm root --allow-empty
ETB_ROOT="$(git -C "$ETB_D" rev-parse HEAD)"
git -C "$ETB_D" checkout -q -b feature
git -C "$ETB_D" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm b1 --allow-empty
ETB_B1="$(git -C "$ETB_D" rev-parse HEAD)"
echo x > "$ETB_D/f.txt"; git -C "$ETB_D" add -A
git -C "$ETB_D" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm work
chk "eff-base ancestor passthrough" "$ETB_B1" "$(effective_turn_base "$ETB_D" "$ETB_B1" </dev/null)"
chk "eff-base empty stays empty"    ""         "$(effective_turn_base "$ETB_D" "" </dev/null)"
git -C "$ETB_D" checkout -q -b main "$ETB_ROOT"
echo y > "$ETB_D/g.txt"; git -C "$ETB_D" add -A
git -C "$ETB_D" -c commit.gpgsign=false -c user.email=t@t -c user.name=t commit -qm upstream
git -C "$ETB_D" checkout -q feature
git -C "$ETB_D" -c commit.gpgsign=false -c user.email=t@t -c user.name=t rebase --onto main "$ETB_B1" feature -q >/dev/null 2>&1
ETB_MB="$(git -C "$ETB_D" merge-base HEAD main)"
chk "eff-base repins stale base"    "$ETB_MB"   "$(effective_turn_base "$ETB_D" "$ETB_B1" </dev/null)"
rm -rf "$ETB_D"
# eff-base with stored patch-ids: pre-turn branch commit replayed by rebase.
# Walking the new line and matching the stored patch-id recovers the rewritten
# pre-turn commit (NOT just the upstream merge-base), so the diff arm scopes
# to this turn's `crates/new` and ignores the replayed `crates/old`.
ETBP="$(mk_rebase_repo_preturn 1 1 1)"
ETBP_DIR="$(printf '%s' "$ETBP" | awk -F'|' '{print $1}')"
ETBP_PRE="$(printf '%s' "$ETBP" | awk -F'|' '{print $2}')"
ETBP_PID="$(printf '%s' "$ETBP" | awk -F'|' '{print $3}')"
ETBP_REWRITTEN="$(printf '%s' "$ETBP" | awk -F'|' '{print $4}')"
chk "eff-base patch-id recovers rewritten" "$ETBP_REWRITTEN" "$(printf '%s\n' "$ETBP_PID" | effective_turn_base "$ETBP_DIR" "$ETBP_PRE")"
rm -rf "$ETBP_DIR"

# A0 turn-reset
TS_SID="t-a0"; TS_P="$(turn_state_path "$TS_SID" "$PWD")"
mkdir -p "$(dirname "$TS_P")"; printf '{"impl_files_edited":["x.rs"],"gate_green":["engine"]}' >"$TS_P"
printf '{"session_id":"%s","cwd":"%s"}' "$TS_SID" "$PWD" | bash "$HERE/turn-reset.sh"
chk "A0 clears impl" "[]" "$(jq -c '.impl_files_edited' "$TS_P")"
chk "A0 clears gate" "[]" "$(jq -c '.gate_green' "$TS_P")"

# A bash-deny  (D10: fail-OPEN advisory tripwire — NOT a security boundary)
adeny() { printf '%s' "$1" | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?; }
mk() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"},"cwd":"%s"}' "$1" "$PWD"; }
chk "A denies --no-verify"          2 "$(adeny "$(mk 'git commit -m wip --no-verify')")"
chk "A denies cargo fmt --all"      2 "$(adeny "$(mk 'cargo fmt --all')")"
chk "A denies wrapped fmt --all"    2 "$(adeny "$(mk 'timeout 600 cargo fmt --all')")"
chk "A denies git push --force"     2 "$(adeny "$(mk 'git push --force origin main')")"
chk "A allows conventional commit"  0 "$(adeny "$(mk 'git commit -m \"feat(x): y\"')")"
chk "A allows gh pr create"         0 "$(adeny "$(mk 'gh pr create --title \"Add X\"')")"
chk "A allows grep literal"         0 "$(adeny "$(mk 'grep -rn \"TODO\" crates/')")"
chk "A allows normal nextest"       0 "$(adeny "$(mk 'cargo nextest run -p nebula-engine')")"
chk "A allows push no force"        0 "$(adeny "$(mk 'git push origin main')")"
chk "A fail-open on subshell"       0 "$(adeny "$(mk 'cargo $(echo test)')")"
chk "A fail-open on non-Bash"       0 "$(printf '{"tool_name":"Edit"}' | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?)"

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
# PR #673: a non-`warnings` -D (e.g. -D clippy::all) does NOT enforce the
# documented CI contract — must NOT count as a green gate.
rr 'cargo clippy -p nebula-ddd -- -D clippy::all'
chk "A2 rejects -D non-warnings (#673)" '["aaa","core","engine"]' "$(jq -c '.gate_green' "$R_P")"
# PR #673 (CodeRabbit): --package / --package= are valid cargo forms; a clean
# run with them must record gate_green (else honest agents are false-blocked).
rr 'cargo clippy --package nebula-eee -- -D warnings'
chk "A2 records --package form (#673)" '["aaa","core","eee","engine"]' "$(jq -c '.gate_green' "$R_P")"

# B edit-guard
bdeny() { printf '%s' "$1" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; echo $?; }
W() { printf '{"tool_name":"Write","tool_input":{"file_path":"%s","content":"%s"},"cwd":"%s","session_id":"%s"}' "$1" "$2" "$PWD" "${3:-b-t}"; }
chk "B denies unwrap in lib"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' 'fn f(){ let x = g().unwrap(); }')")"
chk "B denies bare #[allow]"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' '#[allow(dead_code)]\nfn f(){}')")"
chk "B allows justified allow" 0 "$(bdeny "$(W 'crates/engine/src/state.rs' '// guard-justified: FFI shim\n#[allow(dead_code)]\nfn f(){}')")"
# PR #673: no-unwrap has NO escape (CLAUDE.md) — a guard-justified line must
# NOT let unwrap()/expect()/panic!() through in library code.
chk "B denies unwrap even w/ guard-justified (#673)" 2 "$(bdeny "$(W 'crates/engine/src/state.rs' '// guard-justified: legacy\nfn f(){ let x = g().unwrap(); }')")"
BW_SID="b-weaken"; BW_P="$(turn_state_path "$BW_SID" "$PWD")"
mkdir -p "$(dirname "$BW_P")"; printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":[]}' >"$BW_P"
EW='{"tool_name":"Edit","tool_input":{"file_path":"crates/engine/tests/retry.rs","old_string":"assert_eq!(got, want);","new_string":"assert!(true);"},"cwd":"'"$PWD"'","session_id":"'"$BW_SID"'"}'
chk "B denies test-weaken+impl" 2 "$(bdeny "$EW")"
C1_SID="b-crit1"; C1_P="$(turn_state_path "$C1_SID" "$PWD")"
mkdir -p "$(dirname "$C1_P")"; printf '{"impl_files_edited":[],"gate_green":[]}' >"$C1_P"
bdeny "$(W 'crates/zzz/src/m.rs' 'pub fn f()->u8{0}\n#[cfg(test)]\nmod t{}' "$C1_SID")" >/dev/null
chk "B records src w/ inline test (C-1)" 'true' "$(jq -r '.impl_files_edited|index("crates/zzz/src/m.rs")|type=="number"' "$C1_P" 2>/dev/null)"
chk "B per-occurrence justified (I-2)" 2 "$(bdeny "$(W 'crates/engine/src/q.rs' '// guard-justified: a\n#[allow(x)]\n#[allow(y)]\nfn f(){}')")"
CW_SID="b-write"; CW_P="$(turn_state_path "$CW_SID" "$PWD")"
mkdir -p "$(dirname "$CW_P")"; printf '{"impl_files_edited":["crates/engine/src/x.rs"],"gate_green":[]}' >"$CW_P"
CW_F="$(mktemp -d)/zt.rs"; printf '#[test]\nfn t(){ assert_eq!(run(),1); assert!(ok()); }\n' >"$CW_F"
CW_J="$(printf '{"tool_name":"Write","tool_input":{"file_path":"%s","content":"#[test]\\nfn t(){ let _=run(); }"},"cwd":"%s","session_id":"%s"}' "$CW_F" "$PWD" "$CW_SID")"
chk "B denies Write-weaken test (C-2)" 2 "$(printf '%s' "$CW_J" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; echo $?)"
rm -rf "$(dirname "$CW_F")"

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
# Rebase robustness: after `git rebase --onto` reparents the branch onto a base
# that squash-merged an unrelated PR (crates/up), the stale turn_base must NOT
# make C flag crates only changed by that already-merged work. crates/mine is
# this turn's real change.
RB="$(mk_rebase_repo 1 1)"; RB_DIR="${RB%%|*}"; RB_B1="${RB##*|}"
RB_SID="c-rebase"; RB_P="$(turn_state_path "$RB_SID" "$RB_DIR")"; mkdir -p "$(dirname "$RB_P")"
printf '{"impl_files_edited":[],"gate_green":["mine"],"turn_base":"%s"}' "$RB_B1" >"$RB_P"
chk "C ignores rebased-in crate" 0 "$(cstop '{"session_id":"'"$RB_SID"'","cwd":"'"$RB_DIR"'","stop_hook_active":false}')"
# Genuine no-cheat protection survives the fix: the crate this turn DID change,
# with no recorded green gate, is still blocked (repin must not over-relax).
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"%s"}' "$RB_B1" >"$RB_P"
chk "C still blocks real post-rebase crate" 2 "$(cstop '{"session_id":"'"$RB_SID"'","cwd":"'"$RB_DIR"'","stop_hook_active":false}')"
rm -rf "$RB_DIR"
# P1 (Codex review #3269664222): when the branch already had pre-turn commits
# before A0, `git rebase --onto` replays them onto the new base. A0 stores the
# patch-ids of those pre-turn commits; effective_turn_base walks the rebased
# line, matches the patch-id of `old`, and uses the rewritten `old'` as the
# effective base — so C ignores the replayed pre-turn crate and only flags
# this-turn's `crates/new` (which is green here).
RBP="$(mk_rebase_repo_preturn 1 1 1)"
RBP_DIR="$(printf '%s' "$RBP" | awk -F'|' '{print $1}')"
RBP_PRE="$(printf '%s' "$RBP" | awk -F'|' '{print $2}')"
RBP_PID="$(printf '%s' "$RBP" | awk -F'|' '{print $3}')"
RBP_SID="c-rebase-pre"; RBP_P="$(turn_state_path "$RBP_SID" "$RBP_DIR")"; mkdir -p "$(dirname "$RBP_P")"
printf '{"impl_files_edited":[],"gate_green":["new"],"turn_base":"%s","turn_base_patch_ids":["%s"]}' "$RBP_PRE" "$RBP_PID" >"$RBP_P"
chk "C ignores pre-turn branch commit (P1)" 0 "$(cstop '{"session_id":"'"$RBP_SID"'","cwd":"'"$RBP_DIR"'","stop_hook_active":false}')"
# Genuine protection survives: same fixture, this-turn crate `new` NOT green
# -> still blocked (patch-id walk must not over-relax).
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"%s","turn_base_patch_ids":["%s"]}' "$RBP_PRE" "$RBP_PID" >"$RBP_P"
chk "C still blocks real this-turn crate (P1)" 2 "$(cstop '{"session_id":"'"$RBP_SID"'","cwd":"'"$RBP_DIR"'","stop_hook_active":false}')"
rm -rf "$RBP_DIR"
# D fmt (must always exit 0, never block)
dfmt() { printf '%s' "$1" | bash "$HERE/fmt.sh" >/dev/null 2>&1; echo $?; }
chk "D exits 0 non-rust"  0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"README.md"},"cwd":"'"$PWD"'"}')"
chk "D exits 0 missing rs" 0 "$(dfmt '{"tool_name":"Write","tool_input":{"file_path":"crates/zzz/src/nope.rs"},"cwd":"'"$PWD"'"}')"
# Integration: cheat path (edit impl then neuter a test) => B denies
S_SID="smoke"; S_P="$(turn_state_path "$S_SID" "$PWD")"; mkdir -p "$(dirname "$S_P")"
printf '{"impl_files_edited":[],"gate_green":[]}' >"$S_P"
printf '{"tool_name":"Write","tool_input":{"file_path":"crates/engine/src/state.rs","content":"pub fn add(a:i32,b:i32)->i32{a+b}"},"cwd":"%s","session_id":"%s"}' "$PWD" "$S_SID" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1 || true
SE='{"tool_name":"Edit","tool_input":{"file_path":"crates/engine/tests/state.rs","old_string":"assert_eq!(add(2,2),4);","new_string":"assert!(true);"},"cwd":"'"$PWD"'","session_id":"'"$S_SID"'"}'
printf '%s' "$SE" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; chk "SMOKE cheat denied" 2 "$?"
# Integration: clean impl edit => allowed
printf '{"impl_files_edited":[],"gate_green":[]}' >"$S_P"
printf '{"tool_name":"Write","tool_input":{"file_path":"crates/engine/src/ok.rs","content":"pub fn add(a: i32, b: i32) -> i32 { a + b }"},"cwd":"%s","session_id":"%s"}' "$PWD" "$S_SID" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; chk "SMOKE clean allowed" 0 "$?"
# Per-hook cases are appended by later tasks below this line. # HOOKMARK

# E intent-gate (ADR-0083 deterministic structural-budget tier)
egate() { printf '%s' "$1" | bash "$HERE/intent-gate.sh" >/dev/null 2>&1; echo $?; }
E_SID="e-skel"; E_P="$(turn_state_path "$E_SID" "$PWD")"; mkdir -p "$(dirname "$E_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$E_P"
chk "E loop-guard allows"   0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":true}')"
chk "E default allows"      0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":["crates/engine/src/x.rs"],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$E_P"
chk "E defers to C broken"  0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":2}' >"$E_P"
chk "E loop-bound allows"   0 "$(egate '{"session_id":"'"$E_SID"'","cwd":"'"$PWD"'","stop_hook_active":false}')"

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
# Regression (tracked): staged code whose lines start with `+` must also count.
# git diff emits such a line as `++…`; the old `^\+([^+]|$)` grep rejected it
# (only the untracked `+ ` sentinel was protected). The awk count fixes both.
EBT_DIR="$(mktemp -d)"
( cd "$EBT_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/ebt/src && { for i in $(seq 1 450); do echo "+marker $i"; done; } > crates/ebt/src/plus.rs \
  && git add -A )
EBT_SID="e-bud-trk"; EBT_P="$(turn_state_path "$EBT_SID" "$EBT_DIR")"; mkdir -p "$(dirname "$EBT_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"","intent_attempts":0}' >"$EBT_P"
chk "E counts +-prefixed tracked" 2 "$(egate '{"session_id":"'"$EBT_SID"'","cwd":"'"$EBT_DIR"'","stop_hook_active":false}')"
rm -rf "$EBT_DIR"

# E new-file budget (cap 5)
EF_DIR="$(mktemp -d)"
( cd "$EF_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/ef/src && for i in 1 2 3 4 5 6; do echo "fn f${i}(){}" > "crates/ef/src/m${i}.rs"; done )
EF_SID="e-nf"; EF_P="$(turn_state_path "$EF_SID" "$EF_DIR")"; mkdir -p "$(dirname "$EF_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EF_P"
chk "E blocks >5 new files" 2 "$(egate '{"session_id":"'"$EF_SID"'","cwd":"'"$EF_DIR"'","stop_hook_active":false}')"
rm -rf "$EF_DIR"

# E large-blob proxy (single added run > 100 lines in one code file)
EL_DIR="$(mktemp -d)"
( cd "$EL_DIR" && git init -q && git -c user.email=t@t -c user.name=t commit -qm init --allow-empty \
  && mkdir -p crates/el/src && { echo 'fn big(){'; for i in $(seq 1 130); do echo "  let v$i=$i;"; done; echo '}'; } > crates/el/src/f.rs )
EL_SID="e-blob"; EL_P="$(turn_state_path "$EL_SID" "$EL_DIR")"; mkdir -p "$(dirname "$EL_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' >"$EL_P"
chk "E blocks >100-line blob" 2 "$(egate '{"session_id":"'"$EL_SID"'","cwd":"'"$EL_DIR"'","stop_hook_active":false}')"
rm -rf "$EL_DIR"

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
# E rebase robustness: a stale turn_base after `git rebase --onto` must not make
# intent-gate count the squash-merged upstream delta (crates/up = 450 LoC)
# against this turn's tiny real change (crates/mine = 1 LoC).
ERB="$(mk_rebase_repo 450 1)"; ERB_DIR="${ERB%%|*}"; ERB_B1="${ERB##*|}"
ERB_SID="e-rebase"; ERB_P="$(turn_state_path "$ERB_SID" "$ERB_DIR")"; mkdir -p "$(dirname "$ERB_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"%s","intent_attempts":0}' "$ERB_B1" >"$ERB_P"
chk "E ignores rebased-in net-LoC" 0 "$(egate '{"session_id":"'"$ERB_SID"'","cwd":"'"$ERB_DIR"'","stop_hook_active":false}')"
rm -rf "$ERB_DIR"
# E P1 (Codex review #3269664222): big pre-turn `crates/old` (450 LoC) +
# this-turn `crates/new` (1 LoC); after rebase, A0-stored patch-id of `old`
# locates the rewritten `old'`, so intent-gate counts only the +1 net delta,
# not the replayed 450 LoC.
ERBP="$(mk_rebase_repo_preturn 1 450 1)"
ERBP_DIR="$(printf '%s' "$ERBP" | awk -F'|' '{print $1}')"
ERBP_PRE="$(printf '%s' "$ERBP" | awk -F'|' '{print $2}')"
ERBP_PID="$(printf '%s' "$ERBP" | awk -F'|' '{print $3}')"
ERBP_SID="e-rebase-pre"; ERBP_P="$(turn_state_path "$ERBP_SID" "$ERBP_DIR")"; mkdir -p "$(dirname "$ERBP_P")"
printf '{"impl_files_edited":[],"gate_green":[],"turn_base":"%s","turn_base_patch_ids":["%s"],"intent_attempts":0}' "$ERBP_PRE" "$ERBP_PID" >"$ERBP_P"
chk "E ignores pre-turn branch LoC (P1)" 0 "$(egate '{"session_id":"'"$ERBP_SID"'","cwd":"'"$ERBP_DIR"'","stop_hook_active":false}')"
rm -rf "$ERBP_DIR"

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
