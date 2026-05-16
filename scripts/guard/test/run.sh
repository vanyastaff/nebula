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
LS_T="$(mktemp)"; printf '{"impl_files_edited":"oops"}' >"$LS_T"
chk "load_state normalizes bad shape" '{"impl_files_edited":[],"gate_green":[]}' "$(load_state "$LS_T")"; rm -f "$LS_T"
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
chk "A fail-open on subshell"       0 "$(adeny "$(mk 'cargo \$(echo test)')")"
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

# B edit-guard
bdeny() { printf '%s' "$1" | bash "$HERE/edit-guard.sh" >/dev/null 2>&1; echo $?; }
W() { printf '{"tool_name":"Write","tool_input":{"file_path":"%s","content":"%s"},"cwd":"%s","session_id":"%s"}' "$1" "$2" "$PWD" "${3:-b-t}"; }
chk "B denies unwrap in lib"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' 'fn f(){ let x = g().unwrap(); }')")"
chk "B denies bare #[allow]"   2 "$(bdeny "$(W 'crates/engine/src/state.rs' '#[allow(dead_code)]\nfn f(){}')")"
chk "B allows justified allow" 0 "$(bdeny "$(W 'crates/engine/src/state.rs' '// guard-justified: FFI shim\n#[allow(dead_code)]\nfn f(){}')")"
BW_SID="b-weaken"; BW_P="$(turn_state_path "$BW_SID" "$PWD")"
mkdir -p "$(dirname "$BW_P")"; printf '{"impl_files_edited":["crates/engine/src/state.rs"],"gate_green":[]}' >"$BW_P"
EW='{"tool_name":"Edit","tool_input":{"file_path":"crates/engine/tests/retry.rs","old_string":"assert_eq!(got, want);","new_string":"assert!(true);"},"cwd":"'"$PWD"'","session_id":"'"$BW_SID"'"}'
chk "B denies test-weaken+impl" 2 "$(bdeny "$EW")"

# Per-hook cases are appended by later tasks below this line. # HOOKMARK

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
