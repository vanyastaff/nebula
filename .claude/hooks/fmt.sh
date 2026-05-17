#!/usr/bin/env bash
# Format-only, single file, never organize-imports (split-edit safe), never blocks.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/_lib.sh"
read_input
case "$(jqg '.tool_name')" in Write|Edit|MultiEdit) :;; *) allow;; esac
f="$(jqg '.tool_input.file_path')"; f="${f%$'\r'}"  # jq -r CRLF on git-bash
case "$f" in
  *.rs)   rustfmt --edition 2024 "$f" >/dev/null 2>&1 || true;;
  *.toml) command -v taplo >/dev/null 2>&1 && taplo fmt "$f" >/dev/null 2>&1 || true;;
esac
allow
