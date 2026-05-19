#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"
# Spec §4.C: record base HEAD so C catches crate changes COMMITTED mid-turn
# (git status alone goes clean after a commit). --verify -q stays SILENT and
# exits non-zero on an unborn branch (zero commits): plain `rev-parse HEAD`
# would print "HEAD" to stdout there, making turn_base non-empty so C runs a
# vacuous HEAD..HEAD diff. Empty turn_base => C skips the diff arm and
# degrades to git-status + B-union (the intended no-commits behavior).
tb="$(git -C "$cwd" rev-parse --verify -q HEAD 2>/dev/null || true)"
# Also snapshot the patch-id of every PRE-TURN branch commit (commits ahead of
# upstream at A0). `git patch-id --stable` hashes the diff content alone, so a
# later `git rebase --onto` produces commits with the SAME patch-ids on the new
# line. effective_turn_base walks the rebased branch and matches against this
# stored set to recover the rewritten counterpart of turn_base — keeping the
# committed-this-turn diff arm scoped to THIS turn instead of widening to the
# whole branch divergence (Codex PR #726 review #3269664222). Empty array =>
# no pre-turn commits (fresh branch) or no upstream ref => safe degradation
# (effective_turn_base falls back to upstream merge-base).
pids_json="[]"
if [ -n "$tb" ] && have_jq; then
  mb=""
  for ref in origin/main main '@{upstream}'; do
    git -C "$cwd" rev-parse --verify -q "$ref" >/dev/null 2>&1 || continue
    candidate="$(git -C "$cwd" merge-base HEAD "$ref" 2>/dev/null)"
    [ -n "$candidate" ] && { mb="$candidate"; break; }
  done
  if [ -n "$mb" ] && [ "$mb" != "$tb" ]; then
    pids_json="$(git -C "$cwd" rev-list --reverse "${mb}..HEAD" 2>/dev/null \
                  | while IFS= read -r c; do
                      [ -n "$c" ] || continue
                      git -C "$cwd" show "$c" 2>/dev/null \
                        | git patch-id --stable 2>/dev/null \
                        | awk 'NF>0{print $1; exit}'
                    done \
                  | jq -Rsc 'split("\n") | map(rtrimstr("\r")) | map(select(length>0)) | .[0:256]' 2>/dev/null)"
    [ -z "$pids_json" ] && pids_json="[]"
  fi
fi
if have_jq; then
  state="$(jq -nc \
            --arg s "${sid:-unknown}" \
            --arg t "$(date -u +%FT%TZ)" \
            --arg tb "$tb" \
            --argjson pids "$pids_json" \
            '{session:$s,started_at:$t,impl_files_edited:[],gate_green:[],turn_base:$tb,turn_base_patch_ids:$pids}' 2>/dev/null)"
fi
if [ -z "${state:-}" ]; then
  state="$(printf '{"session":"%s","started_at":"%s","impl_files_edited":[],"gate_green":[],"turn_base":"%s","turn_base_patch_ids":[]}' "${sid:-unknown}" "$(date -u +%FT%TZ)" "$tb")"
fi
save_state "$p" "$state"
allow
