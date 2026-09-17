//! Bounded, provider-free failure records for durable execution state.
//!
//! An execution aggregate that fails has to say *what* failed in a form that
//! survives a checkpoint, a resume, and a post-mortem read — and it has to say
//! it without carrying the failed action's own text with it. The action layer
//! wraps arbitrary provider payloads: `ActionErrorSource` forwards `Display`
//! to whatever `dyn Error` the action supplied, so any record that renders an
//! error's *source chain* publishes provider text (and whatever secret the
//! provider chose to quote) into `executions.state`, `port_execution_journal`,
//! logs, and API bodies.
//!
//! [`ErrorEnvelope`] is the shape that replaces that. It keeps the parts a
//! durable reader can act on — a machine-readable [`ErrorCode`], its
//! [`ErrorCategory`], and whether the failure was retryable — plus two
//! deliberately impoverished diagnostic channels: a bounded, control-character-
//! escaped `redacted_message` that only framework-authored text may enter, and
//! a `source_codes` list that carries the *typed* identity of a cause where the
//! cause has one, instead of its prose.
//!
//! The record is versioned. A durable row written by a different shape — a
//! bare error string from before this shape existed, or a future version this
//! build cannot interpret — fails to decode rather than being coerced into a
//! plausible-looking record.

use std::fmt;

use nebula_error::{ErrorCategory, ErrorCode};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Longest a [`ErrorEnvelope`]'s redacted message may be, in bytes.
///
/// Durable state is read back on every resume and echoed into API bodies, so
/// an unbounded message is paid for repeatedly. The bound sits far above any
/// honest framework diagnostic.
pub const MAX_REDACTED_MESSAGE_BYTES: usize = 512;

/// Appended to a message cut to fit [`MAX_REDACTED_MESSAGE_BYTES`].
///
/// A truncated message must read as truncated: an operator comparing two
/// records has to tell "this differs" from "this is the first 512 bytes of
/// something that differs".
pub const TRUNCATION_MARKER: &str = "…";

/// The only [`ErrorEnvelope`] record version this build writes or accepts.
///
/// A record naming any other version fails to decode — see
/// [`ErrorEnvelope`]'s `Deserialize` contract.
pub const ERROR_ENVELOPE_VERSION: u8 = 1;

/// A durable failure record: typed, bounded, and free of provider payloads.
///
/// # Serialized shape
///
/// ```json
/// {
///   "version": 1,
///   "code": "ENGINE:NODE_FAILED",
///   "category": "internal",
///   "retryable": false,
///   "redacted_message": "node process failed"
/// }
/// ```
///
/// `redacted_message` and `source_codes` are omitted when empty.
///
/// # Decoding
///
/// Decoding is fail-closed. Unknown fields, an unrecognised `category`, a
/// missing field, a `code` that is not a JSON string, a record naming another
/// `version`, and a bare JSON string (the pre-envelope shape) are all errors.
/// Nothing coerces an uninterpretable row into a record that looks valid.
///
/// Decoding also re-checks, on `redacted_message`, `code`, and every
/// `source_codes` entry, the same bound and escape the encoder applies to
/// every text field: at most [`MAX_REDACTED_MESSAGE_BYTES`] bytes, and no
/// character `must_escape` names — control characters, the Arabic Letter Mark
/// and the other bidirectional and zero-width format controls, and the
/// line/paragraph separators. A row from outside this build — a hand-edited
/// row, or one written by code that skipped the constructors — is refused
/// rather than trusted, naming the field and the rule it broke, never the
/// value it refused.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorCategory, ErrorCode};
/// use nebula_execution::ErrorEnvelope;
///
/// let envelope = ErrorEnvelope::new(
///     ErrorCode::new("ENGINE:NODE_FAILED"),
///     ErrorCategory::Internal,
///     false,
/// )
/// .with_redacted_message("node process failed");
///
/// let json = serde_json::to_string(&envelope).expect("serializable");
/// let decoded = ErrorEnvelope::from_durable_json(&json).expect("round-trips");
/// assert_eq!(decoded, envelope);
///
/// // The pre-envelope shape is refused, not reinterpreted.
/// assert!(ErrorEnvelope::from_durable_json(r#""connection timeout""#).is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorEnvelope {
    version: EnvelopeVersion,
    code: ErrorCode,
    category: ErrorCategory,
    retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redacted_message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_codes: Vec<ErrorCode>,
}

