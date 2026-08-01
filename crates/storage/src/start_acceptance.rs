//! Shared start-key replay decision.
//!
//! All three backends reach the same fork once a reservation already exists:
//! the stored fingerprint either matches this request or it does not. Keeping
//! the comparison here means a backend cannot drift into accepting a mismatch
//! (which would hand two different requests one execution) or into rejecting a
//! match (which would break retry convergence).

use nebula_storage_port::store::{KeyedStart, StartAcceptance};

/// Decide whether an existing reservation is this request replayed.
///
/// The version must match as well as the digest: digests produced under
/// different canonicalization rules are not comparable, so a version change
/// reads as a mismatch rather than risking a false match.
///
/// `stored_version` is the raw persisted integer. A value outside the
/// fingerprint version range cannot equal any version this binary produces, so
/// it falls through to a mismatch — the fail-closed direction.
pub(crate) fn replay_outcome(
    start: &KeyedStart<'_>,
    stored_version: i64,
    stored_digest: &[u8],
    execution_id: String,
) -> StartAcceptance {
    let version_matches = stored_version == i64::from(start.fingerprint.version());
    if version_matches && stored_digest == start.fingerprint.digest().as_slice() {
        StartAcceptance::Replayed { execution_id }
    } else {
        StartAcceptance::FingerprintMismatch
    }
}

#[cfg(test)]
mod tests {
    use nebula_storage_port::Scope;
    use nebula_storage_port::dto::{ControlCommand, ControlMsg, NewExecution};
    use nebula_storage_port::store::{KeyedStart, StartAcceptance, StartFingerprint};

    use super::replay_outcome;

    const VERSION: u16 = 1;

    fn keyed_start<'a>(
        scope: &'a Scope,
        command: &'a ControlMsg,
        state: &'a serde_json::Value,
        digest: [u8; 32],
    ) -> KeyedStart<'a> {
        KeyedStart {
            scope,
            start_key: "key-a",
            fingerprint: StartFingerprint::new(VERSION, digest),
            execution_id: "exe_a",
            execution: NewExecution::new("wf_a", state),
            command,
        }
    }

    fn fixture() -> (Scope, ControlMsg, serde_json::Value) {
        let scope = Scope::new("ws", "org");
        let command = ControlMsg {
            id: [1u8; 16],
            execution_id: "exe_a".to_owned(),
            command: ControlCommand::Start,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        };
        (scope, command, serde_json::json!({}))
    }

    #[test]
    fn identical_version_and_digest_replay_the_original_receipt() {
        let (scope, command, state) = fixture();
        let start = keyed_start(&scope, &command, &state, [7u8; 32]);

        assert_eq!(
            replay_outcome(&start, i64::from(VERSION), &[7u8; 32], "exe_a".to_owned()),
            StartAcceptance::Replayed {
                execution_id: "exe_a".to_owned()
            }
        );
    }

    #[test]
    fn a_different_digest_is_a_mismatch() {
        let (scope, command, state) = fixture();
        let start = keyed_start(&scope, &command, &state, [7u8; 32]);

        assert_eq!(
            replay_outcome(&start, i64::from(VERSION), &[8u8; 32], "exe_a".to_owned()),
            StartAcceptance::FingerprintMismatch
        );
    }

    #[test]
    fn the_same_digest_under_different_rules_is_a_mismatch() {
        let (scope, command, state) = fixture();
        let start = keyed_start(&scope, &command, &state, [7u8; 32]);

        assert_eq!(
            replay_outcome(
                &start,
                i64::from(VERSION) + 1,
                &[7u8; 32],
                "exe_a".to_owned()
            ),
            StartAcceptance::FingerprintMismatch,
            "digests from different canonicalization rules are not comparable"
        );
    }

    #[test]
    fn a_truncated_or_corrupt_digest_is_a_mismatch_not_a_match() {
        let (scope, command, state) = fixture();
        let start = keyed_start(&scope, &command, &state, [7u8; 32]);

        assert_eq!(
            replay_outcome(&start, i64::from(VERSION), &[7u8; 16], "exe_a".to_owned()),
            StartAcceptance::FingerprintMismatch
        );
        assert_eq!(
            replay_outcome(&start, -1, &[7u8; 32], "exe_a".to_owned()),
            StartAcceptance::FingerprintMismatch,
            "a version outside the persisted range must fail closed"
        );
    }
}
