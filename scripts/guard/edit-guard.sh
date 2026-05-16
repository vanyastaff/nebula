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
