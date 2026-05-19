---
description: Scan Rust code against the 26 categories from rust-intel and return a triaged report with concrete fixes.
argument-hint: "[path]"
---

# /rust-cc-audit

Audits Rust code against the full taxonomy in the `rust-intel` skill. Removes the developer's need to know all 26 categories — finds what a senior reviewer with that document in their head would catch.

## Arguments

- `$ARGUMENTS` — path to a file, directory, or crate. Defaults to the current working directory.

## Process

1. **Load the `rust-intel` skill.** This is the only source of rules. If the skill is unavailable, emit `⚠️ BLOCKED: skill rust-intel is not registered` and stop.

2. **Pin the world.** Read `Cargo.toml` (and `CLAUDE.md`, if present). Record the exact versions of `tokio`, `axum`, `sqlx`, `reqwest`, `serde`, `hyper`, `clap`, and any other key dependency. Without this, §A1 (API hallucinations) cannot be checked — block instead of guessing.

3. **Determine scope.**
   - If `$ARGUMENTS` is empty: walk `src/**/*.rs` relative to cwd.
   - If a file: just that file.
   - If a directory: every `*.rs` recursively, excluding `target/`.
   - Skip generated code (`OUT_DIR`, `build.rs` output).

4. **Walk every category in the skill.** Iterate §A1 through §C7 as enumerated in `rust-intel.md`. For each, apply that category's BANNED/REQUIRED rules verbatim from the skill — do not re-state them here. The skill is the single source of rule wording; this command is the workflow harness.

5. **For every finding, produce:**
   - **Category:** `§XN — name`
   - **File:line:column** (or line range for multiline patterns)
   - **Citation** of the relevant fragment (3–10 lines of context)
   - **Why it's dangerous** — one sentence referencing the spec's wording
   - **Concrete fix** — a patch or code that applies to this file (not generic advice like "use a bounded channel")
   - **Severity:** `critical` (silent data loss / UB / leak / deadlock), `high` (probable production bug), `medium` (antipattern with no immediate risk), `info` (style).

6. **Report grouping:**
   - By severity (critical → info).
   - Inside a severity, by tier (A → B → C).
   - End with a Post-flight summary in the spec's canonical form (every `unsafe`, `unwrap`, `Arc<Mutex<_>>`, double lock, `.lock().unwrap()`, crypto call, new dependency, etc. — the list at the bottom of `rust-intel.md`).

## Report format

```
# rust-audit report

**Scope:** <path>
**Pinned versions:** tokio=X.Y, sqlx=A.B, ...
**Found:** N critical, M high, K medium, L info

---

## CRITICAL

### [§B2] src/handler.rs:47–52 — Mutex held across .await
```rust
let guard = state.lock().unwrap();
let value = guard.get(&key).cloned();
some_async_op(value).await  // ← guard still alive
```
**Why dangerous:** `std::sync::Mutex` blocks the tokio worker across `.await` — deadlocks under load.
**Fix:**
```rust
let value = {
    let guard = state.lock().unwrap();
    guard.get(&key).cloned()
};  // guard dropped before .await
some_async_op(value).await
```

### [§B8] src/notifier.rs:88 — Forgotten .await
...

---

## HIGH
...

---

## Post-flight summary

- `unsafe`: 0
- `unwrap`/`expect`: 4 (src/parse.rs:12, src/parse.rs:34, ...)
- `Arc<Mutex<_>>`: 2 (src/state.rs:8, src/cache.rs:22)
- Double locks in one function: 1 (src/handler.rs:91 — order undocumented, §B9 risk)
- `.lock().unwrap()`: 5 (consider explicit poisoning handling, §B2)
- Crypto calls: none
- New dependencies: none
- `unbounded_channel`: 1 (src/events.rs:14 — unjustified, §B14)
```

## Behavioral principles

- **Don't invent findings.** If a category isn't activated, don't mention it. A short report beats a synthetic one.
- **Don't "fix" in the repo.** Report only. Applying fixes is a separate step the user authorizes.
- **Block on uncertainty.** If a crate version is unknown and §A1 needs it, emit a blocking message — don't guess.
- **Don't restate the spec.** Reference the paragraph (`§B2`) instead of paraphrasing its text.

## Limits

- This is static analysis via reading. It doesn't replace `cargo clippy`, `miri`, `tokio-console`, `loom` — the spec's Post-flight checklist still recommends them explicitly.
- Categories that need runtime observation (steady-state memory growth for §B10) can only be flagged as "candidate" — not confirmed without profiling.
