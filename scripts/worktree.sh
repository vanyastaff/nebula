#!/usr/bin/env bash
set -euo pipefail

readonly WORKTREE_DIR=".worktrees"
readonly DEFAULT_BASE="${NEBULA_WORKTREE_BASE:-origin/main}"
readonly VALID_TYPES="build chore ci docs feat fix perf refactor revert style test"

die() {
  echo "nebula-worktree: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
Usage:
  bash scripts/worktree.sh new <name> <type> <scope> [base]
  bash scripts/worktree.sh list
  bash scripts/worktree.sh remove <name>
  bash scripts/worktree.sh finish <name> [branch] [main-branch]
  bash scripts/worktree.sh commit <type> <scope> <message...>

Examples:
  bash scripts/worktree.sh new retry-pipeline fix resilience
  bash scripts/worktree.sh finish retry-pipeline
  bash scripts/worktree.sh commit fix resilience "harden retry semantics"
USAGE
}

repo_root() {
  git rev-parse --show-toplevel 2>/dev/null || die "not inside a git repository"
}

slugify() {
  local raw="$1"
  printf '%s' "$raw" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9._-]+/-/g; s/^-+//; s/-+$//; s/-+/-/g'
}

validate_type() {
  local type="$1"
  for valid in $VALID_TYPES; do
    if [[ "$type" == "$valid" ]]; then
      return 0
    fi
  done
  die "invalid type '$type'; expected one of: $VALID_TYPES"
}

ensure_clean_name() {
  local label="$1"
  local value="$2"
  [[ -n "$value" ]] || die "$label cannot be empty"
  [[ "$value" != "." && "$value" != ".." ]] || die "$label cannot be '$value'"
  [[ "$value" != *"/"* && "$value" != *"\\"* ]] || die "$label cannot contain path separators"
}

