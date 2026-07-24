# Nebula xtask

`nebula-xtask` contains repository automation whose answer must stay aligned
with Cargo's workspace graph. It is a `publish = false` workspace package and
is deliberately outside Nebula's product dependency layers.

## CI plan commands

```bash
cargo xtask ci-plan full
cargo xtask ci-plan diff --base <sha> --head <sha> --comparison merge-base
cargo xtask ci-plan diff --base <sha> --head <sha> --comparison direct
```

`full` includes every `workspace_member`, including `nebula-xtask` itself.
Metadata is loaded with `--locked`; a missing or stale lockfile is an error and
is never rewritten by either the planner or its workspace Cargo alias. `diff`
reads `git diff --name-status -z -M -C --find-copies-harder`, resolves each
changed path to the deepest package manifest directory, and adds every
transitive reverse workspace dependent. `merge-base` uses `base...head`;
`direct` uses `base head`.

A successful `ci-plan` command emits only compact schema-v1 JSON on stdout:

```json
{"schema_version":1,"scope":"full","reason":"full-request","count":1,"include":[{"package":"example-package","test_features":[]}]}
```

Entries are sorted by exact Cargo package name. `--help` and `--version` follow
Clap's standard successful human-readable stdout contract; invalid usage uses
stderr and Clap's exit code. A planner failure is nonzero, writes a diagnostic
to stderr, and emits no partial stdout. Plans are capped at 256 entries and
450 KiB. The byte cap leaves headroom for GitHub's UTF-16 output accounting
beneath the 1 MiB per-job boundary.

## Package metadata

A package that needs additional features only while running its tests declares
them in its own manifest:

```toml
[package.metadata.nebula.ci]
test-features = ["rotation"]
```

The `metadata.nebula.ci` table is strict: a scalar policy or an unknown key is
an error, while metadata outside that table is unaffected. Every test feature
must exist in that package's `[features]`. The planner loads Cargo metadata with
all features so optional dependency edges participate in reverse closure, but
resolved features never become test features implicitly.

Package-name lists are forbidden for **selection**. A consumer may still carry
an explicit, independent gate-policy list after selection. Pre-push does this
for the minimal no-default-feature surfaces of `nebula-resilience`,
`nebula-log`, `nebula-expression`, `nebula-credential`, `nebula-resource`, and
`nebula-storage`. Those policy names only add a gate to an already-selected
package and never change matrix membership.

## Conservative fallbacks

Missing diff SHAs select the full workspace. Invalid nonempty revisions are an
error. Copy detection considers unchanged sources, and both sides of rename and
copy records participate in ownership. Deletions, unknown/ambiguous ownership,
unresolved old rename owners, raw paths containing backslashes, bootstrap
changes, and excluded `crates/*/fuzz` paths select the full root workspace. The
fuzz packages remain outside this workspace and are not claimed as covered.
Known documentation, editor, and asset-only changes outside package
ownership can produce an empty diff plan. Package-local README, docs, and asset
changes select their owner and reverse dependents because they may be
compile-time inputs.

See [`docs/QUALITY_GATES.md`](../../docs/QUALITY_GATES.md) for the workflow and
local-hook consumer contract.
