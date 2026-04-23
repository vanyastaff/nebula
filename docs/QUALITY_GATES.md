# Quality gates (layered)

This document is the **single** human-oriented description of Nebula’s mechanical quality system. Normative product rules remain in `docs/PRODUCT_CANON.md` and satellites; **this file** is only about **how** we enforce code quality in the toolchain.

## Canonical external sources (citation index)

Before adding or tightening a rule, tie it to **at least one** of:

| Name | URL | One-line summary |
|------|-----|------------------|
| *Rust Design Patterns* — Introduction | https://rust-unofficial.github.io/patterns/intro.html | Idioms vs patterns vs anti-patterns; Rust is not classic OOP. |
| *Rust Design Patterns* — Design principles | https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html | SOLID, DRY, KISS, Law of Demeter, etc., as vocabulary. |
| *Rust Design Patterns* — Idioms | https://rust-unofficial.github.io/patterns/idioms/index.html | Community idioms; prefer simple, readable code (KISS). |
| *Rust Design Patterns* — Patterns | https://rust-unofficial.github.io/patterns/patterns/index.html | Reusable solutions; YAGNI reminder. |
| *Rust Design Patterns* — Anti-patterns | https://rust-unofficial.github.io/patterns/anti_patterns/index.html | Counter-examples; ineffective or risky “solutions”. |
| Rust Reference — `unsafe` keyword | https://doc.rust-lang.org/reference/unsafe-keyword.html | Unsafe blocks/functions discharge proof obligations; unsafe ops need documented reasoning. |
| Rust API Guidelines — Checklist | https://rust-lang.github.io/api-guidelines/checklist.html | Full public-API checklist (C-* ids). |
| Rust API Guidelines — C-GOOD-ERR | https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err | Error types implement `std::error::Error`, `Send` + `Sync`; avoid useless `()` errors; meaningful types at boundaries. |
| Rust API Guidelines — C-SEND-SYNC | https://rust-lang.github.io/api-guidelines/interoperability.html#c-send-sync | Types should be `Send`/`Sync` where possible; thread-safety matches reality. |
| Rust Reference — Patterns | https://doc.rust-lang.org/reference/patterns.html | Syntax and semantics of patterns (`match`, `if let`, `..` rest pattern, exhaustiveness). |
| Rust Reference — Rest pattern (`..`) | https://doc.rust-lang.org/reference/patterns.html#r-patterns.rest | Rest pattern matches remaining fields/elements; relevant to collapsing duplicate match arms. |
| *Rust Expert Style Guide* (repo, LLM) | `docs/RUST_EXPERT_STYLE_GUIDE.md` → `docs/guidelines/` | Optional behavioral contract: rule IDs `L-`/`M-`/`I-`/…, Reference-leaning language rules; **subordinate** to `PRODUCT_CANON` / `STYLE`. |

**Clippy lint pages** (mechanization — layer 2) are linked from compiler output, e.g.  
https://rust-lang.github.io/rust-clippy/stable/index.html — use the `#lint-name` anchor for the exact version pinned in `rust-toolchain.toml`.

## Layer model (preference order)

1. **rustc / Clippy** — cannot ship if CI fails (`cargo clippy … -- -D warnings`).
2. **`cargo deny`** — dependency policy (`deny.toml`); already in `lefthook.yml` on manifest changes.
3. **`xtask`** — checks Clippy cannot express (grep/heuristic); **must** print a citation URL or repo path on failure.
4. **`.cursor/rules/*.mdc`** — only where (1)–(3) cannot encode the invariant; each such rule must state a **mechanization path** (see `docs/AGENT_PROTOCOL.md`).

## What is already enforced (audit snapshot)

**Observable in repo:**

- **Workspace lints:** `Cargo.toml` → `[workspace.lints]` — `clippy::pedantic`, `clippy::nursery`, `clippy::cargo` at **warn**, with a large **allow** list for known noise; **`-D warnings`** in CI turns every warn into failure (`/.github/workflows/ci.yml` `clippy` job).
- **`clippy.toml`:** thresholds, MSRV, test allows for unwrap/dbg in tests, `mem_forget = deny`, doc idents, etc.
- **`rustfmt.toml`:** nightly options; CI `fmt` job runs `cargo +nightly fmt --check`.
- **`deny.toml`:** crate graph bans.

Do **not** duplicate those knobs elsewhere — extend them in place and update this doc.

