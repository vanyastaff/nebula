# Agent protocol (canonical)

**Authority:** This file is the single source of truth for the repository meta-protocol.  
**Audience:** Any automated or human agent working in this workspace (Claude Code, Cursor, Codex, etc.).  
**Related:** `.cursor/rules/00-meta-protocol.mdc` enforces this file; `CLAUDE.md` and `AGENTS.md` point here. Post-edit pattern pass: **`docs/IDIOM_REVIEW_CHECKLIST.md`**; layered gates: **`docs/QUALITY_GATES.md`**.

---

## Universal principles (read first)

These apply to **every** task. They are intentionally **general**: they state *what kind of judgment* the project expects, not an exhaustive list of situations. **Normative product and house rules** live in **`docs/PRODUCT_CANON.md`** and **`docs/STYLE.md`**; this section only orients agents so those documents stay the single source of detail.

1. **Authority stack.** **Canon and Nebula docs beat generic advice.** If Rust books or web examples disagree with `PRODUCT_CANON.md` or `STYLE.md`, follow Nebula. The **crate graph** is binding (`deny.toml`); do not add dependencies that break layers.

2. **Ownership and surface.** Discover **who owns** a concept before adding or renaming **public** types (`docs/GLOSSARY.md`, naming table in `STYLE.md` §3). Parallel types with overlapping meaning are an **ADR** decision, not a silent workaround.

3. **Shape follows requirements, not only the smallest diff.** When **context around a decision has grown** (more cases, more optional combinations, more parameters, more shared locking), the **representation** may need to change — not just another branch on the old shape. **Locally minimal edits** are safe for small, stable spots; they are risky when they **accumulate** structural debt. If you are about to extend a form that already feels crowded, **stop**, name the tension, and either propose a refactor (and wait where needed) or justify why the existing shape still matches the domain.

4. **Rust 1.95+ as a professional default.** Pin and behavior follow **`rust-toolchain.toml`**. For **language semantics**, prefer the **Rust Reference**; for **idioms and trade-offs**, *Rust Design Patterns* and **`STYLE.md`** §1–2. Prefer **explicit types** (enums, newtypes) over stringly APIs and boolean flags where the domain is finite — see `STYLE.md` antipatterns.

5. **Evidence and calibration.** Claims about the codebase require **tool output** (see below). Mark **`verified:`** / **`documented:`** / **`hypothesis:`** honestly.

6. **Mechanics support principles — they do not replace them.** Inspect/implement, git history on hot spots, post-edit checklist, and Clippy exist to make **good habits reliable**, not to encode every future design choice. **Thresholds and pattern checks** are spelled out in **`docs/IDIOM_REVIEW_CHECKLIST.md`** and **`docs/QUALITY_GATES.md`**.

---

## Meta-protocol (verbatim)

Rules for any agent operating in this repo:

- **Evidence before assertion.** Any claim of the form "X is used / not
  used", "Y is a duplicate", "Z is re-exported", "this is dead code", "no
  one calls this" MUST be accompanied by tool output (rg, Serena LSP,
  cargo check) with paths and line numbers in the same response. Claims
  without evidence are forbidden — mark them as `hypothesis:` if
  speculation is genuinely needed.

- **Re-exports are not invisible.** Before claiming a type/function is
  unused, grep for `pub use.*<Name>` and check every `mod.rs` / `lib.rs`
  in the reverse dependency chain. State explicitly that re-exports were
  checked.

- **Macros count.** When checking usage of a type, also grep for macros
  that construct it (e.g. `credential_key!` for `CredentialKey`).

- **Glossary-first ownership — discover before you add public surface.**
  `docs/GLOSSARY.md` maps **which crate owns** each named symbol; read the
  relevant section before adding or renaming public types. For patterns
  in `docs/STYLE.md` §3 (`*Metadata`, `*Schema`, `*Key`, `*Id`, …), search
  the workspace (`rg`, crate `README`, `lib.rs`) for an existing type with
  the same role. If the glossary assigns ownership to a crate (e.g. shared
  handles in `nebula-core`, execution types in `nebula-execution`), extend
  or use that crate’s types unless an ADR defines a new boundary. Adding a
  parallel type with overlapping meaning in a different crate requires an
  ADR and a clear non-overlapping contract — not ad-hoc duplication.

- **Same name ≠ same type.** When a public name appears in multiple
  crates, treat it as a potential invariant violation until proven
  intentional by an ADR. Report it, do not resolve it silently.

- **Migration state is real.** If an ADR is `status: migration-in-progress`
  or `superseded-by`, the code is expected to contain transitional
  artifacts. Do not report these as bugs; do not declare the migration
  complete without verifying every affected symbol.

- **Canon beats intuition; code beats canon.** If /docs disagrees with
  reality, report the gap — do not paper over it with a plausible
  narrative.