impl ErrorEnvelope {
    /// Build a record from a failure's typed identity.
    ///
    /// The three arguments are the whole non-diagnostic contract: what failed
    /// ([`ErrorCode`]), what class of failure it was ([`ErrorCategory`]), and
    /// whether the caller may try again. Diagnostic text is opt-in through
    /// [`Self::with_redacted_message`] and [`Self::with_source_codes`].
    ///
    /// `code` is normalised exactly as the message channel is: codes are
    /// identifiers, but one that carries a character `must_escape` names, or
    /// that exceeds [`MAX_REDACTED_MESSAGE_BYTES`], is rewritten rather than
    /// refused, so this constructor stays infallible and every record it
    /// builds is readable by [`Self::from_durable_json`]. A clean literal
    /// code is returned unchanged, with no allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{ErrorCategory, ErrorCode};
    /// use nebula_execution::ErrorEnvelope;
    ///
    /// let envelope = ErrorEnvelope::new(ErrorCode::new("ENGINE:CANCELLED"), ErrorCategory::Cancelled, false);
    /// assert_eq!(envelope.code().as_str(), "ENGINE:CANCELLED");
    /// assert!(envelope.redacted_message().is_none());
    /// ```
    #[must_use]
    pub fn new(code: ErrorCode, category: ErrorCategory, retryable: bool) -> Self {
        Self {
            version: EnvelopeVersion(ERROR_ENVELOPE_VERSION),
            code: normalize_code(code),
            category,
            retryable,
            redacted_message: None,
            source_codes: Vec::new(),
        }
    }

    /// Attach framework-authored diagnostic text.
    ///
    /// The text is escaped and bounded here, not by the caller: every
    /// character `must_escape` names — control characters, Unicode
    /// bidirectional-override and format controls, and the line/paragraph
    /// separators — is rendered in an escaped form (see `bounded_escaped`),
    /// so stored text cannot forge a log line, emit a terminal escape, or
    /// reorder how a line renders. The result is then cut to
    /// [`MAX_REDACTED_MESSAGE_BYTES`] on a `char` boundary with
    /// [`TRUNCATION_MARKER`] appended.
    ///
    /// **Callers must pass text the framework authored itself.** This method
    /// cannot verify provenance, and a provider payload routed through it is
    /// exactly the leak the envelope exists to prevent. A cause that is a
    /// typed error belongs in [`Self::with_source_codes`]; a cause that is only
    /// prose has no place in a durable record at all.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{ErrorCategory, ErrorCode};
    /// use nebula_execution::{ErrorEnvelope, MAX_REDACTED_MESSAGE_BYTES};
    ///
    /// let envelope = ErrorEnvelope::new(ErrorCode::new("X"), ErrorCategory::Internal, false)
    ///     .with_redacted_message("first line\nsecond line");
    ///
    /// // The newline is stored escaped, so it cannot forge a log record.
    /// assert_eq!(envelope.redacted_message(), Some(r"first line\nsecond line"));
    ///
    /// let long = ErrorEnvelope::new(ErrorCode::new("X"), ErrorCategory::Internal, false)
    ///     .with_redacted_message("x".repeat(MAX_REDACTED_MESSAGE_BYTES * 2));
    /// assert!(long.redacted_message().expect("present").len() <= MAX_REDACTED_MESSAGE_BYTES);
    /// ```
    #[must_use]
    pub fn with_redacted_message(mut self, message: impl AsRef<str>) -> Self {
        self.redacted_message = Some(bounded_escaped(message.as_ref()));
        self
    }

    /// Attach the typed identity of this failure's causes, in chain order.
    ///
    /// This is where a cause belongs when the framework can name it. A cause
    /// the framework cannot name stays out of the record: the durable reader
    /// gets the outer code and nothing invented.
    ///
    /// Each code is normalised the same way [`Self::new`] normalises its
    /// own — see there for the contract.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{ErrorCategory, ErrorCode};
    /// use nebula_execution::ErrorEnvelope;
    ///
    /// let envelope = ErrorEnvelope::new(ErrorCode::new("ENGINE:ACTION"), ErrorCategory::Internal, false)
    ///     .with_source_codes([ErrorCode::new("ACTION:FATAL")]);
    /// assert_eq!(envelope.source_codes()[0].as_str(), "ACTION:FATAL");
    /// ```
    #[must_use]
    pub fn with_source_codes(mut self, codes: impl IntoIterator<Item = ErrorCode>) -> Self {
        self.source_codes = codes.into_iter().map(normalize_code).collect();
        self
    }