## Junior markers — mechanized vs debt

| Marker | Mechanization | Citation (why) |
|--------|----------------|----------------|
| Panic via `unwrap`/`expect` in library paths | **Debt:** `clippy::unwrap_used` / `expect_used` (restriction) would fire thousands of times today; enable in `[workspace.lints]` only after burn-down. | STYLE / canon prefer typed errors; *Idioms* + KISS favor explicit handling — see patterns intro. |
| `dbg!` in non-test code | **Clippy** `dbg_macro` (warn → error under `-D warnings`) once enabled workspace-wide. | Anti-debug shipping; aligns with dependability. |
| `todo!` / `unimplemented!` in shipped code | **Clippy** `todo` / `unimplemented` when enabled; macro-generated `todo!` in derives needs allow at site. | *Patterns* YAGNI / unfinished work should not masquerade as done (`patterns/index.html`). |
| `Rc<Mutex<T>>` / suspicious `Arc<Mutex<T>>` | **Clippy** `rc_mutex`, `arc_with_non_send_sync` (warn). | **C-SEND-SYNC** — threading errors must match real safety; `Mutex` inside `Rc` is a classic footgun (see lint docs). |
| `unsafe` without local reasoning | **Clippy** `undocumented_unsafe_blocks` (configure in `clippy.toml`); convention: `// SAFETY:` immediately above block. | **Rust Reference** — unsafe blocks assert obligations are discharged (`unsafe-keyword.html` § unsafe blocks). |
| `Box<dyn Error>` in **public** API | **`xtask check-junior`** (grep heuristic). | **C-GOOD-ERR** — errors should be concrete, `Error + Send + Sync`, not opaque trait objects at crate boundary. |
| Duplicate stable type names across crates | **`xtask check-surface`**. | Workspace observation + glossary ownership (`docs/GLOSSARY.md`); catches accidental parallel `*Key` types. |
| ADR `status: migration-in-progress` vs code | **`xtask check-adr-sync`** (best-effort YAML front matter). | Process integrity — transitional state must stay traceable. |

## Commands

| Command | Meaning |
|---------|---------|
| `cargo lint` | `cargo clippy --workspace --all-targets -- -D warnings` (alias in `.cargo/config.toml`). |
| `cargo xtask check-junior` | Heuristic grep checks (see `xtask`). |
| `cargo xtask check-surface` | Cross-crate `pub` name collisions (heuristic). |
| `cargo xtask check-adr-sync` | ADR front matter sanity. |
| `cargo quality` | `fmt --check` + `clippy -D warnings` + all `xtask` checks. |
| `cargo precommit` | `cargo quality` + `cargo nextest run --workspace --profile ci --no-tests=pass` (full — use CI or narrow locally when iterating). |

**Test libraries and nextest profiles** (not a mechanical gate) are described in `docs/TESTING.md`.

**Pre-commit / push:** `lefthook.yml` runs fast gates; full `cargo quality` is mirrored in **`.github/workflows/quality.yml`** on PRs.

## How to add a new gate

1. Find a citation in the table above (or add a new row with URL).
2. Prefer **Clippy** or **rustc** — add to `Cargo.toml` `[workspace.lints]` or `clippy.toml`, run `cargo clippy --workspace -- -D warnings`, fix or allow with **file-level** justification.
3. If Clippy cannot express it, add **`xtask`** check with a **printed** citation on failure.
4. Update this file’s tables — **no** orphan scripts.

## Honest limits

- **Semantics** (“parse, don’t validate” at boundaries) are not fully lintable; need review + ADRs.
- **Architecture fit** (SOLID, SRP) — partially reflected in layers (`deny.toml`) and glossary ownership; full judgment is human.
- **Locally-optimal diffs / “sixth `else if`”** — not fully lintable; use **`docs/AGENT_PROTOCOL.md`** (inspect/implement, count triggers, git history) and **`docs/IDIOM_REVIEW_CHECKLIST.md`** after edits.
- **Many Clippy lints intentionally `allow`** — CI green does not mean “ignore the lint’s intent” on **new** code; see **§ Intentionally allowed Clippy** below and **`docs/STYLE.md`** §0.
- **Heuristic** tools (`check-surface` limits to `*Key`/`*Id` names; `check-junior` only `Box<dyn Error>`) — tune in `xtask/src/main.rs` with citations.

---

## Summary (what is mechanical vs not)

### Junior markers now caught mechanically

