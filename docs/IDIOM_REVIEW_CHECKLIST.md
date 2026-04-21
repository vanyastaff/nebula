# Idiomatic Rust — post-implementation review checklist

**Audience:** Agents and humans after a generation / edit pass.  
**Purpose:** Bridge **recall vs production** for LLMs: the model often “knows” a feature but does not apply it under generation load. This list turns recall into a **mechanical pass** over the diff.

**Layering:** **`docs/AGENT_PROTOCOL.md`** (“Universal principles”) and **`docs/STYLE.md`** §0 state **why** in general language. **This file** spells out **checkable** items — counts, patterns, and lint names — so agents do not need a separate rule per future bug class.

**When to run:** After **`implement`**, before claiming the task done. Required when the change touches **`match` / `if let` / `if`–`else if` chains**, **public API**, or **more than ~10 lines** in one function. Optional for trivial edits if you state **N/A** and one line why.

**Related:** `docs/AGENT_PROTOCOL.md` (inspect / implement, structural erosion); `docs/QUALITY_GATES.md` (Clippy layers).

---

## Output format (required when this checklist applies)

Reply with a short table or bullet list:

| Checklist item | Applied to this diff? | Action taken or N/A reason |
|----------------|----------------------|----------------------------|
| … | yes / no / N/A | … |

---

## Language semantics (official reference)

Skim the relevant section when your diff touches patterns or control flow:

| Topic | URL |
|-------|-----|
| Patterns (chapter) | <https://doc.rust-lang.org/reference/patterns.html> |
| Rest pattern (`..`) | <https://doc.rust-lang.org/reference/patterns.html#r-patterns.rest> |
| Pattern matching (exhaustiveness, etc.) | Same chapter + `match` in reference |

---

## Pattern matching and control flow

1. **`match` with several arms returning the same value** — Can **`..` rest**, **`|` or-patterns**, or a **wildcard arm** collapse duplication? (See [rest pattern](https://doc.rust-lang.org/reference/patterns.html#r-patterns.rest).)

2. **`match x { E::A => true, _ => false }` (single variant)** — Prefer **`matches!(x, E::A)`** when it stays clear (Clippy: `match_like_matches_macro`).

3. **Redundant `if let` / `match` only to test `Option`/`Result`** — Clippy: `redundant_pattern_matching`.

4. **`if let Some(x) = y { … } else { return …; }` (or `Err` / early exit)** — Consider **`let … else`** (workspace may allow `manual_let_else` where readability wins; still **ask** on new code).

5. **`match` that could be a single `if let` or `if`** — Clippy: `single_match_else` / `single_match` (when applicable).

6. **Wildcard combined oddly with `|`** — Clippy: `wildcard_in_or_patterns`.

7. **Nesting deeper than ~2 levels** — Prefer **early return**, **`?`**, or **`let-else`** to flatten (align with `docs/STYLE.md`).

---

## Clippy lints (workspace — see root `Cargo.toml`)

These are **warn** (CI uses `-D warnings` → failure). Fix or document a local `allow` with justification:

| Lint | Typical trigger |
|------|-------------------|
| `match_like_matches_macro` | Boolean `match` → `matches!` |
| `redundant_pattern_matching` | `if let` / `match` only for presence test |
| `single_match_else` | `match` with one non-trivial arm + wildcard |
| `wildcard_in_or_patterns` | `A | _` style that can be simplified |

Further pattern-matching ideas (enable per crate when ready): `needless_match`, `manual_let_else` (currently allowed workspace-wide where `match` reads better).

---

## Architecture-shaped checks (pair with `AGENT_PROTOCOL.md` triggers)

8. **`if` / `else if` chain** — If adding a branch hits **≥ 3** branches total, stop and consider **`match` / enum** (see protocol; do not silently add another `else if`).

9. **Repeated `if let Some(…)`** on the same receiver — Second occurrence in a type/module → consider **a method** or **helper**.

10. **New `Option` field on a struct that already has 2+ `Option`s** — Consider an **enum** or grouped config type.

11. **`Arc<Mutex<…>>` / `Rc<Mutex<…>>` proliferation** — Fourth similar field → **decompose**; see also workspace `rc_mutex` / `arc_with_non_send_sync` warns.

---

## Honest limit

This checklist **does not** replace human architecture calls or product-specific modeling. It **does** force **review-mode** recall onto **generation-mode** output and aligns with mechanical gates in `docs/QUALITY_GATES.md`.
