# Decisions

## D-001: Forbid Unsafe in Macro Crate

**Status:** Adopt

**Context:** Macro crate is in the dependency tree of all authors; safety and trust.

**Decision:** #![forbid(unsafe_code)] in nebula-macros. No unsafe in expansion or support code.

**Alternatives considered:** Allow unsafe for "advanced" codegen — rejected; not needed for current derives.

**Trade-offs:** No FFI or low-level tricks; keeps audit and security simple.

**Consequences:** Any future need for unsafe would require a separate crate or explicit justification.

**Migration impact:** None.

**Validation plan:** CI or audit: no unsafe in crate.

---

## D-002: Single Crate for All Platform Derives

**Status:** Adopt

**Context:** Where to put Action, Resource, Plugin, Credential, Parameters, Validator, Config derives.

**Decision:** One crate (nebula-macros) for all; authors depend on one macro crate.

**Alternatives considered:** Separate crate per trait — rejected to avoid version fragmentation and many deps.

**Trade-offs:** Crate size and coupling to multiple trait crates; benefit is one version and one compatibility story.

**Consequences:** When any trait (action/plugin/credential/resource) has breaking change, macro crate may need release to align; document in compatibility matrix.

**Migration impact:** None.

**Validation plan:** Contract tests with each trait crate.

---

## D-003: Attributes Are Versioned

**Status:** Adopt

**Context:** Authors rely on attribute names and behavior.

**Decision:** Attribute set is stable; additive in minor; removal or behavior change in major with MIGRATION.

**Alternatives considered:** Unversioned "experimental" attributes — rejected to avoid silent breakage.

**Trade-offs:** We cannot freely change or remove attributes; must document and deprecate.

**Consequences:** Breaking attribute or output = major; MIGRATION.md for authors.

**Migration impact:** When we break, authors must update attributes and follow MIGRATION.

**Validation plan:** Doc and CI: attribute list and stability documented; contract tests lock output shape.
