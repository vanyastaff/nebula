# ADR-0058: Schema field UI vocabulary (`#[field(...)]` standard)

**Status:** Proposed (2026-05-14)
**Tags:** schema, derive, ui, form-rendering

## Context

Charter F7: *"Schema is the form."* Same `#[derive(Schema)]` definition
serves three audiences: type safety, runtime validation, UI form
generation.

Current `nebula-schema` already implements `#[field]` attribute (see
`crates/schema/macros/`). This ADR **standardizes the closed
vocabulary** of attribute keys to prevent silent typos and ad-hoc
extension by individual plugin authors.

## Decision

### Closed `#[field(...)]` vocabulary

15 keys, all closed-set, `#[non_exhaustive]` enum on internal
representation:

```rust
#[field(
    // === Identity ===
    label = "Email Address",        // human-readable name
    description = "Your account email",
    placeholder = "user@example.com",

    // === Behavior hints (drive widget choice) ===
    secret = true,                  // → password input
    multiline = false,              // → textarea
    readonly = false,
    advanced = false,               // → "show advanced" toggle

    // === Constraints (drive validation + UI) ===
    options = ["GET", "POST", "PUT"],   // → dropdown / radio
    min = 0, max = 100,                 // numeric range or string length
    pattern = "[a-z]+",                 // regex constraint

    // === Layout ===
    section = "Authentication",     // grouping
    order = 1,                      // explicit ordering

    // === Conditional visibility ===
    when = "method == 'POST'",      // see "when expression form" below

    // === Custom rendering reference ===
    widget = "cron",                // closed enum reference, NOT extensible
)]
field_name: FieldType,
```

Unknown keys = compile error with Levenshtein-distance suggestion.

### `when` expression form — pinned

**Decision:** structured attribute syntax, not embedded mini-language.

```rust
#[field(when_eq(method = "POST"))]                  // equality
#[field(when_in(method = ["POST", "PUT"]))]         // membership
#[field(when_ne(auth_kind = "none"))]               // inequality
#[field(when_gt(items_length = 0))]                 // numeric
```

Reasoning (Niko Matsakis Day 6 morning): parsing arbitrary expression
strings in proc-macro multiplies error-message complexity. Structured
attributes give compiler-friendly diagnostics and stay parseable.

Cart's prettier `when = "method == 'POST'"` form rejected for v1.x;
revisit if structured form proves too verbose in practice.

### Closed `Widget` enum

```rust
#[non_exhaustive]
pub enum Widget {
    Text, TextArea, Password, Number, Slider, Checkbox,
    Radio, Dropdown, MultiSelect, DateTime, Cron, Json,
    JsonPath, Code, ColorPicker, FilePicker, Markdown,
}
```

~17 entries. Growth via minor bumps. **No registry, no extensibility.**
(Per F12 — author-side custom widgets dropped for security + UX
consistency.)

### Closed `InputHint` enum

Already exists in `nebula-schema::input_hint`. ~20 entries.
(Email, Url, Password, Phone, Ip, Regex, Markdown, Cron, Date,
DateTime, Time, Color, Duration, Uuid, …)

`InputHint` is a **second axis** orthogonal to `Field` type:
`Field::String` + `InputHint::Email` = email input. `Field::String` +
`InputHint::Cron` = cron expression input. Closed set, semver-disciplined
growth.

### Closed `Format` vocabulary

JSON Schema 2020-12 standard formats only: `email`, `uri`, `ipv4`,
`ipv6`, `date-time`, `date`, `time`, `duration`, `uuid`, `regex`,
plus Nebula extensions: `cron`, `jsonpath`, `semver-range`. ~15 entries.

## Consequences

### Positive

- Vocabulary stable — author can grep documentation for full list.
- Typos compile-error with suggestions.
- UI editor can rely on finite set of widget kinds — no third-party
  widget injection vector.
- JSON Schema export uses standard `format` field where possible,
  `x-nebula-*` annotations for Nebula-specific extensions.

### Negative

- Authors blocked from custom widgets (per F12 decision). Niche cases
  (e.g., proprietary CRM picker) handled by mapping to existing
  widget + custom validation.
- Vocabulary growth requires ADR amendment + minor bump — slower than
  open registry.

### Neutral

- Compile-time enforcement removes a class of editor-side runtime
  errors.

## References

- Conference Day 6 morning (CONFERENCE-NOTES.md) — Cart, dtolnay,
  Esteban, Niko.
- Conference Day 6 mid-afternoon — F12 refined (no custom widgets).
- Existing `crates/schema/src/input_hint.rs` and
  `crates/schema/src/widget.rs`.

## Out of scope

- Form rendering itself — moved to `nebula-editor` separate product.
- JSON Schema export — see ADR-0063.
- Custom validation logic — see `nebula-validator` (extension via
  `Validator` trait).
