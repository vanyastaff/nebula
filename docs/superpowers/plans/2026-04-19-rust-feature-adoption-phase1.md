# Rust Feature Adoption — Phase 1 (Free-Lunch Sweep) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close two low-blast-radius tasks from the Rust 1.75–1.95 feature adoption rollup: remove the last `once_cell` dependency (Phase 1a, one workspace PR) and convert documented `#[allow(...)]` attributes to `#[expect(...)]` across the workspace (Phase 1b, one PR per crate).

**Architecture:**
- **Phase 1a** — Flip the sole remaining `once_cell::sync::OnceCell` at [crates/expression/src/maybe.rs:7](crates/expression/src/maybe.rs:7) to `std::sync::OnceLock` (stable 1.70, bound-compatible, API superset for the `new()`-only usage in this file). Then drop `once_cell = "1.21"` from the workspace and crate Cargo.toml files. Single PR.
- **Phase 1b** — For each of 19 workspace members with `#[allow(...)]` attributes, apply a conversion rubric: flip to `#[expect(...)]` only when the attribute has an explanatory comment or clear contextual rationale; leave bare `#[allow]` alone — those are legitimate forward-compatibility for future lints. Each crate gets its own PR (they're independent; chips can run in parallel). `unfulfilled_lint_expectations` warns when an `#[expect]` no longer fires — that is the intended regression guard.

**Tech stack:** Rust 2024 / 1.95 (MSRV), stable `std::sync::OnceLock` (1.70), `#[expect(lint)]` attribute (1.81 — RFC 2383).

**Scope note — this plan covers Phase 1 only.** Phases 2–5 from the source spec ([docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md](docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md)) — inherent AFIT, `dynosaur` migration, precise-capture `use<>`, and late polish — are significant multi-PR, ADR-gated efforts. Each should get its own plan written after Phase 1 ships. Do not bundle them here.

---

## Baseline counts at HEAD (commit `822ed65d`)

| Metric | Count |
|---|---:|
| `once_cell` call sites in compiled code | 1 (`crates/expression/src/maybe.rs`) |
| `lazy_static!` invocations | 0 (already migrated) |
| `once_cell` workspace dep declarations | 2 (root `Cargo.toml:79` + `crates/expression/Cargo.toml:30`) |
| `#[allow(...)]` in `crates/` + `apps/` | 117 across 19 workspace members |
| `#[expect(...)]` already adopted | 26 across 20 files |
| Conversion budget (Phase 1b) | ~80–90 (the rest are bare `#[allow]` with no rationale — leave them) |

Re-run these before starting (see *Self-preflight commands* at end) to catch drift since plan was written.

---

## File Structure

**Modified in Phase 1a (one PR):**
- `crates/expression/src/maybe.rs` — swap `once_cell::sync::OnceCell` → `std::sync::OnceLock` at line 7 (import) and lines 20, 27, 92, 330 (call sites).
- `Cargo.toml` — delete `once_cell = "1.21"` from `[workspace.dependencies]` (line 79).
- `crates/expression/Cargo.toml` — delete the `once_cell = { workspace = true }` line from `[dependencies]` (line 30).
- `Cargo.lock` — regenerated automatically by Cargo.

**Modified in Phase 1b (one PR per crate; up to 19 PRs):**
- Any `.rs` file inside the target crate that contains a `#[allow(...)]` attribute with documented rationale. Precise paths per crate are enumerated in Tasks 5–23.

**Out of scope for this plan:**
- Phases 2–5 of the source spec (get their own plans).
- Bare `#[allow]` without explanatory comments (legitimate forward-compat; keep as `allow`).
- Markdown design docs (`crates/*/docs/*.md`, `crates/*/plans/*.md`) — illustrative, not compiled.
- Rewriting the rationale comments themselves — a conversion chip should not editorialise on why the attribute exists; it should only flip `allow → expect` where the existing rationale already makes sense.

---

## Task 1: Phase 1a — swap `OnceCell` → `OnceLock` in `maybe.rs`

**Files:**
- Modify: `crates/expression/src/maybe.rs`

**Context:** `CachedExpression.ast` is a lazily-initialised AST cache. The file only uses `OnceCell::new()` for construction — no `get_or_init` / `get_or_try_init` / `set`. `std::sync::OnceLock` is a superset for this shape and has identical `Send + Sync` bounds (`T: Send` for `Send`, `T: Send + Sync` for `Sync`), so no downstream trait bound shifts. `CachedExpression` is `pub` and re-exported via `crates/expression/src/lib.rs:91`; the `ast` field is `#[doc(hidden)]` but technically public — callers that touched it would need to flip type names too. Verify no downstream uses before merging.

- [ ] **Step 1: Confirm no external `.ast` accesses across the workspace**

Run:
```bash
rg --type rust '\.ast\b' --glob '!crates/expression/**'
```
Expected: no matches on the `CachedExpression.ast` field. Matches on unrelated `.ast` names (AST trait impls, proc-macro `syn::ast`, etc.) are OK — verify by inspection that none resolve to `CachedExpression::ast`.

- [ ] **Step 2: Replace the import at `crates/expression/src/maybe.rs:7`**

Old:
```rust
use once_cell::sync::OnceCell;
```

New:
```rust
use std::sync::OnceLock;
```

- [ ] **Step 3: Replace every `OnceCell` identifier in the file with `OnceLock`**

Four sites, all in `crates/expression/src/maybe.rs`:

Line 20 — field type:
```rust
pub ast: OnceLock<Expr>,
```

Line 27 — `Clone` impl:
```rust
ast: OnceLock::new(), // Don't clone the cached AST, let it re-parse if needed
```

Line 92 — `expression()` constructor:
```rust
ast: OnceLock::new(),
```

Line 330 — `Deserialize` impl:
```rust
ast: OnceLock::new(),
```

Rule of thumb: `sed -i 's/OnceCell/OnceLock/g' crates/expression/src/maybe.rs` gets the same result, but verify by `git diff` afterwards that nothing unexpected flipped.

- [ ] **Step 4: Build the crate to confirm the swap compiles**

Run:
```bash
cargo check -p nebula-expression
```
Expected: success. If you see an `unresolved import std::sync::OnceLock` error, MSRV has been mis-set — confirm `rust-toolchain.toml` pins 1.95.

- [ ] **Step 5: Run `nebula-expression` tests including the `CachedExpression` serde round-trip tests**

Run:
```bash
cargo nextest run -p nebula-expression
```
Expected: all tests pass. Specifically `test_serde_value`, `test_serde_expression`, `test_resolve_string_expression`, and `test_maybe_expression_expression` exercise the `CachedExpression` constructor — they must be green.

- [ ] **Step 6: Commit the swap**

```bash
git add crates/expression/src/maybe.rs
git commit -m "refactor(expression): OnceCell → std::sync::OnceLock in MaybeExpression

OnceLock (stable 1.70) is API-compatible for our OnceCell::new()-only
usage and has identical Send+Sync bounds. Prepares the workspace for
removing the once_cell dependency.

Part of Rust 1.75-1.95 feature adoption — Phase 1a."
```

---

## Task 2: Phase 1a — drop the `once_cell` workspace dependency

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/expression/Cargo.toml`
- Auto-update: `Cargo.lock`

- [ ] **Step 1: Delete the crate-level dependency declaration**

In `crates/expression/Cargo.toml`, remove this line from `[dependencies]`:

Old (approximately line 30):
```toml
once_cell = { workspace = true }
```

After removal, the `[dependencies]` block should no longer reference `once_cell`.

- [ ] **Step 2: Delete the workspace-level dependency declaration**

In the root `Cargo.toml`, remove this line from `[workspace.dependencies]`:

Old (line 79):
```toml
once_cell = "1.21"
```

- [ ] **Step 3: Regenerate the lockfile so Cargo drops the transitive entry**

Run:
```bash
cargo update --workspace
```
Expected: `once_cell` entries removed from `Cargo.lock` (confirm by `rg once_cell Cargo.lock` — it may still surface as a transitive dep of `clap_builder`, `hashbrown`, `proc-macro-crate`, etc.; those are not ours to drop, only our direct dep is gone).

- [ ] **Step 4: Run the workspace quickgate**

Run in sequence:
```bash
cargo +nightly fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```
Expected: all green. Doctests matter here because `MaybeExpression` has inline rustdoc examples that construct `CachedExpression`.

- [ ] **Step 5: Confirm no `once_cell` references survive in compiled code**

Run:
```bash
rg --type rust 'once_cell' crates/ apps/ examples/
rg 'once_cell' Cargo.toml crates/*/Cargo.toml apps/*/Cargo.toml examples/*/Cargo.toml
```
Expected: no matches in `.rs` files; no matches in any `Cargo.toml`. Matches in `Cargo.lock` transitive entries (not workspace-owned) are acceptable — we only control direct deps.

- [ ] **Step 6: Run `cargo deny check` to confirm the dep graph is still policy-compliant**

Run:
```bash
cargo deny check
```
Expected: `advisories`, `bans`, `sources`, `licenses` all `ok`. If `duplicate` flags a new dup, investigate before proceeding — the removal should only reduce, not grow, dup count.

- [ ] **Step 7: Commit the dep removal**

```bash
git add Cargo.toml crates/expression/Cargo.toml Cargo.lock
git commit -m "chore(deps): drop once_cell workspace dependency

Workspace now has zero once_cell call sites after the OnceCell →
OnceLock swap in nebula-expression. lazy_static! was already at zero.
LazyLock / OnceLock (stable 1.70 / 1.80) are the workspace standard.

Part of Rust 1.75-1.95 feature adoption — Phase 1a (done)."
```

---

## Task 3: Phase 1b — establish the `#[allow] → #[expect]` conversion rubric

**Files:** (none modified in this task — it establishes the rubric applied by Tasks 4–22)

**Context:** `#[expect(lint)]` (stable 1.81) differs from `#[allow(lint)]` in that it emits `unfulfilled_lint_expectations` if the underlying lint *does not* fire at the attribute site. This is the regression guard — an attribute the author thought was muting something that no longer triggers gets surfaced, letting us delete it instead of carrying dead noise. Conversion is a simple mechanical flip (`allow` → `expect`) but only when the author clearly meant "this lint is firing and we accept it" rather than "keep this quiet in case a future lint triggers".

- [ ] **Step 1: Read the rubric below; keep it in a scratchpad for every Phase 1b task**

**Convert `#[allow(X)]` → `#[expect(X, reason = "...")]` when:**
1. An explanatory comment immediately above (or inline) names the specific reason — e.g. `// dead_code: only used when feature X is on`, `// SAFETY: pre_exec runs between fork/exec`, or a doc comment that makes the rationale obvious from context (e.g. a trait method with `#[allow(clippy::len_without_is_empty)]` where the doc says "always ≥ 1 by construction").
2. The lint can be confirmed to still fire. Validate by removing the attribute and running `cargo clippy -p <crate> --all-targets` — the warning should re-appear. Re-add the attribute, flipped to `#[expect]`.
3. The attribute is narrow: a single lint, scoped to one item (fn, struct, module). Prefer the tightest scope.
4. **Add `reason = "..."` on every converted `#[expect]`.** The workspace convention (24 of 26 pre-existing `#[expect]` attrs in `src/` / `tests/`) is multi-line:
   ```rust
   #[expect(
       lint_name,
       reason = "short phrase naming the invariant or rationale"
   )]
   ```
   Use the existing rationale comment above as source material for the `reason` string — do NOT paraphrase loosely; keep the reason terse (≤ 120 chars). Preserve the doc / SAFETY comment above untouched.

**Leave as `#[allow(X)]` when:**
1. No rationale is present and none is obvious from context.
2. The attribute is on a module-level `#![allow(...)]` that is forward-compat scaffolding (typically `#![allow(dead_code)]` at the top of a test-helper module or a generated-code module where contents churn rapidly).
3. The attribute is inside a macro expansion we can't easily influence.
4. Removing it makes clippy *not* warn — that means the lint doesn't apply right now and `expect` would immediately trip `unfulfilled_lint_expectations`.

**Never do:**
- Blanket `sed -i 's/allow/expect/g'` — the whole point of Phase 1b is the rubric judgement call.
- Editorialise the existing rationale comment — if it reads coherently, leave the prose alone; only flip the attribute.
- Bundle two crates into one PR — each crate is its own chip so `git bisect` stays surgical.

- [ ] **Step 2: Confirm baseline `#[allow]` and `#[expect]` counts before starting any chip**

Run:
```bash
rg --count-matches '#\[allow\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print "allow:", sum}'
rg --count-matches '#\[expect\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print "expect:", sum}'
```
Expected (at plan-write time): `allow: 117`, `expect: 26`. Drift is normal — note your own numbers and use them as the baseline that each chip should reduce the `allow` count against.

- [ ] **Step 3: Read the three exemplar attributes below — they define the "convert" yes/no call for the whole rollout**

Convert-yes example (`crates/plugin/src/versions.rs:114`):
```rust
/// Number of versions stored (always ≥ 1 by construction).
#[allow(clippy::len_without_is_empty)]
pub fn len(&self) -> usize {
    self.versions.len()
}
```
The doc comment **explicitly justifies** why `is_empty` would be nonsense (the type's invariant is `len ≥ 1`). Rationale is present, lint is firing, scope is single-item. → flip to `#[expect(clippy::len_without_is_empty)]`.

Convert-yes example (`crates/sandbox/src/process.rs:725`):
```rust
// SAFETY: pre_exec runs between fork() and exec() in the child.
// We only call async-signal-safe operations (landlock, setrlimit).
#[allow(unsafe_code)]
unsafe {
    cmd.pre_exec(move || { ... });
}
```
Extensive SAFETY comment above the attribute. → flip to `#[expect(unsafe_code)]`.

Convert-yes example (`crates/expression/src/builtins.rs:226`):
```rust
/// Helper to extract a lambda expression from args
#[allow(dead_code)]
pub(crate) fn extract_lambda(arg: &Expr) -> ExpressionResult<(&str, &Expr)> {
    match arg { ... }
}
```
The helper is gated by future callers; doc comment makes that explicit. → flip to `#[expect(dead_code)]`. **Caveat:** if clippy no longer flags this after the next few feature landings, the `expect` will fire `unfulfilled_lint_expectations` — that is the signal to delete the helper. Good.

Skip example (hypothetical):
```rust
#![allow(dead_code)]  // at top of a generated-code module
```
Module-level blanket. Keep as `allow`.

---

## Tasks 4–22: Phase 1b per-crate conversion chips

Each task is **one PR** converting documented `#[allow]` attributes in a single workspace member. Tasks are listed in ascending size order (smallest crate first — lets you validate the procedure cheaply before hitting the big ones). Each chip follows the same six-step shape:

1. Create a chip branch off `main` (or the Phase 1 integration branch).
2. Run `rg '#\[allow\(' crates/<name>/ apps/<name>/` to list every site.
3. Apply the rubric from Task 3 to each site; flip to `#[expect(...)]` where it qualifies.
4. Run crate-local verify: `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo nextest run -p <crate>`.
5. Confirm no `unfulfilled_lint_expectations` warnings appear in the clippy run.
6. Commit with message `refactor(<crate>): #[allow] → #[expect] where documented (Phase 1b)`.

Each task records the target crate, the site count (raw upper bound — actual flipped count will be ≤ this), and the exact verify commands.

### Task 4: `nebula-expression` chip (1 site)

**Target:** `crates/expression/src/builtins.rs:226` — `#[allow(dead_code)]` on `extract_lambda` with doc comment.

- [ ] **Step 1: Branch**
```bash
git checkout -b chip/expect-expression
```

- [ ] **Step 2: List the `#[allow]` sites**
```bash
rg -n '#\[allow\(' crates/expression/
```
Expected: one hit, line 226 of `builtins.rs`.

- [ ] **Step 3: Flip the attribute**

In `crates/expression/src/builtins.rs:226`, change:
```rust
#[allow(dead_code)]
```
to:
```rust
#[expect(dead_code)]
```
(Doc comment above is the rationale — leave it untouched.)

- [ ] **Step 4: Verify locally**
```bash
cargo clippy -p nebula-expression --all-targets -- -D warnings
cargo nextest run -p nebula-expression
```
Expected: clippy green, tests green, no `unfulfilled_lint_expectations`.

- [ ] **Step 5: Commit**
```bash
git add crates/expression/src/builtins.rs
git commit -m "refactor(expression): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 5: `nebula-plugin` chip (1 site)

**Target:** `crates/plugin/src/versions.rs:114` — `#[allow(clippy::len_without_is_empty)]` on `len()`, doc states the invariant.

- [ ] **Step 1: Branch off main**
```bash
git checkout main
git checkout -b chip/expect-plugin
```

- [ ] **Step 2: List the `#[allow]` sites**
```bash
rg -n '#\[allow\(' crates/plugin/
```
Expected: one hit (`versions.rs:114`).

- [ ] **Step 3: Flip the attribute**
In `crates/plugin/src/versions.rs:114`, change `#[allow(clippy::len_without_is_empty)]` to `#[expect(clippy::len_without_is_empty)]`.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-plugin --all-targets -- -D warnings
cargo nextest run -p nebula-plugin
```

- [ ] **Step 5: Commit**
```bash
git add crates/plugin/src/versions.rs
git commit -m "refactor(plugin): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 6: `nebula-sandbox` chip (1 site)

**Target:** `crates/sandbox/src/process.rs:725` — `#[allow(unsafe_code)]` with SAFETY comment.

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-sandbox
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/sandbox/
```
Expected: one hit (`process.rs:725`).

- [ ] **Step 3: Flip**
In `crates/sandbox/src/process.rs:725`, change `#[allow(unsafe_code)]` to `#[expect(unsafe_code)]`.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-sandbox --all-targets -- -D warnings
cargo nextest run -p nebula-sandbox
```

- [ ] **Step 5: Commit**
```bash
git add crates/sandbox/src/process.rs
git commit -m "refactor(sandbox): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 7: `nebula-api` chip (2 sites)

**Targets:** `crates/api/tests/webhook_transport_integration.rs`, `crates/api/tests/common/mod.rs`.

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-api
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/api/
```
Expected: two hits (both in `tests/`).

- [ ] **Step 3: For each hit, apply the rubric from Task 3**

For each of the two sites:
- Read the attribute, the surrounding doc/comment context, and the item being annotated.
- If there is a clear rationale comment or the rationale is obvious (e.g. a known-unused test fixture that exists for future expansion), flip to `#[expect(...)]`.
- If the attribute is a bare `#![allow(dead_code)]` at the top of a test helper module with no rationale, leave it as `allow`.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-api --all-targets -- -D warnings
cargo nextest run -p nebula-api
```

- [ ] **Step 5: Commit**
```bash
git add crates/api/tests/
git commit -m "refactor(api): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 8: `nebula-plugin-sdk` chip (2 sites)

**Targets:** `crates/plugin-sdk/tests/broker_smoke.rs` (2 hits).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-plugin-sdk
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/plugin-sdk/
```

- [ ] **Step 3: Apply rubric to each hit**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-plugin-sdk --all-targets -- -D warnings
cargo nextest run -p nebula-plugin-sdk
```

- [ ] **Step 5: Commit**
```bash
git add crates/plugin-sdk/
git commit -m "refactor(plugin-sdk): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 9: `nebula-credential` chip (2 sites)

**Targets:** `crates/credential/src/handle.rs`, `crates/credential/src/layer/encryption.rs`.

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-credential
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/credential/
```

- [ ] **Step 3: Apply rubric.** Reminder: credential code touches the encryption layer — do not rename or rewrite comments, only flip the attribute.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-credential --all-targets -- -D warnings
cargo nextest run -p nebula-credential
```

- [ ] **Step 5: Commit**
```bash
git add crates/credential/
git commit -m "refactor(credential): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 10: `nebula-core` chip (3 sites)

**Targets:** `crates/core/src/lib.rs` (2), `crates/core/src/id/types.rs` (1).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-core
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/core/
```

- [ ] **Step 3: Apply rubric.** `lib.rs` hits may be crate-level `#![allow(...)]` — inspect carefully; crate-level blanket attrs without rationale stay as `allow`.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-core --all-targets -- -D warnings
cargo nextest run -p nebula-core
```

- [ ] **Step 5: Commit**
```bash
git add crates/core/
git commit -m "refactor(core): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 11: `nebula-action` chip (3 sites)

**Targets:** `crates/action/tests/derive_action.rs` (1), `crates/action/src/webhook.rs` (2).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-action
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/action/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-action --all-targets -- -D warnings
cargo nextest run -p nebula-action
```

- [ ] **Step 5: Commit**
```bash
git add crates/action/
git commit -m "refactor(action): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 12: `nebula-sdk` chip (3 sites)

**Targets:** `crates/sdk/macros-support/src/attrs.rs` (3).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-sdk
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/sdk/
```

- [ ] **Step 3: Apply rubric.** Macro support code — inspect whether attributes silence lints that fire *inside* generated code; if so, flipping may trip `unfulfilled_lint_expectations` at call sites where the lint doesn't fire for all inputs. If clippy -D warnings passes after the flip, the conversion is correct.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-sdk --all-targets -- -D warnings
cargo nextest run -p nebula-sdk
```

- [ ] **Step 5: Commit**
```bash
git add crates/sdk/macros-support/
git commit -m "refactor(sdk): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 13: `nebula-system` chip (4 sites)

**Targets:** `crates/system/src/disk.rs` (2), `crates/system/src/info.rs` (1), `crates/system/src/lib.rs` (1).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-system
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/system/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-system --all-targets -- -D warnings
cargo nextest run -p nebula-system
```

- [ ] **Step 5: Commit**
```bash
git add crates/system/
git commit -m "refactor(system): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 14: `nebula-cli` (apps) chip (4 sites)

**Targets:** `apps/cli/src/tui/event.rs` (2), `apps/cli/src/tui/app.rs` (2).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-cli
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' apps/cli/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-cli --all-targets -- -D warnings
cargo nextest run -p nebula-cli
```
(Package name may differ — confirm with `cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.manifest_path | contains("apps/cli")) | .name'` if the above fails.)

- [ ] **Step 5: Commit**
```bash
git add apps/cli/
git commit -m "refactor(cli): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 15: `nebula-validator` chip (6 sites)

**Targets:** `crates/validator/src/prelude.rs` (1), `crates/validator/src/macros.rs` (5).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-validator
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/validator/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-validator --all-targets -- -D warnings
cargo nextest run -p nebula-validator
```

- [ ] **Step 5: Commit**
```bash
git add crates/validator/
git commit -m "refactor(validator): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 16: `nebula-desktop` (apps) chip (6 sites)

**Targets:** `apps/desktop/src-tauri/src/error.rs` (3), `apps/desktop/src-tauri/src/auth/oauth.rs` (3).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-desktop
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' apps/desktop/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**

Desktop uses Tauri — verify inside the Tauri manifest dir:
```bash
( cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings && cargo nextest run )
```

- [ ] **Step 5: Commit**
```bash
git add apps/desktop/
git commit -m "refactor(desktop): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 17: `nebula-engine` chip (7 sites)

**Targets:** `crates/engine/src/engine.rs` (7).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-engine
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/engine/
```

- [ ] **Step 3: Apply rubric.** `engine.rs` is the ~7900-LOC monolith — the seven hits may be clustered; inspect each individually and avoid drive-by edits.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-engine --all-targets -- -D warnings
cargo nextest run -p nebula-engine
```

- [ ] **Step 5: Commit**
```bash
git add crates/engine/src/engine.rs
git commit -m "refactor(engine): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 18: `nebula-log` chip (8 sites)

**Targets:** `crates/log/src/observability/registry.rs` (1), `crates/log/src/config/writer.rs` (2), `crates/log/src/builder/telemetry.rs` (1), `crates/log/src/builder/mod.rs` (2), `crates/log/examples/custom_observability.rs` (2).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-log
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/log/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-log --all-targets -- -D warnings
cargo nextest run -p nebula-log
```

- [ ] **Step 5: Commit**
```bash
git add crates/log/
git commit -m "refactor(log): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 19: `nebula-storage` chip (8 sites)

**Targets:** `crates/storage/src/rows/mod.rs` (8).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-storage
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/storage/
```

- [ ] **Step 3: Apply rubric.** `rows/mod.rs` hits are likely row-mapping structs that some columns are only needed for the typed derive — confirm rationale per row.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-storage --all-targets -- -D warnings
cargo nextest run -p nebula-storage
```

- [ ] **Step 5: Commit**
```bash
git add crates/storage/
git commit -m "refactor(storage): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 20: `nebula-resource` chip (10 sites)

**Targets:** `crates/resource/tests/dx_evaluation.rs` (1), `crates/resource/tests/dx_audit.rs` (1), `crates/resource/tests/basic_integration.rs` (3), `crates/resource/src/runtime/managed.rs` (2), `crates/resource/src/runtime/pool.rs` (3).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-resource
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/resource/
```

- [ ] **Step 3: Apply rubric**

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-resource --all-targets -- -D warnings
cargo nextest run -p nebula-resource
```

- [ ] **Step 5: Commit**
```bash
git add crates/resource/
git commit -m "refactor(resource): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 21: `nebula-schema` chip (20 sites)

**Targets:** source: `crates/schema/src/has_schema.rs` (1), `field.rs` (1), `expression.rs` (2), `key.rs` (1), `path.rs` (1), `value.rs` (1), `validated.rs` (2). Tests: `crates/schema/tests/derive_schema.rs` (6), `tests/compile_fail/*.rs` (5 files × 1 each).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-schema
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/schema/
```

- [ ] **Step 3: Apply rubric.** Compile-fail tests intentionally fail to compile; their `#[allow]`s are on the *test harness* code, not the failing snippet — the lint may or may not fire depending on how the test harness is built. For each, verify by removing and running the compile-fail suite; re-add as `expect` only if the lint still fires.

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-schema --all-targets -- -D warnings
cargo nextest run -p nebula-schema
```

- [ ] **Step 5: Commit**
```bash
git add crates/schema/
git commit -m "refactor(schema): #[allow] → #[expect] where documented (Phase 1b)"
```

### Task 22: `nebula-resilience` chip (25 sites — largest)

**Targets:** `crates/resilience/src/circuit_breaker.rs` (7), `retry.rs` (3), `rate_limiter.rs` (10), `pipeline.rs` (1), `hedge.rs` (3), `gate.rs` (1).

- [ ] **Step 1: Branch**
```bash
git checkout main && git checkout -b chip/expect-resilience
```

- [ ] **Step 2: List sites**
```bash
rg -n '#\[allow\(' crates/resilience/
```

- [ ] **Step 3: Apply rubric.** Highest per-crate count; budget an extra hour. `rate_limiter.rs` alone has 10 — work file-by-file and commit incrementally within the branch (no push until the whole crate is done).

- [ ] **Step 4: Verify**
```bash
cargo clippy -p nebula-resilience --all-targets -- -D warnings
cargo nextest run -p nebula-resilience
```

- [ ] **Step 5: Commit**
```bash
git add crates/resilience/
git commit -m "refactor(resilience): #[allow] → #[expect] where documented (Phase 1b)"
```

---

## Task 23: Post-sweep final verification

**Files:** (none — pure verification)

**Context:** After Phase 1a and all per-crate chips merge, confirm the workspace is in the state the plan targets. Run against `main` (or the integration branch that carries all 20 PRs from Tasks 1–22).

- [ ] **Step 1: Confirm `once_cell` surface is zero in compiled code**

```bash
rg --type rust 'once_cell' crates/ apps/ examples/
rg 'once_cell' Cargo.toml crates/*/Cargo.toml apps/*/Cargo.toml examples/*/Cargo.toml
```
Expected: both commands return no matches.

- [ ] **Step 2: Confirm `#[allow]` count dropped within budget**

```bash
rg --count-matches '#\[allow\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print sum}'
rg --count-matches '#\[expect\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print sum}'
```
Expected: `#[allow]` total dropped to ~27–37 (the bare forward-compat attrs that stayed); `#[expect]` total grew to ~106–116. The exact split depends on rubric calls — the combined sum should stay near 143 minus the "skip" set (no net conversion loss).

- [ ] **Step 3: Confirm no `unfulfilled_lint_expectations` in a full workspace build**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep -i 'unfulfilled_lint_expectations' || echo 'OK: no unfulfilled expectations'
```
Expected: `OK: no unfulfilled expectations`.

- [ ] **Step 4: Run the canonical quickgate end-to-end**

```bash
cargo +nightly fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
```
Expected: all green. If any check trips, it's almost certainly a chip that landed with a stale `#[expect]` — bisect across the chip merges to find it.

- [ ] **Step 5: Refresh the CI artifact counts referenced by the spec**

Update the spec file at [docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md](docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md) — bump the "adopted / remaining `#[allow]`" numbers in the Inventory section's `#[allow(...)]` → `#[expect(...)]` subsection to the new counts from Step 2. This keeps the source-of-truth aligned when Phase 2 picks up.

- [ ] **Step 6: Note in `docs/MATURITY.md` that Phase 1 (free-lunch) is complete**

Find the row referencing the Rust feature adoption rollup (if one exists) or add a one-line entry in the relevant workspace-health note that says "2026-xx-xx: Phase 1 of Rust 1.75–1.95 adoption complete — `once_cell` dep dropped, ~80 `#[expect]` conversions landed across 19 chips". Small note — part of docs-sync.

---

## Self-preflight commands (run before starting)

These reproduce the baseline counts in the plan header. Drift since plan write is expected; use your own output as the baseline that each chip should reduce.

```bash
# once_cell call sites in compiled code
rg --count-matches 'once_cell' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print "once_cell sites:", sum}'
# expected ≈ 1

# lazy_static! invocations (should be zero)
rg --count-matches 'lazy_static!' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print "lazy_static! sites:", sum}'
# expected = 0

# #[allow] and #[expect] totals
rg --count-matches '#\[allow\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print "allow:", sum}'
rg --count-matches '#\[expect\(' --glob '*.rs' crates/ apps/ | awk -F: '{sum+=$2} END {print "expect:", sum}'
# expected ≈ allow: 117, expect: 26
```

---

## Hazards and rollback

- **once_cell swap hazard** — if external callers touch `CachedExpression.ast` via the doc-hidden field (Step 1 of Task 1 confirms this is zero in workspace), they will fail to compile after the type rename. Since the field is `#[doc(hidden)]`, this is not a SemVer-breaking guarantee; still, include the type-flip in the release notes if `nebula-expression` publishes between this PR and the next.
- **Lint drift hazard** — a chip that flips `allow → expect` based on today's clippy behaviour can trip `unfulfilled_lint_expectations` after a future clippy update changes scope. The mitigation is the guard itself — next CI run surfaces the stale `expect`, and it either becomes a delete (attribute no longer needed) or a flip back to `allow` (lint semantics shifted). Treat trips as information, not incidents.
- **Rollback** — each of Tasks 1, 2, 4–22 is a self-contained PR. Revert is a single `git revert <sha>` against any individual chip. Phase 1a revert would require reintroducing `once_cell` in both Cargo.toml files and the import line in `maybe.rs`.

---

## Self-review checklist (for the plan author)

Before handing this plan to an implementor, confirm:

- [ ] Every spec-side requirement for Phase 1 from [2026-04-19-rust-feature-adoption-plan.md §Phase 1](docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md) is covered by a task. (Phase 1a — Tasks 1 + 2; Phase 1b — Task 3 rubric + Tasks 4–22 per-crate chips + Task 23 verification.)
- [ ] No "TBD" / "TODO" / "similar to Task N" placeholders. Chip tasks (4–22) intentionally share a structure because the six-step shape is identical — each task restates the shape with its own crate name and counts.
- [ ] Cargo commands use consistent flags (`--all-targets -- -D warnings`) and crate-scoped targets (`-p <name>`).
- [ ] Commit-message convention matches workspace norms (`refactor(scope): …`, `chore(deps): …`).
- [ ] The rubric in Task 3 is explicit about both "convert" and "skip" cases and gives three concrete exemplars, not one.
- [ ] The crate rollout order is ascending by count (1 → 25) — lets an implementor validate the procedure on easy crates before hitting the 25-site `nebula-resilience` chip.
