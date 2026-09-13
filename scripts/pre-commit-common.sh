#!/usr/bin/env bash
# Shared, fail-closed decoding for the dedicated pre-commit plan protocol.

load_pre_commit_plan() {
  local path
  local -a rust_paths=()
  for path in "$@"; do
    [[ "$path" == *.rs ]] && rust_paths+=("$path")
  done
  pre_commit_plan_json=""
  if [[ ${#rust_paths[@]} -eq 0 ]]; then
    return 0
  fi

  pre_commit_plan_json="$(cargo xtask pre-commit-plan -- "${rust_paths[@]}")"
  local LC_ALL=C
  if (( ${#pre_commit_plan_json} > 460800 )); then
    echo "lefthook: pre-commit plan exceeds 450 KiB" >&2
    return 1
  fi

  if ! jq -se '
    def identifier:
      type == "string" and test("\\A[A-Za-z0-9_][A-Za-z0-9_-]*\\z");
    def manifest:
      type == "string" and length > 0 and
      (contains("\\") | not) and
      (explode | all(. >= 32 and . != 127)) and
      (split("/") | all(. != "" and . != "." and . != "..")) and
      (split("/") | last == "Cargo.toml");
    length == 1 and (.[0] | . as $plan |
    (type == "object") and
    (keys == ["fixtures", "packages", "schema_version", "standalone_manifests"]) and
    (.schema_version == 1) and
    (.packages | type == "array") and
    (.packages == (.packages | sort | unique)) and
    all(.packages[]; identifier) and
    (.standalone_manifests | type == "array") and
    (.standalone_manifests == (.standalone_manifests | sort | unique)) and
    all(.standalone_manifests[]; manifest) and
    (.fixtures | type == "array") and
    ([.fixtures[].manifest_path] == ([.fixtures[].manifest_path] | sort | unique)) and
    ((.packages | length) + (.standalone_manifests | length) + (.fixtures | length) <= 256) and
    all(.fixtures[];
      . as $fixture |
      (keys == ["manifest_path", "owner", "test_target"]) and
      (.manifest_path | manifest) and
      (.owner | identifier) and
      (.test_target | identifier) and
      ($plan.packages | index($fixture.owner) != null) and
      ($plan.standalone_manifests | index($fixture.manifest_path) == null)
    ))
  ' >/dev/null <<< "$pre_commit_plan_json"; then
    echo "lefthook: nebula-xtask emitted an invalid pre-commit plan" >&2
    return 1
  fi
}
