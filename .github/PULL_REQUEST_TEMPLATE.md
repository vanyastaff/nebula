<!--
PR title must follow Conventional Commits (enforced by .github/workflows/pr-validation.yml):
  type(scope)?: description    — scope is optional
  types: feat | fix | docs | style | refactor | perf | test | chore | ci | build | revert
-->

## Summary

<!-- What does this PR do, and why? One paragraph. -->

## Linked issue

<!-- Link the Linear issue and any GitHub issues this PR closes or references. -->

- Closes NEB-
- Refs NEB-

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

## Test plan

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

## Docs checklist

<!--
Required for non-trivial design or execution-lifecycle changes.
See docs/PRODUCT_CANON.md §17 (Definition of Done).
Delete items that do not apply, or delete this section for pure bug fixes and mechanical refactors.
-->

- [ ] Reviewed `docs/PRODUCT_CANON.md` — no silent semantic drift, no new undocumented lifecycle
- [ ] Layer direction preserved (core → business → exec → api; no upward deps)
- [ ] If an L2 invariant changed: ADR added under `docs/adr/` with seam test in this PR
- [ ] `docs/MATURITY.md` row updated if crate maturity changed
- [ ] Crate `README.md` / `lib.rs //!` updated if public surface changed
- [ ] `docs/INTEGRATION_MODEL.md` updated if Resource / Credential / Action / Plugin / Schema surface changed
- [ ] `docs/STYLE.md` updated if a new idiom or antipattern surfaced
- [ ] `docs/GLOSSARY.md` updated if a new term was introduced
- [ ] Plan or spec that motivated this change archived or updated (link: <!-- path or URL -->)

## Safety / security impact

- [ ] No new `unwrap()` / `expect()` / `panic!()` in library code (tests and binaries excepted)
- [ ] No silent error suppression (`let _ = …` on `Result`, `.ok()`, `.unwrap_or_default()` on fallible IO)
- [ ] Execution / engine state transitions go through `transition_node()` (no direct `node_state.state = …`) — see #255
- [ ] Credentials / secrets stay encrypted, redacted, and zeroized across all added paths
- [ ] New `unsafe` blocks carry a `SAFETY:` comment with justification

## Notes for reviewers

<!-- Anything reviewers should focus on, known follow-ups, or out-of-scope items. Optional. -->
