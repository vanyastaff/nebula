# Quality gates

How Nebula mechanically enforces code quality in the toolchain. Normative
product rules live in [`docs/PRODUCT_CANON.md`](./PRODUCT_CANON.md); the
agent-facing discipline contract is the **Enforced Discipline** table in
[`AGENTS.md`](../AGENTS.md). This file is only about *how* the knobs work and
*why* some Clippy lints are intentionally `allow`. The knobs themselves live in
`Cargo.toml` (`[workspace.lints]`), `clippy.toml`, `deny.toml`, and
`.claude/hooks/` — extend them in place; do not duplicate them elsewhere.

Layer order (strongest first): rustc / Clippy (`-D warnings` in CI) →
`cargo deny` (`deny.toml`) → committed guard hooks (`.claude/hooks/`, the D10
no-cheat core + the ADR-0083 Layer-2 budget) → human review.

## Metadata-driven package selection

`cargo xtask ci-plan` is the sole package selector for the test-matrix workflow
and the pre-push crate gate. It asks Cargo for locked format-v1 metadata with
all features and without a platform filter, then computes the transitive
reverse closure of changed workspace packages across normal, development,
build, optional, and target-specific dependency edges. A missing or stale
lockfile is an error and the planner never updates it. Package names and
ownership therefore come from manifests rather than directory-name conventions.

The commands are:

```bash
cargo xtask ci-plan full
cargo xtask ci-plan diff --base <sha> --head <sha> --comparison merge-base
cargo xtask ci-plan diff --base <sha> --head <sha> --comparison direct
```

`merge-base` uses Git three-dot semantics and is the pull-request/local mode;
`direct` compares the two tips and is the merge-queue mode. Push and manual CI
runs request `full`. A local pre-push without a discoverable upstream also
requests `full`; it is not downgraded to a smoke list.

A successful `ci-plan` writes exactly one compact, deterministic JSON document
(plus its trailing newline). `--help` and `--version` use Clap's successful
human-readable stdout contract; invalid CLI usage uses stderr and Clap's exit
code. Planner diagnostics go to stderr and never emit partial JSON. Schema
version 1 is:

```json
{"schema_version":1,"scope":"diff","reason":"workspace-packages-changed","count":1,"include":[{"package":"nebula-core","test_features":[]}]}
```

Entries use exact Cargo package names, are sorted and deduplicated, and are
limited to 256. The serialized document is conservatively capped at 450 KiB.
GitHub accounts job outputs as UTF-16, so this leaves headroom below its 1 MiB
per-job boundary even for an ASCII-heavy plan. CI and the pre-push hook validate
the schema, count, ordering, and entry types before invoking Cargo. Only
`{ "include": ... }` is exported as the GitHub matrix; `count` is a separate
job output so an empty matrix skips the test job without weakening the stable
aggregator check.

Per-package test features are declared only here:

```toml
[package.metadata.nebula.ci]
test-features = ["feature-name"]
```

The `metadata.nebula.ci` table is a strict policy object: scalar values and
unknown keys are errors. The selector also rejects feature names absent from
that package's declared `[features]`; unrelated package metadata remains
unrestricted. It never copies Cargo's resolved feature set. These features
apply only to `nextest`; `cargo check --all-features
--all-targets`, rustdoc with `-D warnings`, the six no-default checks
(`resilience`, `log`, `expression`, `credential`, `resource`, `storage`), and
the `DATABASE_URL`-gated Postgres suite retain their independent contracts.
The six names are an explicit no-default-feature **gate-policy** list, not a
second package selector: the metadata plan decides membership first, and this
policy only adds a check when one of those selected packages promises a usable
minimal feature surface.

Package ownership uses the deepest workspace manifest directory, so a nested
derive package wins over its parent. Git copy detection includes unchanged
sources, and rename/copy records examine both old and new paths. Backslashes in
raw Git paths are treated as uncertain rather than rewritten. Deletions,
unresolved old owners, unknown or ambiguous paths,
selector/bootstrap changes, and paths below `crates/*/fuzz` select the full
workspace. Fuzz packages are excluded from the root workspace and are **not**
claimed as tested by this matrix; a fuzz-path change triggers full root-workspace
coverage as the conservative fallback. Explicit documentation/editor/assets
paths that are not owned by a workspace package may produce a valid
zero-package plan. Ownership is resolved first: package-local README, docs, and
assets changes select that package and its reverse dependents because those
files may be compile-time inputs. A deletion or any other full-scope condition
takes precedence over the docs-only classification.

