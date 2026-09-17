//! Value-free summarization of a [`serde_json`] decode failure.
//!
//! A raw [`serde_json::Error`]'s `Display` embeds the value it choked on —
//! `invalid type: string "…", expected a bool` — and every crate that decodes
//! untrusted or previously-persisted JSON (a stored action state, an inbound
//! webhook body, a durable execution row written by an older build) forwards
//! that error into a surface with its own, wider reach: a typed error's
//! `Display`, a `%error`/`reason` log field, a durable failure record. Four
//! surfaces this summary reaches today: [`nebula-action`](https://docs.rs/nebula-action)'s
//! `ActionError::Validation::detail`, [`nebula-storage-port`](https://docs.rs/nebula-storage-port)'s
//! `StorageError::Serialization`, the durable typed failure envelope and every
//! log line that re-reads it (reached through both of those), and
//! [`nebula-engine`](https://docs.rs/nebula-engine)'s own `tracing::warn!`
//! fields on a decode it can only skip or log rather than turn into a typed
//! error. Forwarding the decoder's own message would republish whatever
//! secret the decoded payload happened to quote, one layer further down —
//! this module exists so no consumer has to choose between a useful
//! diagnostic and that leak.
//!
//! This module is the single owner of value-free decode summaries: every
//! consumer that needs one depends on it rather than keeping its own copy.
//! The `serde_json` feature keeps the dependency optional, so a consumer of
//! this crate that never decodes untrusted JSON pays nothing for it.

use serde_json::error::Category;

/// Summarize a [`serde_json`] decode failure **without** quoting the value that
/// failed to decode.
///
/// The summary is framework-authored **by construction**: it is assembled
/// from the failure's [`Category`] and the parser's line/column, never from
/// the error's own message, so no byte of the decoded value can reach it. A
/// parser that has no position — [`serde_json::from_value`] has no source
/// text to point into and reports an unset one — yields the position-less
/// form instead of a bogus `line 0`.
///
/// # Examples
///
/// ```
/// use nebula_error::decode::value_free_decode_summary;
///
/// let error = serde_json::from_value::<bool>(serde_json::json!("not a bool"))
///     .expect_err("a string is not a bool");
/// let summary = value_free_decode_summary(&error);
///
/// assert!(!summary.contains("not a bool"));
/// assert_eq!(summary, "data error while decoding the value");
/// ```
#[must_use]
pub fn value_free_decode_summary(error: &serde_json::Error) -> String {
    let failure_kind = match error.classify() {
        Category::Io => "i/o error",
        Category::Syntax => "syntax error",
        Category::Data => "data error",
        Category::Eof => "unexpected end of input",
    };
    // A parse position is 1-based, so line 0 is the parser's way of saying it
    // has none — never render it as a real location.
    if error.line() == 0 {
        format!("{failure_kind} while decoding the value")
    } else {
        format!(
            "{failure_kind} at line {} column {}",
            error.line(),
            error.column()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Value the decoder must not re-publish. Structured so a substring match
    /// cannot false-positive on unrelated framework text.
    const MARKER: &str = "MARKER-9f3a-secret";

    #[test]
    fn summary_omits_value_and_position_when_parser_has_none() {
        // `serde_json`'s own `Display` quotes the value it failed on. Drive a
        // real error whose message carries a marker, then prove the summary
        // reports the failure without reproducing that marker.
        let error = serde_json::from_value::<bool>(serde_json::json!(MARKER))
            .expect_err("a string is not a bool");
        assert!(
            error.to_string().contains(MARKER),
            "premise: the raw error quotes the value: {error}"
        );

        // `from_value` has no source text, so the parser reports no position.
        assert_eq!((error.line(), error.column()), (0, 0));

        let summary = value_free_decode_summary(&error);
        assert!(!summary.contains(MARKER), "{summary}");
        assert_eq!(summary, "data error while decoding the value");
    }

    #[test]
    fn summary_keeps_the_parser_position() {
        // A `from_str` data error does carry a position, and pointing the
        // operator at it costs no payload bytes.
        let error = serde_json::from_str::<bool>(&format!("\"{MARKER}\""))
            .expect_err("a string is not a bool");
        assert_eq!((error.line(), error.column()), (1, 20));

        let summary = value_free_decode_summary(&error);
        assert!(!summary.contains(MARKER), "{summary}");
        assert_eq!(summary, "data error at line 1 column 20");
    }
}