- **Pedantic + nursery + cargo** Clippy groups at **warn**, CI **`-D warnings`** (existing `/.github/workflows/ci.yml`).
- **`mem_forget` = deny** (`clippy.toml`).
- **`dbg_macro`, `rc_mutex`, `arc_with_non_send_sync`** at **warn** (root `Cargo.toml` — cites **C-SEND-SYNC** and Clippy lint IDs in comments above the block).
- **Pattern-matching recall** — `match_like_matches_macro`, `redundant_pattern_matching`, `single_match_else`, `wildcard_in_or_patterns` at **warn** (root `Cargo.toml`; human checklist `docs/IDIOM_REVIEW_CHECKLIST.md`).
- **`xtask check-junior`**: `pub fn` + `Box<dyn Error>` trait object (API Guidelines **C-GOOD-ERR**).
- **`xtask check-surface`**: duplicate `pub struct`/`enum` names ending in **`Key` or `Id`** across packages (glossary-adjacent collisions).
- **`xtask check-adr-sync`**: ADR front matter with `migration-in-progress` should mention `affects-symbols` (when that status appears).
- **`cargo deny`** on manifest changes (lefthook).
- **`rustfmt`** nightly — `rustfmt.toml` (style guide: https://doc.rust-lang.org/style-guide/ ).

### Still relying on human / agent discipline (honest list)

- **`unwrap` / `expect` in library code** — **not** workspace-denied yet: enabling `clippy::unwrap_used` would require a large burn-down (see table above). **Mechanization path:** enable `unwrap_used` / `expect_used` in `[workspace.lints]` after debt hits zero; until then pedantic review.
- **`todo!` / `unimplemented!`** — not globally denied (macro-generated / doc examples). **Mechanization path:** deny in `[workspace.lints]` with targeted `allow` in known macro sites.
- **Full duplicate `pub` names** (not only `*Key`/`*Id`) — **Mechanization path:** extend `xtask check-surface` with configurable allowlist / richer rustdoc/AST (rust-analyzer or `syn` in xtask).
- **`.clone()` density / trait method count (ISP)** — not implemented (heuristic cost vs value). **Mechanization path:** custom Clippy lint or `dylint` rule with citations.
- **Unsafe without `SAFETY:`** — rely on **`undocumented_unsafe_blocks`** in Clippy + `clippy.toml` (not duplicated in `xtask` to avoid false positives on one-line `unsafe { ... }` patterns).

### Intentionally allowed Clippy (workspace `allow`) — universal policy

Several lints in **`Cargo.toml`** `[workspace.lints.clippy]` are set to **`allow`** **not** because Nebula rejects what they encourage, but because **`warn` would force large or noisy churn** across existing code (style taste, macro sites, API shape, or legacy patterns). That applies broadly — not to one feature such as `let-else`.

**Universal rule for agents and reviewers:** on **new** and **heavily touched** code, still follow the **spirit** of those lints where it improves clarity, safety, or alignment with **`docs/STYLE.md`** and the **Rust Reference** — even when CI does not fail. Use **`docs/IDIOM_REVIEW_CHECKLIST.md`** as the concrete pass; individual checklist items are **examples** of that spirit, not an exhaustive list of lints.

**Mechanization path (any such lint):** workspace **`warn`** after an explicit **burn-down**; **`warn`** only in crates that opt in; or a targeted lint crate / `dylint` on changed paths. **`Cargo.toml`** comments note tradeoffs for specific allows where useful.

### Protocol vs checklist (reviewers)

**`docs/AGENT_PROTOCOL.md`** (universal principles + verbatim rules) and **`docs/IDIOM_REVIEW_CHECKLIST.md`** (checkable items) **overlap in intent by design**: principles say *what kind of judgment*; the checklist says *what to verify on a diff*. They must **not** contradict each other. When you change wording in one file, **skim the other** and align terminology (layers, erosion, inspect/implement, glossary ownership, API shape, pattern style, error handling) so future PRs do not inherit drift.

### Suggested next mechanization targets

1. Burn down `unwrap` in non-test `src/`, then enable **`clippy::unwrap_used`** (restriction) with **Rust patterns / Idioms** + STYLE alignment.
2. Add **`cargo-semver-checks`** or similar for public API breaks (links to API Guidelines stability story).
3. Replace grep `xtask` checks with **`syn`**-based parsing for fewer false positives (same citation strings in error messages).