Bootstrap paths are the root `Cargo.toml`, `Cargo.lock`, `.cargo/**`,
`rust-toolchain*`, `Taskfile.yml`, `deny.toml`, the selector implementation
under `tools/xtask/**`, `.github/workflows/test-matrix.yml`, and
`scripts/pre-push-crate-diff.sh`. Changing how selection works must therefore
prove the complete workspace rather than trusting the changed selector.

## Versioned North Star gate policy

The North Star measurement and exit-gate contract is executable repository
policy:

- registry:
  [`tools/xtask/gates/north-star-v1.toml`](../tools/xtask/gates/north-star-v1.toml);
- evidence schema:
  [`tools/xtask/schemas/gate-evidence-v1.schema.json`](../tools/xtask/schemas/gate-evidence-v1.schema.json);
- canonical multi-run evidence:
  [`tools/xtask/schemas/gate-evidence-v1.example.json`](../tools/xtask/schemas/gate-evidence-v1.example.json);
- validator:

  ```bash
  cargo xtask north-star-gates validate
  ```

The registry is strictly **post-selection**. Its workflow/job bindings say
where a selected gate's proof is or will be required; they never derive a
package list, add a package to a plan, or remove one. `cargo xtask ci-plan`
remains the sole package selector.

Registry v1 contains exactly the ordered `NS01`–`NS22` set. Every entry has one
accountable lead, a typed threshold, an explicit backend set or a bounded
non-applicability reason, versioned evidence location, stable checked-in
required-CI binding, activation checkpoint, and an explicit current state with
a bounded reason.
`red`, `partial`, and `missing` are not passing states. A job binding alone is
not evidence and does not imply that branch protection currently requires that
job.

The v1 command validates the schema with the official Draft 2020-12
meta-schema, compiles it, and validates the complete checked-in multi-run
example. The required pull-request path runs the command in
`.github/workflows/test-matrix.yml` before metadata-driven package selection;
the validator never contributes a package name or changes the plan.

Registry v1 rejects `state = "passed"` before treating any declared artifact as
promotion evidence. Evidence schema v1 is a versioned policy/evidence shape,
not a trustworthy promotion protocol: CI identities are recorded strings, not
trusted attestations, and threshold evaluations are recorded values that the
validator cannot recompute from the referenced raw observations. A later
schema must add both properties before a gate can be promoted.

The schema's bounded, nonempty `runs` array records each source revision,
environment/topology with one database backend or explicit non-applicability,
closed sampling policies (`fail-run` and `no-retry`), cache rules, interval
boundaries, configured fairness, sanitized raw-observation references,
query-plan applicability, and CI identity. Versioned inputs, aggregation
identity, denominator, threshold evaluation, and result are common at the
root. Objects and collections are closed and bounded; secrets and raw business
payloads are prohibited. The canonical example is a failed aggregate because
its denominator includes exclusions and skips.

The versioned focused-working-day definition and its mutable duration live only
in the gate registry. Gate NS06 takes its fairness target and allowed deviation from the versioned
workload/configuration rather than assuming one universal percentage. All
other mutable threshold values remain authoritative in the registry instead of
being copied into this guide.

## Runtime-repair expected-RED evidence

The non-deployment runtime-repair harness has a dedicated serial, retry-free
nextest profile and required workflow:

- policy: `tools/xtask/gates/runtime-repair-red-v1.toml`;
- profile: `.config/nextest.toml` → `profile.runtime-repair-red`;
- workflow: `.github/workflows/runtime-repair-red.yml`;
- verifier:

  ```bash
  cargo xtask runtime-repair-red validate-manifest
  cargo xtask runtime-repair-red verify \
    --nextest-exit-code 100 \
    --junit target/nextest/runtime-repair-red/runtime-repair-red.junit.xml
  ```

The active v1 manifest names the reached behavioral failures that remain
unrepaired. It currently names two: first-party C0 park/restart/resume on file
SQLite and required live PostgreSQL, which now reach the durable drive and park
but do not re-arm the parked wait after a restart. Setup-blocked oracles are not
fabricated as RED cases.

A repaired scenario **stays in the suite** and loses its manifest entry in the
same change that repairs it. It then reconciles as ordinary conformance
coverage, so a regression reads as a plain failure rather than as a gate that
mysteriously went green. Deleting the scenario instead would destroy the very
evidence the harness exists to hold. Promoted this way so far: C7
same-processor ABA and StartKey acceptance on all three backends, and C1
cancellation authority on file SQLite and live PostgreSQL.

