# nebula-macros Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-03

---

## Platform Role

Action, resource, plugin, and credential authors need to implement traits and generate parameter definitions with minimal boilerplate. Procedural macros provide derive(Action), derive(Resource), derive(Plugin), derive(Credential), derive(Parameters), derive(Validator), derive(Config) so that generated code conforms to nebula-action, nebula-resource, nebula-plugin, and nebula-credential contracts. The macros crate is the single source of code generation for these traits; it does not implement domain logic — it only generates code.

**nebula-macros is the proc-macro code generator for Nebula platform traits and parameters.**

It answers: *What derives and attributes are available, and what code do they generate so that engine, runtime, and API accept the output?*

```
Author writes #[derive(Action)] #[action(key="...", name="...", ...)] struct MyAction;
    ↓
Macro expands to impl Action for MyAction { metadata(), ... }
    ↓
Engine/runtime/plugin registry consume the type; no manual impl
```

This is the macros contract: generated code implements the trait and satisfies the contract of the owning crate (action, resource, plugin, credential); attribute set and output shape are stable in patch/minor; breaking changes are major and documented in MIGRATION.md.

---

## User Stories

### Story 1 — Action Author Uses Derive (P1)

Author derives Action with #[action(key, name, description, ...)]; macro generates impl Action. The generated type works with engine and runtime (action registry, execution).

**Acceptance:**
- derive(Action) with required attributes produces compilable impl; metadata and any component refs match action crate contract.
- Attribute changes (additive) in minor; removal or behavior change in major with MIGRATION.

### Story 2 — Resource/Plugin/Credential Authors Use Derives (P1)

Authors derive Resource, Plugin, or Credential with container attributes; macro generates corresponding impl. Generated types work with resource manager, plugin registry, credential manager.

**Acceptance:**
- Each derive documented with required/optional attributes; generated code compiles and satisfies trait.
- Parameters derive produces parameter definitions compatible with nebula-parameter and action metadata.

### Story 3 — Compile-Time Diagnostics (P2)

Invalid attribute or missing required attribute produces a clear compile error (span, message) so authors can fix without reading macro internals.

**Acceptance:**
- Invalid or missing key/name/description (or equivalent) yields actionable error.
- Doc and examples cover common mistakes.

---

## Core Principles

### I. Macros Generate Only; No Domain Logic

**Macros emit code that implements traits and types; they do not implement workflow engine, storage, or credential logic.**

**Rationale:** Domain lives in action, resource, plugin, credential crates; macros are codegen only.

**Rules:**
- No runtime dependency on engine or storage in macro crate; only trait/type dependencies for generated code.
- Forbid unsafe in macro crate (#![forbid(unsafe_code)]).

### II. Output Stability and Compatibility

**Generated code must remain compatible with the trait and contract of the owning crate. Attribute set and expansion output are versioned.**

**Rationale:** Action/plugin/credential/resource crates may evolve; macro output must work with current versions; breaking output = major.

**Rules:**
- Patch/minor: additive attributes or backward-compatible output changes only. No removal of attributes or change of generated signatures without major.
- Contract tests (when added): expand derive, compile with action/resource/plugin/credential, run trait methods.

### III. Clear Compile Errors

**Invalid or missing attributes must produce clear, span-based errors.**

**Rationale:** Proc-macros can be opaque; good diagnostics reduce author friction.

**Rules:**
- Required attribute missing or invalid: emit compile_error! or syn::Error with message and span.
- Document required vs optional attributes in rustdoc and API.md.

---

## Production Vision

In production, all action/resource/plugin/credential authors use nebula-macros (or re-export via nebula-sdk) for derive. Generated code is the primary path for implementing traits; manual impl is supported but not required. From the archives: `nebula-derive.md` (from-archive) and archive-phase-1-core describe parameter derive, action derive, validation, and attribute sets; current crate implements Action, Resource, Plugin, Credential, Parameters, Validator, Config. Production vision: attribute set and output shape are stable; compatibility matrix (macro version X ↔ action/plugin/credential version Y) documented; breaking attribute or output = major + MIGRATION.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Contract tests (macro output + action/resource/plugin/credential) | High | CI: expand, compile, run trait methods |
| Attribute stability doc and deprecation policy | Medium | Document which attributes are stable; deprecation window for removal |
| Diagnostic quality (span, suggestions) | Low | Improve error messages for common mistakes |

---

## Key Decisions

### D-001: Forbid Unsafe in Macro Crate

**Decision:** #![forbid(unsafe_code)] in nebula-macros. No unsafe in expansion or support code.

**Rationale:** Macro crate is trusted by all authors; no FFI or unsafe keeps attack surface minimal.

**Rejected:** Allowing unsafe for "advanced" codegen — not needed for current derives.

### D-002: Single Crate for All Platform Derives

**Decision:** Action, Resource, Plugin, Credential, Parameters, Validator, Config in one crate.

**Rationale:** One version, one place for codegen; authors depend on one macro crate. Simplifies compatibility story.

**Rejected:** Separate crates per trait — would fragment versioning and increase dep tree.

### D-003: Attributes Are Versioned

**Decision:** Attribute set is stable; additive in minor; removal or behavior change in major with MIGRATION.

**Rationale:** Authors rely on attributes; breaking them breaks all downstream crates.

**Rejected:** Unversioned "experimental" attributes that change freely — would break authors.

---

## Open Proposals

### P-001: Expansion Debugging Doc

**Problem:** Authors sometimes need to inspect generated code.

**Proposal:** Document use of cargo expand or similar for macro expansion; add to README or TEST_STRATEGY.

**Impact:** Non-breaking; doc only.

---

## Non-Negotiables

1. **No unsafe in macro crate** — forbid(unsafe_code).
2. **Generated code implements trait and satisfies contract** — action/resource/plugin/credential accept macro output.
3. **Breaking attribute or output = major + MIGRATION.md** — document migration for authors.
4. **Clear compile errors for invalid/missing attributes** — no silent wrong expansion.

---

## Governance

- **PATCH:** Bug fixes (wrong expansion, diagnostics). No attribute or output contract change.
- **MINOR:** Additive attributes; backward-compatible output changes. No removal.
- **MAJOR:** Breaking attribute or generated code. Requires MIGRATION.md and compatibility matrix update.
