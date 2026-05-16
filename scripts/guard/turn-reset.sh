#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"
# Spec §4.C: record base HEAD so C can catch crate changes COMMITTED mid-turn
# (git status alone goes clean after a commit). --verify -q stays SILENT and
# exits non-zero on an unborn branch (zero commits): plain `rev-parse HEAD`
# would print the literal "HEAD" to stdout there, making turn_base non-empty so
# C runs a vacuous HEAD..HEAD diff. Empty turn_base => C skips the diff arm and
# degrades to git-status + B-union (the intended no-commits behavior).
tb="$(git -C "$cwd" rev-parse --verify -q HEAD 2>/dev/null || true)"
save_state "$p" "$(printf '{"session":"%s","started_at":"%s","impl_files_edited":[],"gate_green":[],"turn_base":"%s"}' "${sid:-unknown}" "$(date -u +%FT%TZ)" "$tb")"
allow
