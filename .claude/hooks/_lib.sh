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
load_state() { # $1=path -> always {impl_files_edited:[...],gate_green:[...],turn_base:"..",turn_base_patch_ids:[...]}
  local d='{"impl_files_edited":[],"gate_green":[],"turn_base":"","turn_base_patch_ids":[]}'
  if [ -f "$1" ] && have_jq && jq -e . "$1" >/dev/null 2>&1; then
    jq -c '{impl_files_edited:(if (.impl_files_edited|type)=="array" then .impl_files_edited else [] end),gate_green:(if (.gate_green|type)=="array" then .gate_green else [] end),turn_base:(if (.turn_base|type)=="string" then .turn_base else "" end),turn_base_patch_ids:(if (.turn_base_patch_ids|type)=="array" then .turn_base_patch_ids else [] end)}' "$1" 2>/dev/null || printf '%s' "$d"
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
# flags crates this turn never touched.
#
# Recovery order on history rewrite (turn_base not an ancestor of HEAD):
#   1. If A0 stored patch-ids of pre-turn branch commits, walk the rebased
#      branch (`upstream-mb..HEAD`, oldest→newest) and find the LAST commit
#      whose patch-id is in that set. `git patch-id --stable` is invariant
#      across rebase, so that commit is the rewritten counterpart of the
#      original turn_base — anything above it is THIS turn. (Codex PR #726
#      review #3269664222: without this, branches with pre-turn commits get
#      those older crates re-flagged after a rebase.)
#   2. Else fall back to upstream merge-base (`origin/main` / `main` /
#      `@{upstream}`) — that point is AFTER any squash-merge so the merged
#      work drops out, but pre-turn branch commits would over-report.
#   3. If no upstream ref resolves, "" — callers skip the diff arm
#      (documented safe degradation; B-union + working-tree ground truth
#      still enforce the green gate).
#
# When the stored base IS still an ancestor (the common, no-rewrite case) it
# is returned unchanged — exact prior semantics, zero behavior change.
# Known narrow residual: `main` MERGED (not rebased) into the branch mid-turn
# keeps turn_base an ancestor, so that rarer workflow is out of scope here.
# $1=cwd  $2=stored turn_base
# stdin   one patch-id per line (turn_base_patch_ids; empty stream = none)
# stdout  effective base SHA (may be "")
effective_turn_base() {
  local cwd="$1" tb="$2" ref candidate mb pid c stored found
  local -a pids=()
  # jq -r emits CRLF on git-bash (Windows), so read -r leaves a trailing \r
  # in pid — the comparison below would silently miss every match. Strip it
  # defensively (same pattern as stop-gate.sh's _consider path handling).
  while IFS= read -r pid; do pid="${pid%$'\r'}"; [ -n "$pid" ] && pids+=("$pid"); done
  [ -n "$tb" ] || { printf ''; return 0; }
  if git -C "$cwd" merge-base --is-ancestor "$tb" HEAD 2>/dev/null; then
    printf '%s' "$tb"; return 0          # no rewrite — prior semantics intact
  fi
  mb=""
  for ref in origin/main main '@{upstream}'; do
    git -C "$cwd" rev-parse --verify -q "$ref" >/dev/null 2>&1 || continue
    candidate="$(git -C "$cwd" merge-base HEAD "$ref" 2>/dev/null)"
    [ -n "$candidate" ] && { mb="$candidate"; break; }
  done
  [ -n "$mb" ] || { printf ''; return 0; } # no upstream ref — skip diff arm
  if (( ${#pids[@]} > 0 )); then
    found=""
    while IFS= read -r c; do
      c="${c%$'\r'}"; [ -n "$c" ] || continue
      pid="$(git -C "$cwd" show "$c" 2>/dev/null \
              | git patch-id --stable 2>/dev/null \
              | awk 'NF>0{print $1; exit}')"
      pid="${pid%$'\r'}"
      [ -n "$pid" ] || continue
      for stored in "${pids[@]}"; do
        [ "$pid" = "$stored" ] && { found="$c"; break; }
      done
    done < <(git -C "$cwd" rev-list --reverse "${mb}..HEAD" 2>/dev/null | head -n 512)
    [ -n "$found" ] && { printf '%s' "$found"; return 0; }
  fi
  printf '%s' "$mb"                      # fallback: upstream merge-base
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
