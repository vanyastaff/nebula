#!/usr/bin/env bash
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
sid="$(jqg '.session_id')"; cwd="$(jqg '.cwd')"; [ -n "$cwd" ] || cwd="$PWD"
p="$(turn_state_path "$sid" "$cwd")"
save_state "$p" "$(printf '{"session":"%s","started_at":"%s","impl_files_edited":[],"gate_green":[]}' "${sid:-unknown}" "$(date -u +%FT%TZ)")"
allow