    /// Recorded wire format; readers reject unsupported versions.
    ///
    /// Mirrors [`ExecutionCheckpoint::format_version`](crate::ExecutionCheckpoint::format_version):
    /// the version is a private field with an accessor, so it cannot be desynchronised from the
    /// shape it describes.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version.0
    }

    /// The machine-readable identity of this failure.
    #[must_use]
    pub fn code(&self) -> &ErrorCode {
        &self.code
    }

    /// The class of failure this record describes.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Whether the failure that produced this record was retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// The bounded, escaped, framework-authored diagnostic, if one was attached.
    #[must_use]
    pub fn redacted_message(&self) -> Option<&str> {
        self.redacted_message.as_deref()
    }

    /// The typed identities of this failure's causes, in chain order.
    #[must_use]
    pub fn source_codes(&self) -> &[ErrorCode] {
        &self.source_codes
    }

    /// Decode a durable record, refusing anything this build cannot interpret.
    ///
    /// # Errors
    ///
    /// Returns the `serde_json` error naming why the record was refused: an
    /// unknown version, an unknown field or category, a missing field, or a
    /// shape that is not a record at all (including the pre-envelope bare
    /// string).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{ErrorCategory, ErrorCode};
    /// use nebula_execution::ErrorEnvelope;
    ///
    /// let record = r#"{"version":1,"code":"X","category":"internal","retryable":true}"#;
    /// assert!(ErrorEnvelope::from_durable_json(record).is_ok());
    ///
    /// let future = r#"{"version":2,"code":"X","category":"internal","retryable":true}"#;
    /// assert!(ErrorEnvelope::from_durable_json(future).is_err());
    /// ```
    pub fn from_durable_json(record: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(record)
    }

    /// Encode this record for durable storage.
    ///
    /// # Errors
    ///
    /// Returns the `serde_json` error if the record cannot be encoded; the
    /// shape holds no values that fail to serialize, so this is unreachable in
    /// practice and exists to keep the encode path fallible for callers.
    pub fn to_durable_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl fmt::Display for ErrorEnvelope {
    /// Renders `code` alone, or `code: redacted_message` when a message was
    /// attached — the compact projection a log line, an OnError port payload,
    /// or an API body shows a reader.
    ///
    /// Neither channel can carry provider text: the encode side
    /// ([`ErrorEnvelope::with_redacted_message`]) admits only framework-authored
    /// text, and the decode side (see the struct's `# Decoding` section)
    /// refuses anything outside that same encoded invariant on the way back
    /// in. The guarantee belongs to the type, not to who happened to write it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())?;
        if let Some(message) = self.redacted_message.as_deref() {
            write!(formatter, ": {message}")?;
        }
        Ok(())
    }
}

/// Serde-only mirror of [`ErrorEnvelope`]'s wire shape.
///
/// [`ErrorEnvelope`] derives [`Serialize`] but hand-rolls [`Deserialize`]: a
/// derived `Deserialize` on the public struct would accept any `code` or
/// `redacted_message` string, including ones the encode side would never
/// produce (oversized, or carrying a raw control or bidi-override character).
/// Decoding through this private twin first, then validating, is what lets
/// [`ErrorEnvelope::deserialize`] refuse those before an `ErrorEnvelope` value
/// carrying them can exist at all.
#[derive(Deserialize)]
#[serde(rename = "ErrorEnvelope", deny_unknown_fields)]
struct ErrorEnvelopeWire {
    version: EnvelopeVersion,
    code: ErrorCode,
    category: ErrorCategory,
    retryable: bool,
    #[serde(default)]
    redacted_message: Option<String>,
    #[serde(default)]
    source_codes: Vec<ErrorCode>,
}

impl<'de> Deserialize<'de> for ErrorEnvelope {
    /// See the struct's `# Decoding` section for the contract this enforces.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ErrorEnvelopeWire::deserialize(deserializer)?;

        if let Some(message) = wire.redacted_message.as_deref() {
            reject_unless_bounded_and_escaped(message, "redacted_message")?;
        }
        reject_unless_bounded_and_escaped(wire.code.as_str(), "code")?;
        for source in &wire.source_codes {
            reject_unless_bounded_and_escaped(source.as_str(), "source_codes")?;
        }