- **Confidence calibration.** Distinguish `verified:` (tool output in
  this response), `documented:` (cited from /docs), `hypothesis:`
  (informed guess, needs checking). Never mix these tiers silently.

- **Scope discipline for audits.** When asked "is X a duplicate / problem
  / bug", answer exactly that question. Do not expand to a general
  architecture review unless asked.

- **Layers, ownership, and SOLID / SRP.** Crates are organized in **one-way
  layers**; the binding graph is `deny.toml` and the product rules are
  `docs/PRODUCT_CANON.md`. Do not add dependencies that violate those
  boundaries. **Single Responsibility (SRP):** a crate or module should
  have **one coherent reason to change** — aligned with its layer and with
  `docs/GLOSSARY.md` type ownership; avoid mixing unrelated concerns in one
  module or bypassing the owning crate with a parallel type. **Other SOLID
  ideas** (open/closed at trait seams, substitutability of trait impls,
  narrow trait surfaces, depend on abstractions at boundaries) apply where
  they match Rust idioms (traits, `newtype`, composition). External
  reference for names and intuition:
  [Rust Design Patterns — Design principles](https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html).
  If a change would satisfy a “principle” but break `PRODUCT_CANON.md` or
  `deny.toml`, **stop** — canon and the dependency graph win; revise via ADR
  if the architecture should change.

- **Implementation quality — maintainable, not “first compile”.** Changes
  should read like **review-ready production code**: correct error handling
  (`Result`, typed errors per `docs/STYLE.md`), respect crate boundaries and
  secrets rules, and add or update tests when behavior changes. Read
  surrounding code and match existing patterns. Before choosing an API or
  abstraction, consult **official Rust docs** as needed
  ([`doc.rust-lang.org`](https://doc.rust-lang.org/) — `std`, *The Book*,
  *Reference* for semantics) and the **Rust Design Patterns** book for
  idioms and trade-offs
  ([introduction](https://rust-unofficial.github.io/patterns/intro.html),
  [design principles](https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html)).
  Nebula’s **`docs/STYLE.md`** and **`docs/PRODUCT_CANON.md`** override generic
  advice when they conflict. Prefer clarity and explicitness over clever
  one-liners; if a shortcut is tempting, name the trade-off or use the
  simpler structure that fits the codebase.

- **Recall vs production (LLMs).** A model may explain a language feature
  correctly (e.g. [rest patterns](https://doc.rust-lang.org/reference/patterns.html#r-patterns.rest)
  in the Rust Reference) yet omit it while generating code. **Mitigations:**
  Clippy (see `docs/QUALITY_GATES.md`), the post-implementation pass below, and
  concrete rules — not vague “write idiomatically” instructions.

- **Inspect before implement (two modes).** Default: start in **inspect**
  mode — **no code edits** until you output (1) what you will change, (2)
  whether the **current shape** of the code still fits that change, (3) if the
  shape looks **stale**, what refactor you propose (or `N/A`). Allowed in
  inspect: read full files, neighbors, `rg`, **`git log` / blame** on hot
  paths. Move to **implement** only after explicit user **go** (or equivalent).
  **Escape hatch:** change **≤ 5 lines** and **no control-flow** change (no new
  branches, no new public surface) — you may skip inspect; **state in the PR /
  task output** that inspect was skipped and why.

- **Structural erosion (stop; do not silently extend).** Align with
  **Universal principles §3**: before you apply the smallest patch that
  “only adds one more case”, check whether the **current structure** still fits
  the **accumulated** requirements. If complexity has crossed a line, **stop**,
  record that in your output, and either (a) propose a refactor and wait for
  approval where appropriate, or (b) give a **short justification** why the
  existing shape remains valid. **Concrete stop conditions and examples**
  (counts, patterns, Clippy names) are in **`docs/IDIOM_REVIEW_CHECKLIST.md`**
  (architecture-shaped checks) — use that file so this protocol stays **principle-first**.

- **Git history on incremental growth.** Before editing a large function or
  module that clearly grew piecemeal, run
  `git log -n 15 --follow -- path/to/file` (and optionally `git blame` on the
  region). If recent history is mostly incremental edits and you are adding
  another increment, flag **erosion risk**: original implied scope vs current
  scope, and whether consolidation belongs in this task or a follow-up.

- **Post-implementation review pass.** After coding, run
  **`docs/IDIOM_REVIEW_CHECKLIST.md`** against your diff: which items applied,
  what you changed, what is N/A. **Required** when the change touches
  `match` / `if let` / `if` chains, **public API**, or **> ~10 lines** in one
  function.

- **Periodic architecture audit (separate ritual).** Do not bundle full-crate
  refactors into every feature PR unless asked. For accumulated drift, use a
  **separate** session: walk a crate, list refactor candidates, **do not
  implement** until approved.
