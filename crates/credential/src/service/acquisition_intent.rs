//! Service-owned identity and fencing context for interactive continuations.
//!
//! Version 1 readers accept the legacy raw protocol payload as a Create
//! continuation. Create also continues to serialize as that raw payload, so
//! old and new readers remain bidirectionally compatible during a rolling
//! deploy. Only the future, currently unexposed ReauthorizeExisting path emits
//! the versioned service envelope and therefore requires upgraded readers.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroize;

use crate::PendingState;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AcquisitionIntent {
    Create {
        credential_key: String,
    },
    ReauthorizeExisting {
        credential_id: String,
        observed_version: u64,
        observed_material_epoch: u64,
        credential_key: String,
    },
}

impl AcquisitionIntent {
    pub(crate) fn create_for_key(credential_key: &str) -> Self {
        Self::Create {
            credential_key: credential_key.to_owned(),
        }
    }

    pub(crate) fn expectation(&self) -> AcquisitionExpectation {
        match self {
            Self::Create { credential_key, .. } => AcquisitionExpectation::Create {
                credential_key: credential_key.clone(),
            },
            Self::ReauthorizeExisting {
                credential_id,
                observed_version,
                observed_material_epoch,
                credential_key,
                ..
            } => AcquisitionExpectation::ReauthorizeExisting {
                credential_id: credential_id.clone(),
                observed_version: *observed_version,
                observed_material_epoch: *observed_material_epoch,
                credential_key: credential_key.clone(),
            },
        }
    }
}

impl std::fmt::Debug for AcquisitionIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Create { .. } => "AcquisitionIntent::Create([redacted])",
            Self::ReauthorizeExisting { .. } => {
                "AcquisitionIntent::ReauthorizeExisting([redacted])"
            },
        })
    }
}

impl Zeroize for AcquisitionIntent {
    fn zeroize(&mut self) {
        match self {
            Self::Create { credential_key } => {
                credential_key.zeroize();
            },
            Self::ReauthorizeExisting {
                credential_id,
                observed_version,
                observed_material_epoch,
                credential_key,
            } => {
                credential_id.zeroize();
                *observed_version = 0;
                *observed_material_epoch = 0;
                credential_key.zeroize();
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum AcquisitionExpectation {
    Create {
        credential_key: String,
    },
    ReauthorizeExisting {
        credential_id: String,
        observed_version: u64,
        observed_material_epoch: u64,
        credential_key: String,
    },
}

pub(crate) struct AcquisitionPending<P: Zeroize> {
    intent: Option<AcquisitionIntent>,
    protocol: P,
}

impl<P: Zeroize> AcquisitionPending<P> {
    pub(crate) fn new(intent: AcquisitionIntent, protocol: P) -> Self {
        Self {
            intent: Some(intent),
            protocol,
        }
    }

    pub(crate) fn intent_matches(&self, expected: &AcquisitionExpectation) -> bool {
        match &self.intent {
            Some(intent) => &intent.expectation() == expected,
            None => matches!(expected, AcquisitionExpectation::Create { .. }),
        }
    }

    pub(crate) fn protocol(&self) -> &P {
        &self.protocol
    }

    pub(crate) fn intent_for_next(
        &self,
        expected: &AcquisitionExpectation,
    ) -> Option<AcquisitionIntent> {
        self.intent.clone().or_else(|| match expected {
            AcquisitionExpectation::Create { credential_key } => {
                Some(AcquisitionIntent::create_for_key(credential_key))
            },
            AcquisitionExpectation::ReauthorizeExisting { .. } => None,
        })
    }
}

impl<P: Zeroize> Zeroize for AcquisitionPending<P> {
    fn zeroize(&mut self) {
        self.intent.zeroize();
        self.protocol.zeroize();
    }
}

impl<P: Zeroize> Drop for AcquisitionPending<P> {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl<P: Zeroize> zeroize::ZeroizeOnDrop for AcquisitionPending<P> {}

impl<P> PendingState for AcquisitionPending<P>
where
    P: PendingState,
{
    const KIND: &'static str = P::KIND;

    fn expires_in(&self) -> Duration {
        self.protocol.expires_in()
    }
}

impl<P: Zeroize> std::fmt::Debug for AcquisitionPending<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AcquisitionPending([redacted])")
    }
}

#[derive(Serialize, Deserialize)]
struct VersionedPending<P> {
    version: u8,
    intent: AcquisitionIntent,
    protocol: P,
}

#[derive(Serialize)]
struct VersionedPendingRef<'a, P> {
    version: u8,
    intent: &'a AcquisitionIntent,
    protocol: &'a P,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PendingWire<P> {
    Versioned(VersionedPending<P>),
    Legacy(P),
}

impl<P> Serialize for AcquisitionPending<P>
where
    P: Serialize + Zeroize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.intent.as_ref() {
            Some(intent @ AcquisitionIntent::ReauthorizeExisting { .. }) => VersionedPendingRef {
                version: 1,
                intent,
                protocol: &self.protocol,
            }
            .serialize(serializer),
            Some(AcquisitionIntent::Create { .. }) | None => self.protocol.serialize(serializer),
        }
    }
}

