//! API contract test — exercises every public type by constructing it.

#![allow(
    clippy::no_effect_underscore_binding,
    reason = "bindings exist solely to pin down the public API shape"
)]

use nebula_log::{DestinationFailurePolicy, Rolling, WriterConfig, observability::HookPolicy};

#[test]
fn api_contract_exposes_policy_types() {
    let _dest = DestinationFailurePolicy::BestEffort;
    let _hook = HookPolicy::Inline;
    let _writer = WriterConfig::Multi {
        policy: DestinationFailurePolicy::FailFast,
        writers: vec![WriterConfig::Stderr],
    };
    let _rolling = Rolling::Size(64);
}
