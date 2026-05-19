# Roadmap

What's planned beyond v0.1. Ordered by value-per-cost.

## 1. Commands (tooling on top of the spec) — ✅ initial set shipped in v0.1

Goal: turn the passive skill into active tools that **find and fix** mistakes themselves, so a developer doesn't need to memorize all 26 categories.

### `/rust-audit [path]` — ✅ shipped

**Use case:** "Check my code."

**Input:** path to a file, directory, or crate (defaults to the current working directory).

**Behavior:**
1. Read `Cargo.toml` to pin versions.
2. Scan Rust sources against all 26 categories.
3. For each finding: category (`§B2`), file:line, code citation, why it's dangerous, concrete fix (not generic advice).
4. Group by tier (A/B/C) and severity.
5. End with a summary in the Post-flight checklist format from the spec.

**Why first:** maximum value, minimum friction. Existing code in → triaged report out. Activates every category in one pass.

See [`../commands/rust-audit.md`](../commands/rust-audit.md).

### `/rust-fix <error-message>` — ✅ shipped

**Use case:** "I have an error / panic / weird behavior."

**Input:** `rustc` / `cargo clippy` output, panic stack trace, or runtime-anomaly description.

**Behavior:**
1. Parse the message → map to a spec category (`E0277 → §A2`, `deadlock → §B9`, `OOM in graph → §B10`).
2. Explain the **root cause**, not the symptom.
3. Propose a fix that **doesn't violate other categories** (especially: not the reflexive `.clone()` from §C5).
4. State the preventive rule from the spec so the same mistake doesn't recur.

**Value:** removes the developer's need to navigate rustc docs and stale StackOverflow answers.

See [`../commands/rust-fix.md`](../commands/rust-fix.md).

### `/rust-plan <task>` — ✅ shipped

**Use case:** "I want to write X."

**Input:** task description in natural language.

**Behavior:**
1. Run the description through the spec's trigger table → identify activated categories.
2. Ask clarifying Pre-flight questions (crate versions, cancel-safety, async/sync context, lifetimes).
3. Output an implementation plan with explicit risk points and preconditions.
4. **Does not write code** — plan only. Code happens in a separate step with full context loaded.

**Value:** catches mistakes at the design stage, when rolling them back is still cheap.

See [`../commands/rust-plan.md`](../commands/rust-plan.md).

## 2. Category expansions

Categories with observed patterns but insufficient empirical backing or sharp BANNED/REQUIRED wording. They move into the main spec as data accumulates.

- **§B16 (draft). `serde` (de)serialization edge cases.** Field renames, untagged enums, `Option<T>` null vs missing keys, `#[serde(default)]` semantics. LLMs conflate "field absent" with "null" and vice versa.
- **§B17 (draft). FFI and `Drop` across the ABI boundary.** Passing `Box<T>` to C, leaks under panic unwinding through `extern "C"`, `catch_unwind`.
- **§B18 (draft). `#[no_std]` and `alloc`.** LLMs reflexively pull `std::*` paths into `no_std` crates, breaking embedded builds.
- **§C8 (draft). Workspace-level dependencies and feature unification.** Cargo features are union'd across the workspace — a crate can unexpectedly receive a feature it explicitly disabled.
- **§C9 (draft). `tracing` instrumentation patterns.** Span leakage across `.await` boundaries, context loss in `tokio::spawn`.

## 3. Meta-layer refinements

- **Trigger table:** cover ~5 more prompt patterns observed in real user requests.
- **Calibrated uncertainty:** add a self-assessment scale for cases where LLMs are prone to overconfidence (§B3 already flagged as one such case).
- **Repro snippets:** for each BANNED formulation, attach a minimal compilable example (needed as the test corpus for §1 tooling).

## 4. Infrastructure

- `git init` and a public repository.
- CI: markdown linting, broken-internal-link checks.
- Test corpus: `examples/` with deliberately broken Rust per category, to run through `/rust-audit` as a regression suite.

## Open questions

- Should `rust-intel.md` be split into per-tier files, or is the current density still net-positive?
- Do human-readable artifacts (README, CHANGELOG, roadmap) need a Russian translation, or is English enough alongside the English spec?
- What's the right versioning granularity: each new category = minor, or batch them?
