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
