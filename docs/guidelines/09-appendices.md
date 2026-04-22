# 9. Appendices

[← Review Checklist](./08-review-checklist.md) | [README](./README.md)

---

## Appendix A. Classical Design Principles as Rust Rules

- **SRP** → struct decomposition `[P-009]`, crate boundaries `[P-010]`.
- **OCP** → `#[non_exhaustive]` + trait default methods + minor-version additions.
- **LSP** → trait contracts must hold across all implementors; don't document one behavior and implement another.
- **ISP** → small focused traits (`Display`, `FromStr`, `Iterator`), not god-traits.
- **DIP** → depend on traits, not concrete types, at component boundaries.
- **CRP** → Rust has no inheritance; composition is the only option.
- **DRY** → but use rule of three; premature abstraction costs more than duplication until third occurrence.
- **KISS** → `[M-002]`, `[M-003]`.
- **LoD** → struct decomposition `[P-009]` naturally enforces it.
- **Design by contract** → typestate `[F-001]` for compile-time contracts; `debug_assert!` for runtime.
- **Encapsulation** → module-level privacy; default `pub(crate)`.
- **CQS** → `&self` = query, `&mut self` = command; avoid mutating "queries".
- **POLA** → no `Deref` magic `[A-003]`, no implicit panics, no silent `From` chains that lose information.
- **Single Choice** → enum with `#[non_exhaustive]` is the canonical home for a variant set.

---

## Appendix B. Required Reading

- **The Rust Reference** — normative language semantics.
- **The Rustonomicon** — mandatory before writing any non-trivial `unsafe`.
- **Rust API Guidelines** — naming, trait choice, stability discipline.
- **Ralf Jung's blog** — "Two kinds of invariants" and subsequent posts on unsafe semantics.
- **Async Book** — async runtimes, `Pin`, pinning, cancellation.
- **Rust Unofficial Patterns** — this document's source.

Ecosystem exemplars to study:

- `serde` — optics in practice.
- `tower` — middleware + layers.
- `axum` — extractor pattern for ergonomic handlers.
- `sqlx` — compile-time SQL validation via macros + typestate.
- `hyper` — connector-generic client.
- `tracing` — subscriber-based diagnostic layer.
- `tokio` — runtime internals reference.

---

## Appendix C. Rule Index (for the LLM)

Prefix legend — in order of precedence:

`L` language rules (hard) → `M` meta → `A` anti-patterns → `I` idioms → `P` patterns → `F` functional → `R` modern features.

To resolve a conflict, follow the lower letter. If two rules with the same prefix conflict, the one with the more specific scope wins.

When a user request conflicts with an `L-` or `M-` rule without explicit override, respond by implementing the safe version and explaining the deviation in a brief comment — do not silently produce unsound code.

### Full rule ID list

**Meta-principles** (`M-`):
`M-001` invalid states, `M-002` YAGNI, `M-003` API surface, `M-004` composition, `M-005` explicit seams, `M-006` zero-cost, `M-007` one fact one place.

**Language rules** (`L-`):

- UB: `L-UB-001` fixed list, `L-UB-002` aliasing, `L-UB-003` invalid values, `L-UB-004` alignment, `L-UB-005` const provenance, `L-UB-006` dangling.
- Drop: `L-DROP-001`…`L-DROP-008` (order, temporaries, lifetime extension, partial moves, best-effort, no async).
- Variance: `L-VAR-001`…`L-VAR-004` (table, inheritance, `PhantomData`, drop check).
- Layout: `L-REPR-001`…`L-REPR-007` (default, C, transparent, packed, enum repr, ZST, niche).
- Dyn: `L-DYN-001`…`L-DYN-005` (compatibility, where-Sized, async fn, lifetime defaults, auto traits).
- Coherence: `L-COH-001`…`L-COH-004` (orphan, fundamental, overlap, constrained params).
- Patterns: `L-PAT-001`…`L-PAT-005` (exhaustive, if-let guards, default binding, `@`, or-pattern drop).
- Unsafe: `L-UNSAFE-001`…`L-UNSAFE-008` (SAFETY, unsafe-in-unsafe-fn, docs, narrow, encapsulate, unsafe trait, `&raw`, `MaybeUninit`).
- Special: `L-SPECIAL-001`…`L-SPECIAL-007` (Sized/DST, Send+Sync, Copy≠Drop, Pin, `'static`, `?Sized`, overflow).
- Expressions: `L-EXPR-001`…`L-EXPR-004` (place vs value, assignment, block, `&raw`).
- Items: `L-ITEM-001`…`L-ITEM-005` (const/static, non_exhaustive, must_use, inline, derive order).
- Macros: `L-MACRO-001`…`L-MACRO-003` (declarative vs proc, hygiene, `$crate`).

**Idioms** (`I-`):
`I-001`…`I-016` — borrow target, `format!`, `new`+`Default`, owning collections, RAII, `mem::take`, on-stack dyn, FFI errors, FFI strings in/out, Option iteration, closure captures, `#[non_exhaustive]`, doctest boilerplate, temporary mutability, return consumed arg.

**Design patterns** (`P-`):
`P-001`…`P-014` — Command, Interpreter, Newtype, RAII, Strategy, Visitor, Fold, Builder, struct decomposition, small crates, unsafe containment, custom-trait bounds, FFI object API, FFI type consolidation.

**Anti-patterns** (`A-`):
`A-001`…`A-014` — `.clone()` to silence, `#![deny(warnings)]`, Deref polymorphism, `static mut`, `Box<dyn Error>` in lib, `Arc<Mutex<Vec>>` bus, `unwrap` in tasks, over-broad `pub`, spurious async, `+` for strings, `unwrap` on input, manual close, growing-to-non-dyn, unjustified `allow`.

**Functional concepts** (`F-`):
`F-001` typestate, `F-002` iterator vs for, `F-003` optics, `F-004` `impl Trait`.

**Modern Rust** (`R-`):
`R-001` if-let guards, `R-002` `cfg_select!`, `R-003` `core::range`, `R-004` atomic update, `R-005` let chains, `R-006` async closures, `R-007` async trait, `R-008` `&raw`, `R-009` `LazyLock`/`OnceLock`, `R-010` `MaybeUninit` slice, `R-011` RPITIT, `R-012` precise capturing, `R-013` tooling.

---

[← Review Checklist](./08-review-checklist.md) | [README](./README.md)