The workflow runs on every pull request and main-branch push so a repair outside
the harness cannot bypass reconciliation. The verifier accepts only raw nextest
exit 100 and reconciles on *failures*, not on the total case count: exactly the
manifested identities may fail, each with one failing JUnit case and exactly one
standalone `EXPECTED_RED:<reason-code>` marker captured from stderr; every other
selected test must have passed. No skips, errors, timeouts, retries, reruns,
flaky outcomes, or unmanifested failures are tolerated. Ignored,
`should_panic`, sentinel, compile-fail, synthetic JUnit, and failure-body-only
markers are not production RED evidence.

## Mechanized junior markers

What is enforced today, observable in the repo (the `Cargo.toml`
`[workspace.lints.clippy]` block carries the citations referenced here):

| Marker | Mechanization (current) |
|--------|-------------------------|
| Pedantic / nursery / cargo lint families | `[workspace.lints.clippy]` at **warn**; CI `cargo clippy … -- -D warnings` turns every warn into a hard failure (`.github/workflows/ci.yml`). |
| `std::mem::forget` misuse | `mem_forget = "deny"` (`Cargo.toml`). |
| `Rc<Mutex>` / non-`Send`/`Sync` `Arc` footguns | `rc_mutex` / `arc_with_non_send_sync` = **warn** — cites [C-SEND-SYNC](https://rust-lang.github.io/api-guidelines/interoperability.html#c-send-sync). |
| `dbg!` shipped in non-test code | `dbg_macro = "warn"` (tests exempt via `clippy.toml`). |
| `unwrap()` / `expect()` / `panic!()` in library code | **Enforced**, no escape, by `.claude/hooks/edit-guard.sh` (AGENTS.md "Enforced Discipline" / D10) — not via `clippy::unwrap_used`, which would need a workspace-wide burn-down first. |
| `unsafe` without local reasoning | `undocumented_unsafe_blocks` + `clippy.toml` (`accept-comment-above-statement/attributes`); convention `// SAFETY:` above the block. |
| Function bloat / cognitive complexity / nesting | `clippy.toml` thresholds (`too-many-lines = 100`, `cognitive-complexity = 25`, `excessive-nesting = 5`) are **inert workspace-wide** (the lints are `allow` — see next section) but enforced **diff-scoped on new code** by `.claude/hooks/intent-gate.sh` (ADR-0083). |
| Duplicate utility / oversized / file-sprawling turns | `.claude/hooks/intent-gate.sh` net-LoC / new-file / large-blob / duplicate-public-symbol budgets (ADR-0083), with a `// budget-justified:` escape. |

Still review-only (honest list — no full mechanization yet): `Box<dyn Error>`
at public API boundaries; duplicate *stable type* names across crates (the
duplicate-*symbol* heuristic in `intent-gate.sh` is a partial, conservative
proxy, not a full check); ADR front-matter ↔ code traceability.

## Intentionally allowed Clippy

Several lints in `Cargo.toml` `[workspace.lints.clippy]` are set to `allow`
**not** because Nebula rejects what they encourage, but because `warn` would
force large or noisy churn across existing code (style taste, macro sites, API
shape, a generic-heavy workspace, or legacy patterns). This is a universal
policy, not a per-feature exception.

**Rule for agents and reviewers:** on **new** and **heavily-touched** code,
follow the *spirit* of those lints where it improves clarity, safety, or
alignment with the Rust API Guidelines and Reference — even when CI is green.
CI passing does not mean "ignore the lint's intent" on new code.

That spirit is no longer review-only: it is mechanized **diff-scoped** by the
ADR-0083 Layer-2 gate (`.claude/hooks/intent-gate.sh`). The inert `clippy.toml`
complexity thresholds, plus net-LoC / new-file / duplicate-symbol budgets, are
enforced on the turn's changed code while legacy stays grandfathered.

**Mechanization path for any such `allow`:** workspace `warn` only after an
explicit burn-down, or `warn` in crates that opt in, or a targeted
`dylint`/lint crate on changed paths. The sequenced legacy structural-debt
burn-down workstream (ADR-0083 § Follow-up) reconciles the
`cognitive_complexity` / `too_many_lines` allowance crate-by-crate; until then
`intent-gate.sh` holds the line on new code.

## Diff-scoped structural budget (ADR-0083)

The `cognitive_complexity` / `too_many_lines` workspace `allow` stays — flipping
them on 36 crates is thousands of legacy warnings. `.claude/hooks/intent-gate.sh`
holds new code to a diff-scoped budget instead: the **large-blob proxy** is
derived from the `clippy.toml` `too-many-lines = 100` threshold; the **net-LoC
(400)**, **new-file (5)** and **duplicate-symbol** caps are the gate's own
independent budgets (not `clippy.toml` thresholds). All carry a
`// budget-justified:` escape. Legacy is grandfathered; the separate legacy
burn-down workstream reconciles the inert clippy thresholds crate-by-crate.
