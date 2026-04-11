//! Regression guard for the in-memory size of key public types.
//!
//! The asserted numbers are the baseline measured on 2026-04-11 after
//! the domain-group refactor (`.claude/crates/action.md`). They exist
//! so any future change that silently inflates these hot types has to
//! either (a) explain itself in the PR, or (b) update the constants
//! below with the new baseline.
//!
//! **Platform note:** sizes are asserted for 64-bit targets only
//! (`#[cfg(target_pointer_width = "64")]`). On 32-bit they are merely
//! printed, not asserted.
//!
//! Run with: `cargo nextest run -p nebula-action --test type_sizes`.

use std::collections::HashMap;
use std::mem::size_of;
use std::time::Duration;

use nebula_action::{
    ActionContext, ActionError, ActionHandler, ActionMetadata, ActionOutput, ActionResult,
    BinaryData, Cost, DeferredOutput, IncomingEvent, OutputEnvelope, OutputMeta, Progress,
    StreamOutput, Timing, TokenUsage, TriggerEventOutcome,
};

#[test]
#[cfg(target_pointer_width = "64")]
fn top_level_type_sizes_are_stable() {
    // Hot path — engine touches these on every action dispatch. Any
    // growth here needs deliberate justification.
    assert_eq!(
        size_of::<ActionResult<serde_json::Value>>(),
        216,
        "ActionResult<Value> grew — did you add a large inline variant? \
         Prefer boxing the cold variant or update this baseline."
    );
    assert_eq!(
        size_of::<ActionResult<()>>(),
        216,
        "ActionResult<T> should not depend on T — the biggest variants \
         do not carry T."
    );
    assert_eq!(
        size_of::<ActionOutput<serde_json::Value>>(),
        144,
        "ActionOutput<Value> grew — `BinaryData` is the inline variant \
         that drives this size, check it first."
    );
    assert_eq!(size_of::<ActionContext>(), 104);
    assert_eq!(size_of::<ActionMetadata>(), 168);
    assert_eq!(size_of::<ActionError>(), 64);
    assert_eq!(size_of::<ActionHandler>(), 24);
    assert_eq!(size_of::<IncomingEvent>(), 96);
    assert_eq!(size_of::<TriggerEventOutcome>(), 32);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn output_support_type_sizes_are_stable() {
    // OutputEnvelope / OutputMeta / DeferredOutput are large but
    // cold — they sit off the per-node dispatch path. Assert current
    // sizes so the engine-side work that boxes or slims them later
    // has explicit before/after numbers.
    assert_eq!(size_of::<BinaryData>(), 136);
    assert_eq!(size_of::<StreamOutput>(), 136);
    assert_eq!(size_of::<DeferredOutput>(), 360);
    assert_eq!(size_of::<OutputEnvelope<serde_json::Value>>(), 416);
    assert_eq!(size_of::<OutputMeta>(), 272);
    assert_eq!(size_of::<Progress>(), 48);
    assert_eq!(size_of::<Timing>(), 56);
    assert_eq!(size_of::<Cost>(), 48);
    assert_eq!(size_of::<TokenUsage>(), 32);
}

/// Sanity check for the niche optimisation.
///
/// If any variant of `ActionOutput` ever grows a `NonZero`/pointer field
/// that gives up its niche, `Option<ActionOutput<T>>` will no longer
/// match `ActionOutput<T>` in size. That is a silent performance cliff
/// for the `Skip` variant of `ActionResult` which carries exactly this
/// `Option`.
#[test]
#[cfg(target_pointer_width = "64")]
fn option_of_action_output_uses_niche() {
    assert_eq!(
        size_of::<Option<ActionOutput<serde_json::Value>>>(),
        size_of::<ActionOutput<serde_json::Value>>(),
        "Option<ActionOutput> lost its niche — a variant now carries a \
         non-nichable field. Check recent additions to ActionOutput."
    );
}

/// Smoke-print baseline sizes. Always runs; asserts nothing. Useful
/// for `--nocapture` when chasing a change.
#[test]
fn print_type_size_baseline() {
    let rows: &[(&str, usize)] = &[
        (
            "ActionResult<Value>",
            size_of::<ActionResult<serde_json::Value>>(),
        ),
        ("ActionResult<()>", size_of::<ActionResult<()>>()),
        (
            "ActionOutput<Value>",
            size_of::<ActionOutput<serde_json::Value>>(),
        ),
        ("ActionContext", size_of::<ActionContext>()),
        ("ActionMetadata", size_of::<ActionMetadata>()),
        ("ActionError", size_of::<ActionError>()),
        ("ActionHandler", size_of::<ActionHandler>()),
        ("IncomingEvent", size_of::<IncomingEvent>()),
        ("TriggerEventOutcome", size_of::<TriggerEventOutcome>()),
        ("BinaryData", size_of::<BinaryData>()),
        ("StreamOutput", size_of::<StreamOutput>()),
        ("DeferredOutput", size_of::<DeferredOutput>()),
        (
            "OutputEnvelope<Value>",
            size_of::<OutputEnvelope<serde_json::Value>>(),
        ),
        ("OutputMeta", size_of::<OutputMeta>()),
        ("Progress", size_of::<Progress>()),
        ("Timing", size_of::<Timing>()),
        ("Cost", size_of::<Cost>()),
        ("TokenUsage", size_of::<TokenUsage>()),
        ("serde_json::Value", size_of::<serde_json::Value>()),
        ("String", size_of::<String>()),
        ("Vec<u8>", size_of::<Vec<u8>>()),
        ("Duration", size_of::<Duration>()),
        (
            "HashMap<String, ActionOutput<Value>>",
            size_of::<HashMap<String, ActionOutput<serde_json::Value>>>(),
        ),
    ];
    println!();
    for (name, size) in rows {
        println!("{name:40} size={size:>6}  lines={}", size.div_ceil(64),);
    }
}
