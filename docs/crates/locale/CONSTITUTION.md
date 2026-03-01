# nebula-locale Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula serves users in multiple languages and regions. API errors, validation messages, and UI strings should be localized. A single locale crate provides locale negotiation, translation bundle management, and localized formatting so that API, runtime, and action surfaces can render user-visible text consistently.

**nebula-locale is the planned localization and internationalization layer.**

It answers: *How is user locale resolved (e.g. from request or config), how are translation bundles loaded and looked up, and how do crates format user-visible messages and errors?*

```
Request or context carries locale (e.g. Accept-Language, tenant setting)
    ↓
Locale negotiation and fallback (e.g. en-US → en → default)
    ↓
Translation lookup by message key; localized formatting (date, number)
    ↓
API/runtime/action return localized strings
```

Contract: centralized locale negotiation; deterministic fallback; stable message keys. Crate is planned; not yet implemented.

---

## User Stories

### Story 1 — API Returns Localized Error (P1)

API validates request; validation fails. Response message is in user's language (e.g. from Accept-Language or tenant locale). Same error code, different message text.

**Acceptance**: Locale resolved from request; message key → translation; fallback chain documented.

### Story 2 — Stable Message Keys (P1)

Translations are keyed by stable message ID. Adding a new locale or changing wording does not change keys. Minor = additive keys; major = key or interpolation break with MIGRATION.

**Acceptance**: Message key stability; interpolation (e.g. {name}) documented; no breaking key in minor.

### Story 3 — Cross-Crate Contract (P2)

API, validator, action, runtime use same locale contract: resolve locale, lookup(key), format. No duplicate locale logic per crate.

**Acceptance**: Locale crate owns resolution and lookup; other crates call in; error and UI message keys documented.

---

## Core Principles

### I. Centralized Locale Negotiation

**One place for locale resolution and fallback. API and runtime do not implement their own.**

**Rationale**: Consistent behavior; single place to fix and extend.

### II. Deterministic Fallback

**Same Accept-Language and catalog ⇒ same chosen locale and message.**

**Rationale**: Testability and predictability.

### III. No Business Logic in Locale Crate

**Locale crate does not implement workflow, auth, or validation. It only resolves locale and looks up/format messages.**

**Rationale**: Single responsibility.

---

## Production Vision

Locale negotiation from request/tenant; translation bundles (fluent-style or key-value); localized formatting for dates/numbers; stable message keys. API and UI consume same contract. From archives: fluent-style translations, localized error rendering. Gaps: implement crate; integration with API/validator/action.

### Key gaps

| Gap | Priority |
|-----|----------|
| Implement crates/locale | Critical |
| Translation bundle format and loading | High |
| API/validator integration | High |
| Message key registry and coverage | Medium |

---

## Non-Negotiables

1. **Centralized locale and translation** — one contract for API/runtime/action.
2. **Stable message keys** — additive in minor; break only in major.
3. **Deterministic fallback** — same input ⇒ same locale and message.
4. **Breaking message key or interpolation = major + MIGRATION.md**.

---

## Governance

- **MINOR**: Additive locale/catalog support.
- **MAJOR**: Message-key or interpolation semantic breaks; MIGRATION.md required.
