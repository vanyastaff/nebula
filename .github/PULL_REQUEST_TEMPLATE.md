<!--
PR title must follow Conventional Commits (enforced by .github/workflows/pr-validation.yml):
  type(scope)?: description    — scope is optional
  types: feat | fix | docs | style | refactor | perf | test | chore | ci | build | revert
-->

## Summary

<!-- What does this PR do, and why? Keep it to a few sentences. -->

## Related issues

<!-- Link issues this PR closes or references. Leave blank if none. -->

- Closes #
- Refs #

## Type of change

<!-- Tick all that apply. -->

- [ ] `feat` — new capability
- [ ] `fix` — bug fix
- [ ] `docs` — documentation only
- [ ] `style` — formatting / non-functional style change
- [ ] `refactor` — internal restructuring, no behavior change
- [ ] `perf` — performance improvement
- [ ] `test` — tests only
- [ ] `chore` — tooling, maintenance, dependencies
- [ ] `ci` — CI configuration or workflow changes
- [ ] `build` — build system or packaging changes
- [ ] `revert` — reverts a previous change

## Affected crates / areas

<!-- e.g. nebula-engine, nebula-runtime, nebula-credential, docs/, .github/ -->

-

## Changes

<!-- Concrete list of what changed. Bullet points, not prose. -->

-

## Testing

<!-- How did you verify this change? Name the tests or scenarios, not just "ran CI". -->

-

### Local verification

- [ ] `cargo +nightly fmt --all` — formatted
- [ ] `cargo clippy --workspace -- -D warnings` — clean
- [ ] `cargo nextest run --workspace` — passes
- [ ] `cargo test --workspace --doc` — doctests pass (if public docs touched)
- [ ] `cargo deny check` — no new advisories (if `Cargo.toml` touched)

## Breaking changes

<!-- If yes: what breaks, who is affected, migration path. Otherwise write "None". -->

None

## Canon alignment

<!--
Required for non-trivial design or execution-lifecycle changes.
See docs/PRODUCT_CANON.md §17 (Definition of Done).
Delete this section for pure bug fixes, docs, or mechanical refactors.
-->

- [ ] Reviewed `docs/PRODUCT_CANON.md` — no silent semantic drift, no new undocumented lifecycle
- [ ] Layer direction preserved (core ← business ← exec ← api; no upward deps)
- [ ] If an L2 invariant moved: ADR added under `docs/adr/`
- [ ] `docs/MATURITY.md` row updated if crate maturity changed
- [ ] Crate `README.md` / `lib.rs //!` updated if public surface changed

## Safety checklist

- [ ] No new `unwrap()` / `expect()` / `panic!()` in library code (tests and binaries excepted)
- [ ] No silent error suppression (`let _ = …` on `Result`, `.ok()`, `.unwrap_or_default()` on fallible IO)
- [ ] Execution / engine state transitions go through `transition_node()` (no direct `node_state.state = …`) — see #255
- [ ] Credentials / secrets stay encrypted, redacted, and zeroized across all added paths
- [ ] New `unsafe` blocks carry a `SAFETY:` comment with justification

## Notes for reviewers

<!-- Anything reviewers should focus on, known follow-ups, or out-of-scope items. Optional. -->

