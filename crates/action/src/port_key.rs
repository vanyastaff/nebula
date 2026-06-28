//! Re-export of [`PortKey`] from `nebula-core`.
//!
//! The generic validation logic and compile-time macro live in
//! `nebula_core::port_key`. This module keeps action-specific serde rejection
//! tests that exercise `ActionResult` deserialization.

pub use nebula_core::PortKey;

#[cfg(test)]
mod tests {
    use crate::result::ActionResult;

    // ── in-JSON context: ActionResult::Route ─────────────────────────────────

    #[test]
    fn route_with_forged_port_rejected_via_action_result_serde() {
        // This is the primary security test. Before the newtype, this would
        // deserialize Ok because `PortKey = String` accepted anything.
        // After the newtype, "bad port!" is rejected through try_from regardless
        // of whether the data payload itself is valid.
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"Route","port":"bad port!","data":{"type":"Empty"}}"#,
        );
        assert!(
            result.is_err(),
            "ActionResult::Route with a forged port key must be rejected at the serde boundary"
        );
    }

    #[test]
    fn route_with_empty_port_rejected_via_action_result_serde() {
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"Route","port":"","data":{"type":"Empty"}}"#,
        );
        assert!(
            result.is_err(),
            "ActionResult::Route with an empty port key must be rejected at the serde boundary"
        );
    }

    #[test]
    fn route_with_valid_port_accepted_via_action_result_serde() {
        // ActionOutput uses adjacent tagging: {"type":"Empty"} or
        // {"type":"Value","data":<val>}. Use Empty here to keep the fixture minimal.
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"Route","port":"out","data":{"type":"Empty"}}"#,
        );
        assert!(
            result.is_ok(),
            "ActionResult::Route with a valid port key must deserialize successfully"
        );
    }

    // ── ActionResult-level serde rejection — Branch / MultiOutput ────────────
    //
    // These tests are RED on the old `pub type PortKey/BranchKey = String`
    // aliases (invalid strings deserialize Ok) and GREEN after the newtypes
    // route deserialization through TryFrom<String>.

    #[test]
    fn branch_with_forged_selected_rejected_via_action_result_serde() {
        // "bad branch!" contains a space and exclamation mark — always invalid.
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"Branch","selected":"bad branch!","output":{"type":"Empty"},"alternatives":{}}"#,
        );
        assert!(
            result.is_err(),
            "ActionResult::Branch with a forged `selected` key must be rejected at the serde boundary"
        );
    }

    #[test]
    fn branch_with_forged_alternatives_key_rejected_via_action_result_serde() {
        // `selected` is valid but an alternatives map key is forged.
        // Proves that HashMap<BranchKey, _> key deserialization is also guarded.
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"Branch","selected":"ok","output":{"type":"Empty"},"alternatives":{"bad key!":{"type":"Empty"}}}"#,
        );
        assert!(
            result.is_err(),
            "ActionResult::Branch with a forged alternatives map key must be rejected at the serde boundary"
        );
    }

    #[test]
    fn multi_output_with_forged_port_key_rejected_via_action_result_serde() {
        // `outputs` map key "bad port!" is invalid — must be rejected even though
        // the outer type tag and main_output are well-formed.
        let result = serde_json::from_str::<ActionResult<serde_json::Value>>(
            r#"{"type":"MultiOutput","outputs":{"bad port!":{"type":"Empty"}},"main_output":null}"#,
        );
        assert!(
            result.is_err(),
            "ActionResult::MultiOutput with a forged port key must be rejected at the serde boundary"
        );
    }
}
