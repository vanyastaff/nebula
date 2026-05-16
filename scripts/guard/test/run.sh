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
chk "normalize_argv0 fail-closed quoted argv0" UNPARSEABLE "$(normalize_argv0 "ca'rg'o fmt --all")"
chk "normalize_argv0 fail-closed dquote argv0" UNPARSEABLE "$(normalize_argv0 'g"i"t push --force')"
chk "normalize_argv0 wrapper-only UNPARSEABLE" UNPARSEABLE "$(normalize_argv0 'sudo')"
chk "normalize_argv0 strips windows path and exe suffix" cargo "$(normalize_argv0 'C:\tools\cargo.exe fmt --all')"
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
