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
