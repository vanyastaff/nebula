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

## North Star gate registry

The versioned post-selection registry is
[`gates/north-star-v1.toml`](./gates/north-star-v1.toml), and its evidence
contract is
[`schemas/gate-evidence-v1.schema.json`](./schemas/gate-evidence-v1.schema.json).
The canonical multi-run shape example is
[`schemas/gate-evidence-v1.example.json`](./schemas/gate-evidence-v1.example.json).
Validate all three, including every checked-in workflow/job binding, with:

```bash
cargo xtask north-star-gates validate
```

Success emits one compact deterministic JSON line. A validation failure emits
no partial stdout. The validator requires exactly the ordered `NS01`–`NS22`
set, one accountable lead, typed thresholds, backend applicability,
version-matched evidence paths, an explicit current state with a bounded
reason, activation checkpoints, and existing required-CI job IDs. The manifest
itself is the sole repository authority for each gate's North Star wording and
mutable policy.

This registry is gate policy applied after package selection. It does not
derive, add, or remove Cargo packages, and `cargo xtask ci-plan` remains the
sole package selector. A required-CI binding names the stable job that owns the
future or active proof; it does not turn `red`, `partial`, or `missing` into a
passing claim. It validates checked-in workflow/job existence, not branch
protection or status-check ownership.

The command validates and compiles the schema with the official Draft 2020-12
implementation, then validates the canonical bounded multi-run example.
Schema v1 is a versioned policy/evidence shape: its CI identities are recorded
strings rather than trusted attestations, and its threshold evaluation is not
recomputed from raw observations. Registry v1 therefore rejects `state =
"passed"`; all 22 entries must remain `red`, `partial`, or `missing`. Promotion
requires a later evidence schema with trusted CI attestation and recomputable
observations.

## Runtime-repair expected-RED verifier

The versioned expected-case policy is
[`gates/runtime-repair-red-v1.toml`](./gates/runtime-repair-red-v1.toml).
Validate its fixed profile/package/feature/binary identity, sorted canonical
Rust test paths, and kebab-case reason codes with:

```bash
cargo xtask runtime-repair-red validate-manifest
```

The active manifest names ten reached failures: first-party
C0/STARTKEY/cancellation reachability plus component-only C7 on their declared
backend sets. Setup-blocked cases are not represented as failures. The
path-unrestricted `.github/workflows/runtime-repair-red.yml` runs the serial,
retry-free `runtime-repair-red` nextest profile and passes its raw exit status
and JUnit path to:

```bash
cargo xtask runtime-repair-red verify \
  --nextest-exit-code 100 \
  --junit target/nextest/runtime-repair-red/runtime-repair-red.junit.xml
```

The bounded pull parser requires the exact manifest identities, one ordinary
test failure each, and one standalone `EXPECTED_RED:<reason-code>` marker from
captured stderr. Passes, skips, execution errors, timeouts, retries/reruns,
flaky outcomes, failure-body-only markers, missing/extra/duplicate identities,
malformed aggregate counts, an empty active set, or any exit other than
nextest's raw test-failure status 100 are rejected.

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
