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
# substring tripwire. Guarantee is structural: B + A2 + C + CI.
