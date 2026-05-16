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
chk "normalize_argv0 resolves quoted argv0" cargo "$(normalize_argv0 "ca'rg'o fmt --all")"
chk "normalize_argv0 resolves dquote argv0" git "$(normalize_argv0 'g"i"t push --force')"
chk "normalize_argv0 keeps quoted arg value" git "$(normalize_argv0 'git commit -m "fix: bug"')"
chk "normalize_argv0 unrelated quoted ok" gh "$(normalize_argv0 'gh pr create --title "X Y"')"
chk "normalize_argv0 wrapper-only UNPARSEABLE" UNPARSEABLE "$(normalize_argv0 'sudo')"
chk "normalize_argv0 fail-closed unbalanced quote" UNPARSEABLE "$(normalize_argv0 'echo "oops')"
chk "normalize_argv0 fail-closed env -S" UNPARSEABLE "$(normalize_argv0 'env -S cargo fmt --all')"
chk "normalize_argv0 strips windows path and exe suffix" cargo "$(normalize_argv0 'C:\tools\cargo.exe fmt --all')"
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

# A bash-deny  (run hook, capture exit code)
adeny() { printf '%s' "$1" | bash "$HERE/bash-deny.sh" >/dev/null 2>&1; echo $?; }
mk() { printf '{"tool_name":"Bash","tool_input":{"command":"%s"},"cwd":"%s"}' "$1" "$PWD"; }
chk "A denies --no-verify (wrapped)" 2 "$(adeny "$(mk 'env X=1 git commit -m wip --no-verify')")"
chk "A denies clippy -A"            2 "$(adeny "$(mk 'cargo clippy -p nebula-engine -- -A clippy::all')")"
chk "A denies cargo fmt --all"      2 "$(adeny "$(mk 'cargo fmt --all')")"
chk "A denies timeout-wrapped fmt --all" 2 "$(adeny "$(mk 'timeout 600 cargo fmt --all')")"
chk "A fail-closed on subshell"     2 "$(adeny "$(mk 'cargo \$(echo test)')")"
chk "A allows normal nextest"       0 "$(adeny "$(mk 'cargo nextest run -p nebula-engine')")"
chk "A allows conventional commit"  0 "$(adeny "$(mk 'git commit -m \"feat(x): y\"')")"
chk "A allows gh pr create quoted"  0 "$(adeny "$(mk 'gh pr create --title \"Add X\"')")"
chk "A allows grep string literal"  0 "$(adeny "$(mk 'grep -rn \"TODO\" crates/')")"
chk "A denies quoted-token bypass"  2 "$(adeny "$(mk 'cargo \"fmt\" --all')")"
chk "A denies env -S fmt --all"     2 "$(adeny "$(mk 'env -S cargo fmt --all')")"

# Per-hook cases are appended by later tasks below this line. # HOOKMARK

[ "$fail" -eq 0 ] && echo "ALL GUARD TESTS PASSED" || echo "GUARD TESTS FAILED"
exit "$fail"