        Ok(Self {
            version: wire.version,
            code: wire.code,
            category: wire.category,
            retryable: wire.retryable,
            redacted_message: wire.redacted_message,
            source_codes: wire.source_codes,
        })
    }
}

/// Refuse `value` (naming `field`, never echoing `value` itself) unless it
/// satisfies exactly what [`bounded_escaped`] guarantees on the encode side:
/// at most [`MAX_REDACTED_MESSAGE_BYTES`] bytes and no [`must_escape`] character.
fn reject_unless_bounded_and_escaped<E: de::Error>(
    value: &str,
    field: &'static str,
) -> Result<(), E> {
    if value.len() > MAX_REDACTED_MESSAGE_BYTES {
        return Err(de::Error::custom(format_args!(
            "field `{field}` exceeds the {MAX_REDACTED_MESSAGE_BYTES}-byte bound the encoder \
             applies to every text field"
        )));
    }
    if value.chars().any(must_escape) {
        return Err(de::Error::custom(format_args!(
            "field `{field}` contains a character the encoder escapes on every text field"
        )));
    }
    Ok(())
}

/// Whether `bounded_escaped` escapes `c` rather than storing it verbatim.
///
/// Beyond `char::is_control()` (`Cc`: `\n`, `\t`, ESC, …), this also covers
/// the characters that can forge the *visual* shape of a rendered line
/// without being a control character at all: every `Bidi_Control` codepoint —
/// the Arabic Letter Mark (`U+061C`) and the zero-width and bidirectional
/// format controls (`U+200B..=U+200F`, `U+202A..=U+202E`, `U+2066..=U+2069`,
/// `U+FEFF`), with U+202E (RIGHT-TO-LEFT OVERRIDE) able to make a log reader
/// see character order that is not the actual byte order — and the two
/// line/paragraph separators (`U+2028`, `U+2029`), which start a new line in
/// a renderer that only checks for `\n`.
fn must_escape(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// Escape `message` so it cannot forge framing, then cut it to the byte bound.
///
/// Escaping runs first so the bound applies to what is actually stored: a
/// message built from control characters cannot escape the limit by expanding
/// after it was measured. [`char::is_control`] (`Cc`) characters use
/// [`char::escape_debug`] (`\n`, `\t`, …), which is legible; the remaining
/// [`must_escape`] characters use [`char::escape_unicode`] (`\u{202e}`) so the
/// result does not depend on `escape_debug`'s `is_printable` tables, which
/// were never meant to cover bidi/format controls.
fn bounded_escaped(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for character in message.chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else if must_escape(character) {
            escaped.extend(character.escape_unicode());
        } else {
            escaped.push(character);
        }
    }

    if escaped.len() <= MAX_REDACTED_MESSAGE_BYTES {
        return escaped;
    }

    let target = MAX_REDACTED_MESSAGE_BYTES.saturating_sub(TRUNCATION_MARKER.len());
    let end = escaped.floor_char_boundary(target);
    let mut truncated = String::with_capacity(end + TRUNCATION_MARKER.len());
    truncated.push_str(&escaped[..end]);
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

/// Normalise `code` so it satisfies exactly what decode re-checks on the way
/// back in: at most [`MAX_REDACTED_MESSAGE_BYTES`] bytes and no `must_escape`
/// character.
///
/// Codes are identifiers, not free text, and every code reachable in-tree
/// today is a literal that is already clean — the length/escape check below
/// is what keeps that path an identity with no allocation. A code that does
/// carry a bound-exceeding or escapable character is rebuilt through
/// [`ErrorCode::custom`] via `bounded_escaped` rather than refused, which is
/// what keeps [`ErrorEnvelope::new`] and [`ErrorEnvelope::with_source_codes`]
/// infallible.
fn normalize_code(code: ErrorCode) -> ErrorCode {
    if code.as_str().len() <= MAX_REDACTED_MESSAGE_BYTES && !code.as_str().chars().any(must_escape)
    {
        return code;
    }
    ErrorCode::custom(bounded_escaped(code.as_str()))
}

/// The record version, decoded fail-closed.
///
/// A private newtype rather than a bare `u8` field so the version check runs
/// wherever the record is decoded — including inside a larger aggregate such as
/// `ExecutionState`, where no envelope-specific entry point is involved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnvelopeVersion(u8);

impl Serialize for EnvelopeVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for EnvelopeVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let found = u8::deserialize(deserializer)?;
        if found == ERROR_ENVELOPE_VERSION {
            Ok(Self(found))
        } else {
            Err(de::Error::custom(format_args!(
                "unsupported error-envelope version {found}: this build writes and reads \
                 v{ERROR_ENVELOPE_VERSION} only"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_error::codes;

    fn envelope() -> ErrorEnvelope {
        ErrorEnvelope::new(codes::INTERNAL, ErrorCategory::Internal, false)
    }

    fn envelope_with_code(code: ErrorCode) -> ErrorEnvelope {
        ErrorEnvelope::new(code, ErrorCategory::Internal, false)
    }

    #[test]
    fn record_round_trips_through_its_durable_json() {
        let envelope = envelope()
            .with_redacted_message("planning failed")
            .with_source_codes([ErrorCode::new("WORKFLOW:CYCLE")]);

        let json = envelope.to_durable_json().expect("encodes");
        let decoded = ErrorEnvelope::from_durable_json(&json).expect("decodes");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn empty_diagnostic_channels_are_omitted_from_the_record() {
        let json = envelope().to_durable_json().expect("encodes");

        assert!(!json.contains("redacted_message"), "{json}");
        assert!(!json.contains("source_codes"), "{json}");
    }

    #[test]
    fn every_record_names_the_version_this_build_writes() {
        let json = envelope().to_durable_json().expect("encodes");

        assert!(json.contains(r#""version":1"#), "{json}");
    }

    /// Red on revert: the pre-envelope shape was a free-text `String`, so a durable
    /// row written before this type existed is a bare JSON string. Accepting it
    /// would silently keep provider prose in state, which is the whole defect.
    #[test]
    fn legacy_bare_string_record_is_refused() {
        let error = ErrorEnvelope::from_durable_json(r#""connection timeout""#)
            .expect_err("a bare string is not an envelope");

        assert!(error.to_string().contains("invalid type"), "{error}");
    }

    /// Red on revert: a future version must not be read as this one — its
    /// fields may mean something else.
    #[test]
    fn unknown_version_is_refused_with_the_versions_named() {
        let record = r#"{"version":2,"code":"X","category":"internal","retryable":true}"#;

        let error = ErrorEnvelope::from_durable_json(record).expect_err("v2 is not readable");

        let message = error.to_string();
        assert!(
            message.contains("unsupported error-envelope version 2"),
            "{message}"
        );
        assert!(message.contains("v1 only"), "{message}");
    }

    /// Red on revert: a field this build does not know may change what the
    /// fields it does know mean, so the record is refused rather than
    /// partially read.
    #[test]
    fn unknown_field_is_refused() {
        let record =
            r#"{"version":1,"code":"X","category":"internal","retryable":true,"provider":"acme"}"#;

        let error = ErrorEnvelope::from_durable_json(record).expect_err("unknown field");

        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn unknown_category_is_refused() {
        let record = r#"{"version":1,"code":"X","category":"meltdown","retryable":true}"#;

        let error = ErrorEnvelope::from_durable_json(record).expect_err("unknown category");

        assert!(error.to_string().contains("unknown variant"), "{error}");
    }

    #[test]
    fn missing_typed_identity_is_refused() {
        let record = r#"{"version":1,"category":"internal","retryable":true}"#;

        let error = ErrorEnvelope::from_durable_json(record).expect_err("no code");

        assert!(
            error.to_string().contains("missing field `code`"),
            "{error}"
        );
    }

    #[test]
    fn control_characters_are_escaped_so_text_cannot_forge_framing() {
        let envelope = envelope().with_redacted_message("a\nb\u{1b}[31mc\td");

        assert_eq!(
            envelope.redacted_message(),
            Some(r"a\nb\u{1b}[31mc\td"),
            "newline, escape, and tab must all be stored in a visible form"
        );
    }

    #[test]
    fn long_message_is_cut_to_the_bound_on_a_char_boundary() {
        // Multi-byte characters straddle the byte bound; the cut must land on a
        // boundary or slicing panics.
        let message = "é".repeat(MAX_REDACTED_MESSAGE_BYTES);

        let envelope = envelope().with_redacted_message(message);

        let stored = envelope.redacted_message().expect("present");
        assert!(
            stored.len() <= MAX_REDACTED_MESSAGE_BYTES,
            "{}",
            stored.len()
        );
        assert!(stored.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn message_exactly_at_the_bound_is_kept_intact() {
        let message = "x".repeat(MAX_REDACTED_MESSAGE_BYTES);

        let envelope = envelope().with_redacted_message(message.clone());

        assert_eq!(envelope.redacted_message(), Some(message.as_str()));
        assert!(
            !envelope
                .redacted_message()
                .expect("present")
                .ends_with(TRUNCATION_MARKER)
        );
    }

    #[test]
    fn display_projects_code_alone_when_no_message_was_attached() {
        assert_eq!(envelope().to_string(), "INTERNAL");
    }

    #[test]
    fn display_projects_code_and_message_when_one_was_attached() {
        let envelope = envelope().with_redacted_message("planning failed");

        assert_eq!(envelope.to_string(), "INTERNAL: planning failed");
    }

    /// `ErrorCategory`'s `Deserialize` (`nebula-error`, #1016) accepts an
    /// owned string, not only a borrowed one, so `from_value` — which cannot
    /// hand back a borrow into a transient `Value` — now decodes a valid
    /// envelope like any other input. Red on revert: with `ErrorCategory`
    /// back to `<&str>::deserialize`, this fails with "invalid type: string
    /// ..., expected a borrowed string".
    #[test]
    fn valid_envelope_decodes_from_an_owned_value() {
        let envelope = envelope().with_redacted_message("planning failed");
        let value = serde_json::to_value(&envelope).expect("encodes");

        let decoded: ErrorEnvelope =
            serde_json::from_value(value).expect("a valid envelope decodes from an owned Value");

        assert_eq!(decoded, envelope);
    }

    /// Red on revert: with the derived `Deserialize` restored (no field
    /// validation), an oversized `redacted_message` decodes `Ok`.
    #[test]
    fn oversized_message_is_refused_on_decode() {
        let oversized = "x".repeat(MAX_REDACTED_MESSAGE_BYTES + 1);
        let record = serde_json::json!({
            "version": 1,
            "code": "X",
            "category": "internal",
            "retryable": true,
            "redacted_message": oversized,
        })
        .to_string();

        let error = ErrorEnvelope::from_durable_json(&record)
            .expect_err("a message over the byte bound must be refused");

        assert!(
            error.to_string().contains("redacted_message"),
            "the refusal must name the field, not echo the value: {error}"
        );
    }

    /// The bound is `>`, not `>=`: a message landing exactly on
    /// [`MAX_REDACTED_MESSAGE_BYTES`] is the encode side's own output for text
    /// that filled the budget exactly (see
    /// `message_exactly_at_the_bound_is_kept_intact`), so decode must admit it.
    /// Red on revert: an off-by-one `>=` check refuses this message.
    #[test]
    fn message_at_exactly_the_bound_is_admitted_on_decode() {
        let at_bound = "x".repeat(MAX_REDACTED_MESSAGE_BYTES);
        let record = serde_json::json!({
            "version": 1,
            "code": "X",
            "category": "internal",
            "retryable": true,
            "redacted_message": at_bound,
        })
        .to_string();

        let decoded = ErrorEnvelope::from_durable_json(&record)
            .expect("a message exactly at the byte bound must be admitted");

        assert_eq!(decoded.redacted_message(), Some(at_bound.as_str()));
    }

    /// Red on revert: with the derived `Deserialize` restored, a raw `\n`
    /// (the JSON escape `\n` decodes to the actual control character) is
    /// accepted, letting a corrupted or hand-edited row forge a log line.
    #[test]
    fn raw_newline_in_message_is_refused_on_decode() {
        let record = r#"{"version":1,"code":"X","category":"internal","retryable":true,"redacted_message":"a\nb"}"#;

        let error = ErrorEnvelope::from_durable_json(record)
            .expect_err("a raw control character in redacted_message must be refused");

        assert!(
            error.to_string().contains("redacted_message"),
            "the refusal must name the field, not echo the value: {error}"
        );
    }

    /// Red on revert: with the derived `Deserialize` restored, a raw ESC
    /// byte in `code` is accepted, letting a corrupted row forge a terminal
    /// escape sequence through the `Display` projection.
    #[test]
    fn esc_in_code_is_refused_on_decode() {
        let record = serde_json::json!({
            "version": 1,
            "code": "X\u{1b}[31m",
            "category": "internal",
            "retryable": true,
        })
        .to_string();

        let error = ErrorEnvelope::from_durable_json(&record)
            .expect_err("a raw ESC character in code must be refused");

        assert!(
            error.to_string().contains("code"),
            "the refusal must name the field, not echo the value: {error}"
        );
    }

    /// Red on revert: U+202E (RIGHT-TO-LEFT OVERRIDE) is not
    /// `char::is_control()`, so without `must_escape` a raw one in
    /// `redacted_message` decodes `Ok` and can forge the visual order of a
    /// rendered log line.
    #[test]
    fn raw_bidi_override_in_message_is_refused_on_decode() {
        let record = serde_json::json!({
            "version": 1,
            "code": "X",
            "category": "internal",
            "retryable": true,
            "redacted_message": "a\u{202e}b",
        })
        .to_string();

        let error = ErrorEnvelope::from_durable_json(&record)
            .expect_err("a raw bidi-override character must be refused");

        assert!(
            error.to_string().contains("redacted_message"),
            "the refusal must name the field, not echo the value: {error}"
        );
    }

    /// The encode side's own output must always satisfy the decode side: a
    /// message long enough to truncate, containing a character that gets
    /// escaped, round-trips through `to_durable_json`/`from_durable_json`.
    #[test]
    fn a_truncated_and_escaped_message_round_trips_through_decode() {
        let envelope = envelope().with_redacted_message(format!(
            "line one\nline two {}",
            "x".repeat(MAX_REDACTED_MESSAGE_BYTES)
        ));
        assert!(
            envelope
                .redacted_message()
                .expect("present")
                .ends_with(TRUNCATION_MARKER),
            "fixture must actually exercise truncation"
        );

        let json = envelope.to_durable_json().expect("encodes");
        let decoded = ErrorEnvelope::from_durable_json(&json)
            .expect("the encode side's own output must satisfy the decode side's invariant");

        assert_eq!(decoded, envelope);
    }

    /// U+202E (RIGHT-TO-LEFT OVERRIDE), U+2028 (LINE SEPARATOR), and U+061C
    /// (ARABIC LETTER MARK) are not `char::is_control()`, so without
    /// `must_escape`, `bounded_escaped` would pass them through unescaped.
    /// U+061C is the one `Bidi_Control` codepoint outside the contiguous
    /// ranges `must_escape` otherwise covers. Red on revert: `stored`
    /// contains the raw characters instead of their `\u{...}` escapes.
    #[test]
    fn bidi_override_and_line_separator_are_escaped_not_stored_raw() {
        let envelope = envelope().with_redacted_message("a\u{202e}b\u{2028}c\u{061c}d");
        let stored = envelope.redacted_message().expect("present");

        assert_eq!(stored, r"a\u{202e}b\u{2028}c\u{61c}d");
        assert!(!stored.contains('\u{202e}'), "{stored}");
        assert!(!stored.contains('\u{2028}'), "{stored}");
        assert!(!stored.contains('\u{061c}'), "{stored}");
    }

    /// Red on revert: without normalising `code` in [`ErrorEnvelope::new`],
    /// the encode side accepts a code carrying a raw control character
    /// (`ErrorCode::custom` performs no validation), and `to_durable_json` /
    /// `from_durable_json` fails on the way back in because `deserialize`
    /// refuses exactly what `reject_unless_bounded_and_escaped` checks —
    /// the same invariant the encode side was supposed to have already
    /// enforced.
    #[test]
    fn a_code_with_a_control_character_round_trips_after_normalisation() {
        let envelope = envelope_with_code(ErrorCode::custom("A\nB"));

        assert_eq!(envelope.code().as_str(), r"A\nB");

        let json = envelope.to_durable_json().expect("encodes");
        let decoded = ErrorEnvelope::from_durable_json(&json)
            .expect("a normalised code must satisfy the decode side's own invariant");
        assert_eq!(decoded, envelope);
    }

    /// The normalisation in [`ErrorEnvelope::new`] must be an identity for a
    /// code that already satisfies the bound and escape check: no rebuild
    /// through `ErrorCode::custom`, and so no allocation. Proven here by
    /// pointer identity on the borrowed literal's bytes, which a rebuild
    /// through an owned `String` cannot preserve.
    #[test]
    fn a_clean_literal_code_is_unchanged_by_normalisation() {
        let code = codes::INTERNAL;
        let original_ptr = code.as_str().as_ptr();

        let envelope = envelope_with_code(code);

        assert!(std::ptr::eq(
            envelope.code().as_str().as_ptr(),
            original_ptr
        ));
    }
}
