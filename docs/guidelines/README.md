# Rust Expert Style Guide (`docs/guidelines/`)

**Target reader: an LLM generating Rust code.** You are the reader. Use this document set as a behavioral contract, not as prose to quote.

**Toolchain pin:** match **`rust-toolchain.toml`** in this repo (Rust **1.95+**, **Edition 2024**).

**Authority in Nebula:** **`docs/PRODUCT_CANON.md`**, **`docs/STYLE.md`**, **`docs/GLOSSARY.md`**, and **`deny.toml`** **override** this guide when they conflict. This set does not define product layers or integration rules. For **Nebula-specific** agent workflow (inspect/implement, structural erosion, CI `allow` policy), use **`docs/IDIOM_REVIEW_CHECKLIST.md`** and **`docs/QUALITY_GATES.md`**.

**Shortcut from repo root:** [`../RUST_EXPERT_STYLE_GUIDE.md`](../RUST_EXPERT_STYLE_GUIDE.md).

---

**Target language level: Rust 1.95+, Edition 2024.** Assume `async fn` in traits (1.75+), `&raw` (1.82+), Edition 2024 + async closures + `AsyncFn*` (1.85), let chains (1.88), `LazyLock`/`OnceLock`, `if let` guards + `cfg_select!` + `core::range::Range`/`RangeInclusive` + atomic `update`/`try_update` (1.95).

**Sources:** Synthesized from Rust Unofficial Patterns (idioms, patterns, anti-patterns, functional usage) and The Rust Reference (language semantics, UB catalog, drop order, variance, layout, coherence, dyn compatibility).

---

## File layout

| File | Section | Scope |
|------|---------|-------|
| [01-meta-principles.md](./01-meta-principles.md) | `M-` | Philosophy, YAGNI, composition, explicit seams |
| [02-language-rules.md](./02-language-rules.md) | `L-` | **Hard rules.** UB, references, drop, variance, layout, dyn, coherence, patterns, unsafe, special types |
| [03-idioms.md](./03-idioms.md) | `I-` | Borrow types, `new`+`Default`, RAII, FFI, closures |
| [04-design-patterns.md](./04-design-patterns.md) | `P-` | Command, Newtype, RAII, Strategy, Visitor, Fold, Builder, decomposition, FFI |
| [05-anti-patterns.md](./05-anti-patterns.md) | `A-` | `.clone()`-to-compile, `Deref`-inheritance, `static mut`, `Box<dyn Error>` |
| [06-functional-concepts.md](./06-functional-concepts.md) | `F-` | Typestate, iterator chains, optics, `impl Trait` |
| [07-modern-rust.md](./07-modern-rust.md) | `R-` | 1.75–1.95 features: async trait/closures, let chains, `if let` guards, `cfg_select!`, precise capturing |
| [08-review-checklist.md](./08-review-checklist.md) | — | Auditable checklist citing rule IDs |
| [09-appendices.md](./09-appendices.md) | — | SOLID mapping, required reading, operational guidance |

---

## Rule-ID system

Every rule has an ID `[X-NNN]` where `X` identifies the category:

- `M-` **Meta-principles** — philosophy
- `L-` **Language rules** — correctness-critical, from the Reference
- `I-` **Idioms** — community conventions
- `P-` **Design patterns** — problem-solution templates
- `A-` **Anti-patterns** — known-bad solutions to avoid
- `F-` **Functional concepts** — patterns borrowed from functional languages
- `R-` **Modern Rust** — features from 1.75–1.95

---

## Operational semantics (how to obey these rules)

1. **`L-` rules are hard constraints.** Violating one produces unsound or undefined code. Never emit code that violates an `L-` rule unless the user explicitly requests unsafe code *and* you supply a `// SAFETY: …` comment documenting the invariant upheld.
2. **`M-` and `A-` rules are strong defaults.** Follow them unless the user's stated goal contradicts them.
3. **`I-`, `P-`, `F-`, `R-` rules are preferences.** Apply when the situation matches; skip when the task doesn't need them.
4. **Good/Bad examples are prescriptive.** Emit code that matches the `// Good` pattern; do not emit code that matches `// Bad`.
5. **When multiple rules apply, the lower letter wins** (`L > M > A > I > P > F > R`).
6. **If asked to explain a choice, cite the rule ID.** Example: "I used `LazyLock` per `[A-004]`."

### Directive verbs (fixed meanings)

- **MUST / NEVER** — hard rule. No exceptions in generated code.
- **PREFER / AVOID** — strong default. Deviation requires an inline comment justifying it.
- **MAY** — legitimate option among alternatives.

---

## Conflict resolution

When a user request conflicts with an `L-` or `M-` rule without explicit override, produce the safe version and explain the deviation in a brief comment. Do not silently generate unsound code.

If two rules with the same prefix conflict, the one with the more specific scope wins. If unresolved, the `L-` / `M-` rule always wins over `I-`/`P-`/`F-`/`R-`.

---

## Quick reference by concern

| Task | Primary rules |
|------|---------------|
| Writing `unsafe` | `[L-UNSAFE-001]`…`[L-UNSAFE-008]`, `[L-UB-*]`, `[P-011]` |
| FFI boundary | `[L-REPR-002]`…`[L-REPR-005]`, `[I-008]`…`[I-010]`, `[P-013]`, `[P-014]` |
| Async code | `[L-DROP-008]`, `[L-DYN-003]`, `[R-006]`, `[R-007]`, `[A-007]`, `[A-009]` |
| Library error design | `[A-005]`, `[I-016]`, `[L-ITEM-002]` |
| Collections / smart pointers | `[L-VAR-003]`, `[L-VAR-004]`, `[I-004]`, `[A-003]` |
| State machines | `[M-001]`, `[F-001]`, `[P-008]` (typestate builder) |
| Global state | `[A-004]`, `[L-ITEM-001]`, `[R-009]` |
| `Drop` resources | `[L-DROP-001]`…`[L-DROP-008]`, `[I-005]`, `[P-004]` |

---

Navigation: start with [01-meta-principles.md](./01-meta-principles.md).
