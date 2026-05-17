# ADR-0062: `nebula-schema::stdlib` newtype zoo

**Status:** Proposed (2026-05-14)
**Tags:** schema, dx, validation

## Context

Charter F13: *"Newtype with auto-validation is the flagship pattern;
ship `nebula-schema::stdlib` module by default."*

Today, author writing email-input field has three choices:

```rust
// 1. Bare String — no validation, no UI hint:
email: String,

// 2. String with field hint — UI-hinted but type-not-validated:
#[field(hint = "email")] email: String,

// 3. Newtype with auto-validation — flagship pattern:
email: Email,
```

Option 3 is what JSON Schema dynamically-typed languages **cannot do**.
This is Nebula's competitive advantage. But the standard library of
common newtypes doesn't exist yet — author must define `Email`
themselves.

## Decision

Add `nebula-schema::stdlib` **module** (not separate crate) shipping
common domain newtypes.

```toml
nebula-schema = { version = "1.0", features = ["stdlib"] }
# stdlib is default-on
```

### Initial entries (v1.0 ship list)

| Newtype | Wraps | Validation | InputHint | Example |
|---|---|---|---|---|
| `Email` | `String` | RFC 5321 / 5322 lite | `Email` | `user@example.com` |
| `Url` | `url::Url` | URL parser | `Url` | `https://api.example.com/path` |
| `IpAddr` | `std::net::IpAddr` | parser | `Ip` | `192.168.1.1` |
| `Cron` | `String` | cron-parser | `Cron` | `0 9 * * MON` |
| `DurationStr` | `std::time::Duration` | ISO 8601 / humantime parser | `Duration` | `15m`, `PT2H` |
| `Uuid` | `uuid::Uuid` | parser | `Uuid` | `f47ac10b-...` |
| `SemverRange` | `semver::VersionReq` | parser | (none — code editor) | `^1.2.3` |
| `JsonPath` | `String` | jsonpath syntax check | (none — code editor) | `$.users[*].email` |
| `RegexPattern` | `regex::Regex` | regex compile | `Regex` | `[a-z]+` |

Each newtype:

- impls `Deserialize` (parses + validates input)
- impls `Serialize` (round-trips via `Display`)
- impls `HasSchema` (auto-emits matching `InputHint` and `format`)
- impls `Validate` (re-runnable validation for already-deserialized values)
- impls `Display`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Clone`
- carries inner via `pub fn into_inner(self) -> InnerType` and `AsRef<InnerType>`

### Construction API

```rust
impl Email {
    pub fn parse(s: &str) -> Result<Self, ValidationError> { /* ... */ }
}

// Or via TryFrom:
let email: Email = "user@example.com".parse()?;
```

### `define_newtype!` macro

Per Niko Matsakis Day 6 morning — reduce boilerplate:

```rust
nebula_schema::define_newtype! {
    /// Corporate email (must end with @my-company.com)
    pub struct CorpEmail(String);

    fn validate(s: &str) -> Result<(), &'static str> {
        if s.ends_with("@my-company.com") { Ok(()) }
        else { Err("must end with @my-company.com") }
    }

    #[schema(format = "email", widget = "Text")]
}
```

Macro emits all 8 trait impls + Display + parse function.

### Author usage

```rust
#[derive(Schema, Deserialize)]
struct UserSignup {
    email: Email,                // auto-validated, auto-hinted
    homepage: Option<Url>,       // optional + auto-validated
    schedule: Cron,              // auto-validated cron
    timeout: DurationStr,        // ISO 8601 duration
}
```

Form auto-rendered. Validation auto-applied. JSON Schema export auto-
generated with proper `format` annotations.

## Consequences

### Positive

- **Flagship feature shipped.** "Type carries its constraints" — not
  achievable in dynamic-typed JSON Schema world.
- Author DX wins: 1 line vs ~10 lines per custom newtype.
- Forward-compat with JSON Schema export (proper `format` field).
- `define_newtype!` enables author-side custom newtypes (e.g.
  `CorpEmail`, `OrderId`) with same idiomatic API.

### Negative

- Dependency expansion: `url`, `uuid`, `regex`, `semver`,
  `humantime` (or equivalent) added to `nebula-schema` as optional
  deps gated behind `stdlib` feature.
- Every newtype = potential semver complication. Mitigated by
  `#[non_exhaustive]` on internal repr; public API stable.

### Neutral

- Authors who want minimal compile times disable `stdlib`:
  `nebula-schema = { features = ["..."] }` without `stdlib`.

## Selection criteria for future entries

A newtype joins `stdlib` if **all three**:

1. **Domain-universal.** Used across many integration domains (HTTP,
   data, CLI, etc.) — not vendor-specific.
2. **Standard format.** Has well-known parser / spec (IETF RFC, ISO
   standard, well-established library).
3. **No alternative interpretation.** `Email` has unambiguous syntax;
   `PostalAddress` does not (varies by country).

Examples that **would not** make stdlib:

- `CountryCode` — ISO 3166 has multiple variants (alpha-2 vs alpha-3
  vs numeric). Author picks per-application.
- `PhoneNumber` — E.164 vs national formats; libphonenumber too heavy
  for `stdlib` dep.
- `Currency` — ISO 4217 yes, but presentation varies. Skip.

These can ship as **contrib crates** (`nebula-schema-stdlib-phone`
etc.) if community demand emerges.

## References

- Conference Day 6 morning (CONFERENCE-NOTES.md) — Aaron Turon F13
  flagship.
- Conference Day 6 mid-afternoon — `stdlib` as module not separate
  crate (matklad + dtolnay + boats).

## Out of scope

- Custom format parsers per application — author implements via
  `define_newtype!`.
- Vendor-specific newtypes — separate contrib crates.
- Internationalization of validation messages — separate concern.
