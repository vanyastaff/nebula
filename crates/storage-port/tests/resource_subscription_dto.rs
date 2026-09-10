use std::assert_matches;
use std::str::FromStr;
use std::time::Duration;

use nebula_storage_port::Scope;
use nebula_storage_port::dto::{
    AcceptResourceEventRequest, EventEnvelope, EventOccurrenceKey, EventOccurrenceNamespace,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceDeliveryClaimToken, ResourceDeliveryId, ResourceEventId,
    ResourceEventState, ResourceHandoffClaimToken, ResourceKind, ResourceLeaseGeneration,
    ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize, ResourceSlotIdentity,
    ResourceSourceLeaseToken, ResourceSubscriptionId, ResourceSubscriptionState, SharedResourceId,
    SharedResourceIdentity, TerminalDeliveryIneligibility,
};
use uuid::Uuid;

#[test]
fn shared_resource_identity_enforces_exact_byte_boundaries() {
    assert_matches!(
        ResourceKind::new(""),
        Err(nebula_storage_port::dto::SharedResourceValueError::Empty { .. })
    );
    assert_eq!(
        ResourceKind::new("k".repeat(128))
            .expect("128-byte resource kind")
            .as_str()
            .len(),
        128
    );
    assert!(ResourceKind::new("k".repeat(129)).is_err());
    assert!(ResourceKind::new("é".repeat(64)).is_ok());
    assert!(ResourceKind::new(format!("{}a", "é".repeat(64))).is_err());

    assert!(ResourceConfigurationIdentity::try_from_vec(Vec::new()).is_err());
    assert_eq!(
        ResourceConfigurationIdentity::try_from_vec(vec![1; 65_536])
            .expect("maximum configuration identity")
            .as_bytes()
            .len(),
        65_536
    );
    assert!(ResourceConfigurationIdentity::try_from_vec(vec![1; 65_537]).is_err());

    assert!(
        ResourceSlotIdentity::try_from_vec(Vec::new())
            .expect("empty slot identity is valid")
            .as_bytes()
            .is_empty()
    );
    assert!(ResourceSlotIdentity::try_from_vec(vec![2; 65_536]).is_ok());
    assert!(ResourceSlotIdentity::try_from_vec(vec![2; 65_537]).is_err());
}

#[test]
fn shared_resource_identity_uses_exact_fields_after_digest_acceleration() {
    let make_identity = |configuration: &[u8], slot: &[u8]| {
        SharedResourceIdentity::new(
            ResourceKind::new("telegram.bot").expect("valid kind"),
            ResourceCompatibilityVersion::new(3),
            ResourceConfigurationIdentity::try_from_vec(configuration.to_vec())
                .expect("valid configuration"),
            ResourceSlotIdentity::try_from_vec(slot.to_vec()).expect("valid slot"),
        )
    };
    let first = make_identity(b"config-a", b"slot-a");
    let replay = make_identity(b"config-a", b"slot-a");
    let different_configuration = make_identity(b"config-b", b"slot-a");
    let different_slot = make_identity(b"config-a", b"slot-b");

    assert_eq!(first, replay);
    assert_ne!(first, different_configuration);
    assert_ne!(first, different_slot);
    assert_eq!(first.digest(), replay.digest());
    assert_eq!(
        first.digest(),
        &[
            0xbd, 0xd6, 0x90, 0x37, 0x4c, 0xf9, 0xa4, 0xe1, 0x3a, 0x0c, 0xe4, 0x9c, 0x42, 0xd7,
            0x9c, 0x84, 0x69, 0x53, 0xbf, 0xf4, 0x90, 0x65, 0x1b, 0xcc, 0x29, 0x60, 0x14, 0xde,
            0x10, 0x15, 0xd0, 0x46,
        ]
    );
}

