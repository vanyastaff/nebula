#!/usr/bin/env bash
# .claude/hooks/test/run.sh — guard-hook test harness. Exit 1 if any case fails.
#
# Scope after the Lean prune (Stop-gate retirement): the per-turn green-gate
# (stop-gate.sh) and the structural-budget gate (intent-gate.sh) were removed in
# favour of the global anti-lazy hooks + lefthook/CI (the authoritative code
# gate). This harness now covers only the surviving guards: turn-reset (A0),
# bash-deny (advisory tripwire), record (A2 — green recorder, retained but no
# longer read by any Stop gate), edit-guard (no-unwrap / justified-allows /
# no-TODO / test-weakening), and fmt.
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
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[],"turn_base":"","turn_base_patch_ids":[]}' "$(load_state "$LS_T")"; rm -f "$LS_T"
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
# gate_green is jq `unique` => sorted). Retained as a green recorder; no Stop
# gate reads it after the Lean prune, but the canonical-clean-form allowlist is
# still exercised so the recorder cannot silently regress.
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

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
