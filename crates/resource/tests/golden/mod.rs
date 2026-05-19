//! Shared golden-fixture oracle for the topology-collapse characterization
//! suite.
//!
//! A test drives a scenario and records an ordered list of observable
//! `(step, detail)` events. [`EventLog::assert_matches_golden`] serializes
//! the log to a stable JSON string and compares it byte-for-byte against a
//! committed fixture under `tests/fixtures/<name>.golden`. A later refactor
//! replays the *same* scenarios and asserts equality against the *same*
//! committed file, so behavior is diffed against a frozen baseline that does
//! not move with the API.
//!
//! Set `NEBULA_REGENERATE_GOLDENS=1` to (re)write the fixtures instead of
//! asserting. Regeneration is an explicit, opt-in step — never the default,
//! so a behavior change cannot silently rewrite its own baseline.
//!
//! The serialization is intentionally hand-rolled (no `serde`) so the
//! fixture format is fully owned by this module and cannot drift with a
//! dependency.

#![allow(
    dead_code,
    reason = "shared test helper: not every including test binary uses every item"
)]

use std::path::PathBuf;

/// One observed step in a scenario: a stable event name plus a detail
/// string. Both are escaped on serialization so arbitrary content is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub step: String,
    pub detail: String,
}

/// An ordered log of observed events for one scenario.
#[derive(Debug, Default, Clone)]
pub struct EventLog {
    events: Vec<Event>,
}

impl EventLog {
    /// A fresh, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an observed event. Order is significant — it is part of the
    /// captured contract.
    pub fn push(&mut self, step: &str, detail: &str) {
        self.events.push(Event {
            step: step.to_owned(),
            detail: detail.to_owned(),
        });
    }

    /// Serializes the log to the stable golden text format: one JSON object
    /// per line inside a top-level array, deterministic field order.
    pub fn to_golden_string(&self) -> String {
        let mut out = String::from("[\n");
        for (i, ev) in self.events.iter().enumerate() {
            out.push_str("  {\"step\": \"");
            out.push_str(&escape(&ev.step));
            out.push_str("\", \"detail\": \"");
            out.push_str(&escape(&ev.detail));
            out.push_str("\"}");
            if i + 1 < self.events.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push(']');
        out.push('\n');
        out
    }

    /// Compares this log against the committed golden for `name`, or
    /// (re)writes it when `NEBULA_REGENERATE_GOLDENS=1`.
    ///
    /// # Panics
    ///
    /// Panics (failing the test) on any mismatch, or if the fixture is
    /// missing and regeneration was not requested — the proof must be a
    /// committed file, never a runtime-only artifact.
    pub fn assert_matches_golden(&self, name: &str) {
        let path = fixture_path(name);
        let actual = self.to_golden_string();

        if std::env::var("NEBULA_REGENERATE_GOLDENS").as_deref() == Ok("1") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixtures dir");
            }
            std::fs::write(&path, actual.as_bytes())
                .unwrap_or_else(|e| panic!("write golden {}: {e}", path.display()));
            return;
        }

        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "golden fixture {} missing or unreadable ({e}); regenerate \
                 with NEBULA_REGENERATE_GOLDENS=1 and commit it",
                path.display()
            )
        });

        assert_eq!(
            actual,
            expected,
            "observed scenario outcome diverged from the committed golden \
             {}. If this change is intentional, regenerate with \
             NEBULA_REGENERATE_GOLDENS=1 and review the fixture diff.",
            path.display()
        );
    }
}

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(format!("{name}.golden"));
    p
}

/// Minimal JSON string escaping for the fixture format.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}