#[test]
fn occurrence_digest_has_stable_fixed_width_framing() {
    let request = AcceptResourceEventRequest::new(
        Scope::new("workspace", "org"),
        SharedResourceId::from_bytes([1; 16]),
        ResourceSourceLeaseToken::new(Uuid::from_bytes([2; 16]), ResourceLeaseGeneration::new(1)),
        EventOccurrenceNamespace::new("telegram.update").expect("valid namespace"),
        EventOccurrenceKey::try_from_vec(b"event-1".to_vec()).expect("valid occurrence"),
        EventEnvelope::try_from_vec(1, b"payload".to_vec()).expect("valid envelope"),
    );
    assert_eq!(
        request.occurrence_digest(),
        [
            0xaf, 0x8f, 0xa2, 0x96, 0x55, 0x08, 0xfe, 0xab, 0x3f, 0x27, 0xf7, 0x1f, 0x44, 0xd1,
            0xd4, 0x3a, 0x91, 0x80, 0xc8, 0x3b, 0x5b, 0xe5, 0x4b, 0xc1, 0xd7, 0x29, 0xe8, 0xd7,
            0x1c, 0xf2, 0x79, 0x0c,
        ]
    );
}

#[test]
fn consumer_values_enforce_byte_boundaries() {
    assert!(ResourceConsumerKind::new("").is_err());
    assert!(ResourceConsumerKind::new("k".repeat(64)).is_ok());
    assert!(ResourceConsumerKind::new("k".repeat(65)).is_err());

    assert!(ResourceConsumerIdentity::try_from_vec(Vec::new()).is_err());
    assert!(ResourceConsumerIdentity::try_from_vec(vec![1; 512]).is_ok());
    assert!(ResourceConsumerIdentity::try_from_vec(vec![1; 513]).is_err());
}

#[test]
fn page_size_ttl_and_holder_enforce_inclusive_limits() {
    assert!(ResourcePageSize::new(0).is_err());
    assert_eq!(ResourcePageSize::new(1).expect("minimum page").get(), 1);
    assert_eq!(
        ResourcePageSize::new(1_000).expect("maximum page").get(),
        1_000
    );
    assert!(ResourcePageSize::new(1_001).is_err());

    assert!(ResourceLeaseHolder::new("").is_err());
    assert!(ResourceLeaseHolder::new("h".repeat(256)).is_ok());
    assert!(ResourceLeaseHolder::new("h".repeat(257)).is_err());
    assert!(ResourceLeaseTtl::new(Duration::from_millis(999)).is_err());
    assert_eq!(
        ResourceLeaseTtl::new(Duration::from_secs(1))
            .expect("minimum TTL")
            .get(),
        Duration::from_secs(1)
    );
    assert!(ResourceLeaseTtl::new(Duration::from_hours(24)).is_ok());
    assert!(ResourceLeaseTtl::new(Duration::from_hours(24) + Duration::from_secs(1)).is_err());
}

#[test]
fn occurrence_and_envelope_values_enforce_inclusive_limits() {
    assert!(EventOccurrenceNamespace::new("").is_err());
    assert!(EventOccurrenceNamespace::new("n".repeat(128)).is_ok());
    assert!(EventOccurrenceNamespace::new("n".repeat(129)).is_err());
    assert!(EventOccurrenceKey::try_from_vec(Vec::new()).is_err());
    assert!(EventOccurrenceKey::try_from_vec(vec![3; 1_024]).is_ok());
    assert!(EventOccurrenceKey::try_from_vec(vec![3; 1_025]).is_err());
    assert!(EventEnvelope::try_from_vec(1, Vec::new()).is_err());
    assert!(EventEnvelope::try_from_vec(1, vec![4; 1024 * 1024]).is_ok());
    assert!(EventEnvelope::try_from_vec(1, vec![4; 1024 * 1024 + 1]).is_err());
}

#[test]
fn envelope_replay_identity_includes_schema_and_exact_payload_bytes() {
    let accepted =
        EventEnvelope::try_from_vec(7, b"canonical-event".to_vec()).expect("valid envelope");
    let replay =
        EventEnvelope::try_from_vec(7, b"canonical-event".to_vec()).expect("valid replay envelope");
    let different_schema = EventEnvelope::try_from_vec(8, b"canonical-event".to_vec())
        .expect("valid alternate schema envelope");
    let different_bytes = EventEnvelope::try_from_vec(7, b"canonical-evenu".to_vec())
        .expect("valid alternate payload envelope");

    assert_eq!(accepted, replay);
    assert_ne!(accepted, different_schema);
    assert_ne!(accepted, different_bytes);
    assert_eq!(accepted.schema_version(), 7);
    assert_eq!(accepted.canonical_payload(), b"canonical-event");
    assert_eq!(accepted.digest(), replay.digest());
}

