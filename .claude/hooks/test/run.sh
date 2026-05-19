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
LS_T="$(mktemp)"; printf '{"impl_files_edited":"oops"}' >"$LS_T"
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[],"turn_base":""}' "$(load_state "$LS_T")"; rm -f "$LS_T"
chk "crate_of extracts" engine "$(crate_of 'crates/engine/src/engine.rs')"
chk "crate_of windows path" engine "$(crate_of 'crates\\engine\\src\\engine.rs')"
chk "crate_of none" "" "$(crate_of 'README.md')"
is_lib_rust 'crates/engine/src/state.rs'        && chk "is_lib_rust src" 0 0 || chk "is_lib_rust src" 0 1
is_lib_rust 'crates/engine/tests/retry.rs'      && chk "is_lib_rust tests" 1 0 || chk "is_lib_rust tests" 1 1
is_lib_rust 'crates\\engine\\src\\state.rs'     && chk "is_lib_rust win" 0 0 || chk "is_lib_rust win" 0 1

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

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
