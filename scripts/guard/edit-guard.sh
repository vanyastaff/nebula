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
if is_lib_rust "$file"; then
  st="$(printf '%s' "$st" | jq -c --arg f "$nf" '.impl_files_edited = (.impl_files_edited + [$f] | unique)')"
  save_state "$p" "$st"
fi
is_test=0
[[ "$nf" =~ /(tests|benches)/ ]] && is_test=1
printf '%s' "$added" | grep -qE '#\[(cfg\(test\)|test)\]' && is_test=1
jcount=$(printf '%s' "$added" | grep -cE '//[[:space:]]*guard-justified:' || true)
ecount=$(printf '%s' "$added" | grep -oE '#\[allow\(|(^|[^A-Za-z_])(todo!|unimplemented!|unreachable!)\(' | wc -l | tr -d ' ')

if is_lib_rust "$file" && [ "$is_test" -eq 0 ]; then
  if printf '%s' "$added" | grep -qE '\.[[:space:]]*unwrap[[:space:]]*(::<[^>]*>)?[[:space:]]*\(\)|\.[[:space:]]*expect[[:space:]]*\(|(^|[^A-Za-z_])panic![[:space:]]*\(|(Option|Result)[[:space:]]*::[[:space:]]*(unwrap|expect)[[:space:]]*\('; then
    [ "${jcount:-0}" -ge 1 ] || deny "New unwrap()/expect()/panic!() in library code is forbidden (AGENTS.md). Use a typed thiserror variant, or justify with '// guard-justified: <reason>'."
  fi
  [ "${ecount:-0}" -gt "${jcount:-0}" ] \
    && deny "allow/todo!/unimplemented!/unreachable! is a path-of-least-work escape — each needs its own '// guard-justified: <reason>' ($ecount escape(s), $jcount justification(s))."
  printf '%s' "$added" | grep -qE '//[[:space:]]*(TODO|FIXME|HACK|XXX)\b|TODO\([A-Z]+-?[0-9]|(^|[^A-Za-z])Phase[[:space:]][A-Z]\b' \
    && deny "TODO/FIXME/HACK/plan-id comments must not land in committed code."
  printf '%s' "$added" | grep -qE 'let[[:space:]]+_[[:space:]]*=[[:space:]]*([A-Za-z0-9_.]*[._])?(transition|send|write|commit|flush|lock|spawn)[A-Za-z0-9_]*[[:space:]]*\(' \
    && deny "let _ = <call> silently swallows a Result/must-use. Handle the error explicitly."
fi

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
      Write) o="$( [ -f "$file" ] && cat -- "$file" || printf '' )"; n="$added";;
    esac
    weak=0
    [ "$(assert_count "$o")" -gt "$(assert_count "$n")" ] && weak=1
    printf '%s' "$n" | grep -qE 'assert!\([[:space:]]*(true|1[[:space:]]*==[[:space:]]*1)[[:space:]]*\)|#\[ignore\]' && weak=1
    [ "$weak" -eq 1 ] && deny "Weakening a test (fewer asserts/#[test], assert!(true)/tautology/#[ignore]) while impl changed this turn is blocked. Fix the logic, not the test."
  fi
fi
allow