#[test]
fn persisted_closed_enums_round_trip_and_reject_unknown_values() {
    for state in [
        ResourceSubscriptionState::Active,
        ResourceSubscriptionState::Disabled,
        ResourceSubscriptionState::Tombstoned,
    ] {
        assert_eq!(
            ResourceSubscriptionState::from_str(state.as_str()).expect("known subscription state"),
            state
        );
    }
    assert!(ResourceSubscriptionState::from_str("paused").is_err());

    for state in [ResourceEventState::Pending, ResourceEventState::Complete] {
        assert_eq!(
            ResourceEventState::from_str(state.as_str()).expect("known event state"),
            state
        );
    }
    assert!(ResourceEventState::from_str("abandoned").is_err());

    for reason in [
        TerminalDeliveryIneligibility::SubscriptionDisabled,
        TerminalDeliveryIneligibility::SubscriptionTombstoned,
        TerminalDeliveryIneligibility::ConsumerUnavailable,
        TerminalDeliveryIneligibility::UnsupportedEnvelopeSchema,
    ] {
        assert_eq!(
            TerminalDeliveryIneligibility::from_str(reason.as_str())
                .expect("known terminal reason"),
            reason
        );
    }
    assert!(TerminalDeliveryIneligibility::from_str("retry_later").is_err());
}

#[test]
fn opaque_ids_and_tokens_round_trip_without_debug_disclosure() {
    let shared_id = SharedResourceId::from_bytes([1; 16]);
    let subscription_id = ResourceSubscriptionId::from_bytes([2; 16]);
    let event_id = ResourceEventId::from_bytes([3; 16]);
    let delivery_id = ResourceDeliveryId::from_bytes([4; 16]);
    assert_eq!(shared_id.into_bytes(), [1; 16]);
    assert_eq!(subscription_id.into_bytes(), [2; 16]);
    assert_eq!(event_id.into_bytes(), [3; 16]);
    assert_eq!(delivery_id.into_bytes(), [4; 16]);

    let source_token =
        ResourceSourceLeaseToken::from_claim_bytes([0xab; 16], ResourceLeaseGeneration::new(9));
    let delivery_token =
        ResourceDeliveryClaimToken::from_claim_bytes([0xcd; 16], ResourceLeaseGeneration::new(10));
    let handoff_token =
        ResourceHandoffClaimToken::from_claim_bytes([0xef; 16], ResourceLeaseGeneration::new(11));
    assert_eq!(source_token.claim_id().into_bytes(), [0xab; 16]);
    assert_eq!(delivery_token.claim_id().into_bytes(), [0xcd; 16]);
    assert_eq!(handoff_token.claim_id().into_bytes(), [0xef; 16]);
    assert!(!format!("{source_token:?}").contains("abab"));
    assert!(!format!("{delivery_token:?}").contains("cdcd"));
    assert!(!format!("{handoff_token:?}").contains("efef"));
}

#[test]
fn debug_output_redacts_identity_and_payload_bytes() {
    let configuration_secret = "configuration-secret-42";
    let slot_secret = "slot-secret-43";
    let identity = SharedResourceIdentity::new(
        ResourceKind::new("telegram.bot").expect("valid kind"),
        ResourceCompatibilityVersion::new(1),
        ResourceConfigurationIdentity::try_from_vec(configuration_secret.as_bytes().to_vec())
            .expect("valid configuration"),
        ResourceSlotIdentity::try_from_vec(slot_secret.as_bytes().to_vec()).expect("valid slot"),
    );
    let envelope_secret = "event-payload-secret-44";
    let envelope = EventEnvelope::try_from_vec(1, envelope_secret.as_bytes().to_vec())
        .expect("valid envelope");

    let identity_debug = format!("{identity:?}");
    let envelope_debug = format!("{envelope:?}");
    assert!(!identity_debug.contains(configuration_secret));
    assert!(!identity_debug.contains(slot_secret));
    assert!(!envelope_debug.contains(envelope_secret));
    assert!(identity_debug.contains("configuration_len"));
    assert!(envelope_debug.contains("payload_len"));
}

#[test]
fn lease_generation_overflow_fails_closed() {
    assert_eq!(
        ResourceLeaseGeneration::new(41)
            .checked_next()
            .expect("generation has capacity")
            .get(),
        42
    );
    assert!(
        ResourceLeaseGeneration::new(u64::MAX)
            .checked_next()
            .is_err()
    );
}