fetch_base_if_remote() {
  local base="$1"
  if [[ "${NEBULA_WORKTREE_NO_FETCH:-0}" == "1" ]]; then
    return 0
  fi

  if [[ "$base" == */* ]]; then
    local remote="${base%%/*}"
    local branch="${base#*/}"
    if git remote get-url "$remote" >/dev/null 2>&1; then
      git fetch "$remote" "$branch"
    fi
  fi
}

new_worktree() {
  local name_raw="${1:-}"
  local type_raw="${2:-}"
  local scope_raw="${3:-}"
  local base="${4:-$DEFAULT_BASE}"

  [[ -n "$name_raw" && -n "$type_raw" && -n "$scope_raw" ]] || {
    usage
    exit 2
  }

  local name type scope branch path
  name="$(slugify "$name_raw")"
  type="$(slugify "$type_raw")"
  scope="$(slugify "$scope_raw")"

  ensure_clean_name "name" "$name"
  ensure_clean_name "scope" "$scope"
  validate_type "$type"

  branch="${type}/${scope}-${name}"
  path="${WORKTREE_DIR}/${name}"

  mkdir -p "$WORKTREE_DIR"

  [[ ! -e "$path" ]] || die "worktree path already exists: $path"
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    die "local branch already exists: $branch"
  fi

  fetch_base_if_remote "$base"
  git rev-parse --verify --quiet "${base}^{commit}" >/dev/null \
    || die "base ref not found: $base"

  git worktree add -b "$branch" "$path" "$base"

  echo "Created worktree:"
  echo "  path:   $path"
  echo "  branch: $branch"
  echo "  base:   $base"
}

list_worktrees() {
  git worktree list
}

primary_worktree_root() {
  git worktree list --porcelain \
    | sed -n '1s/^worktree //p' \
    | sed 's#\\#/#g'
}

find_worktree_path() {
  local suffix="$1"

  git worktree list --porcelain \
    | sed -n 's/^worktree //p' \
    | sed 's#\\#/#g' \
    | awk -v suffix="/$suffix" 'substr($0, length($0) - length(suffix) + 1) == suffix { print; exit }'
}

ensure_clean_checkout() {
  local path="$1"
  local label="$2"
  local status

  status="$(git -C "$path" status --porcelain --untracked-files=all)"
  if [[ -n "$status" ]]; then
    echo "$status" >&2
    die "$label has uncommitted or untracked files: $path"
  fi
}

main_remote() {
  local main_branch="$1"
  local remote

  remote="$(git config --get "branch.${main_branch}.remote" || true)"
  if [[ -n "$remote" ]]; then
    printf '%s\n' "$remote"
  elif git remote get-url origin >/dev/null 2>&1; then
    printf '%s\n' origin
  fi
}

validate_commit_message() {
  local message="$1"

  if command -v convco >/dev/null 2>&1; then
    printf '%s\n' "$message" | convco check --from-stdin
  elif command -v convco.exe >/dev/null 2>&1; then
    printf '%s\n' "$message" | convco.exe check --from-stdin
  elif command -v pwsh.exe >/dev/null 2>&1; then
    printf '%s\n' "$message" | pwsh.exe -NoProfile -Command '$input | convco check --from-stdin'
  elif command -v powershell.exe >/dev/null 2>&1; then
    printf '%s\n' "$message" | powershell.exe -NoProfile -Command '$input | convco check --from-stdin'
  else
    die "convco is required to validate commits"
  fi
}

remove_worktree() {
  local name_raw="${1:-}"
  [[ -n "$name_raw" ]] || {
    usage
    exit 2
  }

  local name path
  name="$(slugify "$name_raw")"
  ensure_clean_name "name" "$name"
  path="${WORKTREE_DIR}/${name}"

  git worktree remove "$path"
  git worktree prune
}

finish_worktree() {
  local name_raw="${1:-}"
  local branch="${2:-}"
  local main_branch="${3:-main}"

  [[ -n "$name_raw" ]] || {
    usage
    exit 2
  }

  local name primary path current_branch remote
  name="$(slugify "$name_raw")"
  ensure_clean_name "name" "$name"

  primary="$(primary_worktree_root)"
  [[ -n "$primary" ]] || die "could not determine primary worktree"

  path="$(find_worktree_path "${WORKTREE_DIR}/${name}")"
  [[ -d "$path" ]] || die "worktree path does not exist: $path"

  ensure_clean_checkout "$path" "target worktree"

  if [[ -z "$branch" ]]; then
    branch="$(git -C "$path" branch --show-current)"
  fi
  [[ -n "$branch" ]] || die "could not determine target branch for $path"

  cd "$primary"

  ensure_clean_checkout "$primary" "primary worktree"

  current_branch="$(git -C "$primary" branch --show-current)"
  if [[ "$current_branch" != "$main_branch" ]]; then
    git -C "$primary" switch "$main_branch"
  fi

  remote="$(main_remote "$main_branch")"
  if [[ -n "$remote" ]]; then
    git -C "$primary" fetch "$remote" "$main_branch"
    git -C "$primary" pull --ff-only "$remote" "$main_branch"
  else
    echo "nebula-worktree: no remote configured for $main_branch; skipping fetch/pull" >&2
  fi

  git -C "$primary" worktree remove "$path"
  git -C "$primary" worktree prune

  if git -C "$primary" show-ref --verify --quiet "refs/heads/$branch"; then
    git -C "$primary" branch -d "$branch"
  fi

  echo "Finished worktree:"
  echo "  removed: $path"
  echo "  branch:  $branch"
  echo "  main:    $main_branch"
  [[ -z "$remote" ]] || echo "  remote:  $remote"
}

commit_staged() {
  [[ $# -ge 3 ]] || {
    usage
    exit 2
  }

  local type_raw="$1"
  local scope_raw="$2"
  shift 2

  local description="$*"
  local type scope message
  type="$(slugify "$type_raw")"
  scope="$(slugify "$scope_raw")"

  ensure_clean_name "scope" "$scope"
  validate_type "$type"

  if git diff --cached --quiet --exit-code; then
    die "no staged changes to commit"
  fi

  message="${type}(${scope}): ${description}"
  validate_commit_message "$message"
  git commit -m "$message"
}

main() {
  local root
  root="$(repo_root)"
  cd "$root"

  local command="${1:-}"
  shift || true

  case "$command" in
    new)
      new_worktree "$@"
      ;;
    list)
      list_worktrees
      ;;
    remove)
      remove_worktree "$@"
      ;;
    finish)
      finish_worktree "$@"
      ;;
    commit)
      commit_staged "$@"
      ;;
    -h|--help|help|"")
      usage
      ;;
    *)
      usage
      die "unknown command: $command"
      ;;
  esac
}

main "$@"
