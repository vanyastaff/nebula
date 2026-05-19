---
description: Maps an error (rustc / clippy / panic / runtime anomaly) onto a rust-intel category and proposes a root-cause fix.
argument-hint: "<error message or behavior description>"
---

# /rust-cc-fix

Removes the developer's need to navigate rustc docs and StackOverflow. Takes a symptom — returns the cause, the fix, **and** a preventive rule so it doesn't recur.

## Arguments

- `$ARGUMENTS` — a `rustc` message, `cargo clippy` output, panic backtrace, or runtime-behavior description in natural language. May be multiline.

## Process

1. **Load the `rust-intel` skill.** If unavailable, emit `⚠️ BLOCKED: skill rust-intel is not registered` and stop.

2. **Classify the input:**
   - Compiler error (has `error[EXXXX]` or `error:` marker).
   - Clippy warning (has `warning: ... #[warn(clippy::...)]`).
   - Panic / runtime stack trace (has `thread 'main' panicked` or a backtrace).
   - Natural-language anomaly (deadlock, OOM, slow, intermittent flake).

3. **Request context when needed:**
   - For a compiler error — need the relevant source lines (or the file path). If neither is provided, ask. **Don't guess.**
   - For runtime symptoms — need `Cargo.toml` for versions and a repro scenario, if the category depends on either (§A1, §B4).
   - If context is insufficient, emit a blocking message in the spec's canonical form.

4. **Map to a category.** Use the routing table below — it is **only a router** (symptom → category number). The actual rule wording, BANNED/REQUIRED bullets, and remediation guidance live in the skill, never duplicated here. Whenever a new category lands in the `rust-intel` skill (`SKILL.md`), extend this routing table accordingly. Table is non-exhaustive — when no row matches, read the spec's taxonomy directly.

   | Symptom | Category |
   |---|---|
   | E0433, E0432, E0425, E0412, E0405 | §A1 (project organization, API hallucination) |
   | E0277, E0308, E0599, E0407 | §A2 (trait bounds / types) |
   | E0277 with `Send` / `Sync` in the bound | §A2 + §B2/§B15 (context-dependent) |
   | E0596, E0594, E0502, E0499 | §C5 candidate (but check the ownership design first — don't slap `.clone()`) |
   | E0106, E0495, E0521 | §B1 (lifetime laundering / leaking) |
   | `clippy::await_holding_lock` | §B2 (but clippy catches ~30% — check hidden cases too) |
   | `clippy::clone_on_copy`, `clippy::redundant_clone` | §C5 |
   | `clippy::unwrap_used`, `clippy::expect_used` | §C2 |
   | `clippy::missing_safety_doc`, `clippy::undocumented_unsafe_blocks` | §B5 |
   | panic "Cannot start a runtime from within a runtime" | §B15 (block_on inside async) |
   | panic "cannot recursively acquire mutex" | §B9 (lock ordering / re-entry) |
   | panic "PoisonError" / `poisoned lock` | §B2 (poisoning cascade) |
   | Task hangs, `Poll::Pending` forever | §B15 (Waker not registered) |
   | Deadlock without panic, two threads waiting on each other | §B9 |
   | Steady-state RAM growth, OOM after days | §B10 (cycles) or §B14 (unbounded queue) |
   | Latency spike under load, executor starvation | §B11 (blocking executor) |
   | "Under load, `expensive_fetch` runs N times instead of 1" | §B13 (TOCTOU) |
   | "The message/request/write didn't happen but no error either" | §B8 (forgotten `.await`) |
   | Encrypt/decrypt works, but security review finds a vulnerability | §B12 |
   | Feature never activates, code is dead | §C7 (feature typo) |

   If the symptom maps to **multiple** categories, list them all and explain which is primary.

5. **Compose the answer:**
   - **Category(ies):** `§XN` referencing the paragraph.
   - **Real cause:** one or two sentences. Not a paraphrase of the symptom — why it arises in light of the spec's rule.
   - **Why the "obvious" fix is bad** (when one exists — especially §C5 reflexive `.clone()`).
   - **Right fix:** code or patch for the user's concrete example. If code wasn't shown — general form plus an explicit request to share the actual fragment.
   - **Preventive rule** from the spec in one line: what to add to the style/checklist so it doesn't recur.
   - **What to run after the fix:** the matching clippy lint / miri / tokio-console — from the Post-flight checklist.

## Answer format

```
## §XN — <category name>

**Cause.** <…>

**The "obvious" fix that's also bad.** <when applicable — e.g. for a borrow error: "just add .clone()", per §C5>

**Right fix.**
```rust
<patch>
```

**Preventive rule.** <one line from the spec>

**Run after.** `cargo clippy -- -W clippy.await_holding_lock` (for §B2), `miri` (for §B5), `tokio-console` (for §B11), etc.
```

## Behavioral principles

- **Root cause, not symptom.** "Just add `.clone()` to make it compile" is a forbidden answer; see §C5. First ask whether ownership can be restructured.
- **Don't guess versions.** If the fix depends on a version (`axum::Server::bind` disappeared in 0.7), request `Cargo.toml` — don't invent.
- **Acknowledge uncertainty.** The spec warns explicitly: ~50% of LLM cancel-safety assessments in empirical testing were confidently wrong (§B3). If the symptom touches cancel-safety, enumerate every `.await` point and prove — don't assert.
- **Don't restate the solution in disguise.** If you've already named the cause as §B2, don't recite its rules in full — reference.

## Limits

- Doesn't replace static analysis. If the user has lots of code and the location is unclear, redirect to `/rust-cc-audit`.
- Doesn't execute code. All fixes are textual suggestions; the user applies them.