impl<'de, P> Deserialize<'de> for AcquisitionPending<P>
where
    P: Deserialize<'de> + Zeroize,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match PendingWire::<P>::deserialize(deserializer)? {
            PendingWire::Versioned(versioned) if versioned.version == 1 => Ok(Self {
                intent: Some(versioned.intent),
                protocol: versioned.protocol,
            }),
            PendingWire::Versioned(_) => Err(serde::de::Error::custom(
                "unsupported credential acquisition pending version",
            )),
            PendingWire::Legacy(protocol) => Ok(Self {
                intent: None,
                protocol,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use zeroize::Zeroize;

    use super::{AcquisitionExpectation, AcquisitionIntent, AcquisitionPending};

    #[derive(serde::Serialize, serde::Deserialize)]
    struct ProtocolPending(String);

    impl Zeroize for ProtocolPending {
        fn zeroize(&mut self) {
            self.0.zeroize();
        }
    }

    impl Drop for ProtocolPending {
        fn drop(&mut self) {
            self.zeroize();
        }
    }

    impl zeroize::ZeroizeOnDrop for ProtocolPending {}

    impl crate::PendingState for ProtocolPending {
        const KIND: &'static str = "test";

        fn expires_in(&self) -> std::time::Duration {
            std::time::Duration::from_mins(1)
        }
    }

    #[test]
    fn create_wire_stays_legacy_while_reauthorization_is_versioned() {
        let legacy: AcquisitionPending<ProtocolPending> =
            serde_json::from_str(r#""legacy-secret""#).expect("legacy payload decodes");
        let create = AcquisitionExpectation::Create {
            credential_key: "provider.test".to_owned(),
        };
        assert!(legacy.intent_matches(&create));
        assert!(
            !legacy.intent_matches(&AcquisitionExpectation::ReauthorizeExisting {
                credential_id: "credential".to_owned(),
                observed_version: 1,
                observed_material_epoch: 1,
                credential_key: "provider.test".to_owned(),
            })
        );
        let next_create = AcquisitionPending::new(
            legacy
                .intent_for_next(&create)
                .expect("legacy create intent promotes for the next step"),
            ProtocolPending("next-protocol-secret".to_owned()),
        );
        let next_create_wire =
            crate::serde_secret::expose_for_serialization(|| serde_json::to_value(&next_create))
                .expect("next create payload serializes");
        assert_eq!(next_create_wire, "next-protocol-secret");

        let create_pending = AcquisitionPending::new(
            AcquisitionIntent::create_for_key("provider.test"),
            ProtocolPending("protocol-secret".to_owned()),
        );
        let create_wire =
            crate::serde_secret::expose_for_serialization(|| serde_json::to_value(&create_pending))
                .expect("create payload serializes");
        assert_eq!(create_wire, "protocol-secret");

        let reauthorization = AcquisitionPending::new(
            AcquisitionIntent::ReauthorizeExisting {
                credential_id: "credential".to_owned(),
                observed_version: 1,
                observed_material_epoch: 1,
                credential_key: "provider.test".to_owned(),
            },
            ProtocolPending("protocol-secret".to_owned()),
        );
        let reauthorization_wire = crate::serde_secret::expose_for_serialization(|| {
            serde_json::to_value(&reauthorization)
        })
        .expect("reauthorization payload serializes");
        assert_eq!(reauthorization_wire["version"], 1);
        assert_eq!(reauthorization_wire["protocol"], "protocol-secret");
    }

    #[test]
    fn reauthorization_intent_is_exact_and_debug_is_redacted() {
        let intent = AcquisitionIntent::ReauthorizeExisting {
            credential_id: "credential-canary".to_owned(),
            observed_version: 7,
            observed_material_epoch: 3,
            credential_key: "provider.secret-key".to_owned(),
        };
        let pending = AcquisitionPending::new(intent.clone(), ProtocolPending("secret".into()));

        assert!(pending.intent_matches(&intent.expectation()));
        assert!(
            !pending.intent_matches(&AcquisitionExpectation::ReauthorizeExisting {
                credential_id: "substituted".to_owned(),
                observed_version: 7,
                observed_material_epoch: 3,
                credential_key: "provider.secret-key".to_owned(),
            })
        );
        assert!(
            !pending.intent_matches(&AcquisitionExpectation::ReauthorizeExisting {
                credential_id: "credential-canary".to_owned(),
                observed_version: 7,
                observed_material_epoch: 3,
                credential_key: "provider.substituted".to_owned(),
            })
        );
        assert_eq!(
            format!("{intent:?}"),
            "AcquisitionIntent::ReauthorizeExisting([redacted])"
        );
        assert_eq!(format!("{pending:?}"), "AcquisitionPending([redacted])");
    }
}
