#!/usr/bin/env bash
# PreToolUse hook: block Bash commands that read, print, or exfiltrate obvious
# secret files and environment variables. Complements guard-bash.sh (which
# blocks destructive operations). This one is focused on credential leaks.
#
# Contract: receives JSON on stdin with tool_input.command. Exit 0 = allow;
# exit 2 = block (Claude Code's deny signal). stderr is shown to the user.

set -euo pipefail

INPUT=$(cat)

CMD=""
# Prefer jq; fall back to python3; last resort is a brittle sed.
if command -v jq >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | jq -r '.tool_input.command // ""' 2>/dev/null || true)
fi

if [[ -z "$CMD" ]] && command -v python3 >/dev/null 2>&1; then
  CMD=$(printf '%s' "$INPUT" | python3 -c '
import json, sys
try:
    payload = json.load(sys.stdin)
except Exception:
    print("")
    raise SystemExit(0)
print(payload.get("tool_input", {}).get("command", "") or "")
' 2>/dev/null || true)
fi

if [[ -z "$CMD" ]]; then
  CMD=$(printf '%s' "$INPUT" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
fi

if [[ -z "$CMD" ]]; then
  exit 0
fi

# --- 1. Block reads of common secret-file extensions via pager/printer tools.
#        Narrow to whole-token matches so "catalog.md" is not a false positive.
if printf '%s' "$CMD" | grep -qE '\b(cat|less|more|head|tail|bat|xxd)\b[^|;&]*\.(env|pem|key|p12|pfx)(\s|$|\|)'; then
  echo "Blocked by guard-secrets.sh: reading a likely-secret file (.env/.pem/.key/.p12/.pfx)." >&2
  echo "If legitimate (for example, a test fixture), narrow the command or add an allow entry in .claude/settings.local.json." >&2
  exit 2
fi

# --- 2. Block reads of well-known secret paths.
if printf '%s' "$CMD" | grep -qE '\b(cat|less|more|head|tail|bat)\b[^|;&]*/(id_rsa|id_ed25519|id_ecdsa|credentials\.json|service-account\.json|\.npmrc|\.pypirc|\.netrc)(\s|$|\|)'; then
  echo "Blocked by guard-secrets.sh: reading a well-known secret file." >&2
  exit 2
fi

# --- 3. Block unfiltered env / printenv dumps (easy to leak tokens into tool output).
#        Allow `printenv PATH` / `env | grep ...` / `env SOMEVAR=...`.
if printf '%s' "$CMD" | grep -qE '(^|[[:space:]]|;|&&|\|\|)\s*(printenv|env)([[:space:]]*$|[[:space:]]*\|\s*$)'; then
  echo "Blocked by guard-secrets.sh: 'env' / 'printenv' without a filter can leak secrets to tool output." >&2
  echo "Narrow the command: 'printenv PATH', 'env | grep RUST_LOG', or similar." >&2
  exit 2
fi

# --- 4. Block echoing / printing env vars that look like secrets.
if printf '%s' "$CMD" | grep -qE '\b(echo|printf)[[:space:]]+("[^"]*)?\$\{?[A-Z_]*(TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|API_KEY)[A-Z_]*\}?'; then
  echo "Blocked by guard-secrets.sh: echoing an environment variable that looks like a secret." >&2
  echo "If this is intentional (e.g., a dev script printing a non-sensitive token), inline-document why and allow via settings.local.json." >&2
  exit 2
fi

# --- 5. Block curl/wget exfiltration with a secret-looking var in the URL or body.
if printf '%s' "$CMD" | grep -qE '\b(curl|wget)\b[^|;&]*(\$\{?[A-Z_]*(TOKEN|SECRET|KEY|PASSWORD)[A-Z_]*\}?|Authorization:[[:space:]]*Bearer[[:space:]]+\$)'; then
  echo "Blocked by guard-secrets.sh: curl/wget with a secret-looking variable interpolated." >&2
  echo "If legitimate (automation pushing a token to an internal service), run it outside the agent." >&2
  exit 2
fi

exit 0
