# .claude/hooks/_lib.sh — shared helpers for Nebula guard hooks. Source, don't exec.
# Blocking convention: deny() => stderr + exit 2. allow() => exit 0.
guard_input=""
read_input() { guard_input="$(cat)"; }
have_jq() { command -v jq >/dev/null 2>&1; }
# jq -r prints the literal "null" for a missing/null field; callers treat that
# as a real value (e.g. git -C null, turn-null.json). Map a lone "null" to "".
# NOT `($1) // empty`: jq's // is falsy on boolean `false`, which would corrupt
# `.tool_response.success` reads in record.sh. This preserves false/true.
jqg() { local o; o="$(printf '%s' "$guard_input" | jq -r "$1" 2>/dev/null || true)"; [ "$o" = "null" ] && o=""; printf '%s' "$o"; }

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

# Effective base for the committed-this-turn diff arm (stop-gate.sh §4.C 3rd
# source, intent-gate.sh turn-diff scope). Stored turn_base = HEAD captured at
# A0 (turn-reset.sh). When the branch is later rebased / reparented (`git
# rebase --onto`, or its upstream squash-merged into main and the branch
# replayed onto it), that SHA lands on an ABANDONED history line: it is no
# longer an ancestor of HEAD, so a `turn_base..HEAD` diff spans the ENTIRE
# old→new base delta — the whole squash-merged upstream PR — and spuriously
# flags crates this turn never touched. Detect that (merge-base --is-ancestor
# fails) and repin to the branch's divergence floor from upstream (merge-base
# with the first of origin/main / main / @{upstream} that resolves); that point
# is AFTER any squash-merge so the merged work drops out, while commits made on
# THIS branch are still seen (the no-cheat catch is preserved). When the stored
# base IS still an ancestor (the common, no-rewrite case) it is returned
# unchanged — exact prior semantics, zero behavior change. If no upstream ref
# resolves, "" is emitted: callers skip the diff arm (documented safe
# degradation — identical to the unborn-branch path; B-union + working-tree
# ground truth still enforce the green gate). Known narrow residual: a `main`
# MERGED into the branch mid-turn keeps turn_base an ancestor, so that rarer
# workflow is out of scope here (the reproduced bug is `rebase --onto`).
# $1=cwd  $2=stored turn_base  ->  effective base SHA (may be "")
effective_turn_base() {
  local cwd="$1" tb="$2" ref mb
  [ -n "$tb" ] || { printf ''; return 0; }
  if git -C "$cwd" merge-base --is-ancestor "$tb" HEAD 2>/dev/null; then
    printf '%s' "$tb"; return 0          # no rewrite — prior semantics intact
  fi
  for ref in origin/main main '@{upstream}'; do
    git -C "$cwd" rev-parse --verify -q "$ref" >/dev/null 2>&1 || continue
    mb="$(git -C "$cwd" merge-base HEAD "$ref" 2>/dev/null)" || continue
    [ -n "$mb" ] && { printf '%s' "$mb"; return 0; }
  done
  printf ''                              # no upstream ref — skip diff arm
}

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
