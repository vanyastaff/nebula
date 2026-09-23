//! Service-owned identity and fencing context for interactive continuations.

use std::time::Duration;

use base64::Engine as _;
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::PendingState;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AcquisitionIntent {
    Create {
        credential_key: String,
        attempt_id: String,
    },
    ReauthorizeExisting {
        credential_id: String,
        observed_version: u64,
        observed_material_epoch: u64,
        credential_key: String,
        attempt_id: String,
    },
}

impl AcquisitionIntent {
    pub(crate) fn create_for_key(credential_key: &str) -> Self {
        Self::Create {
            credential_key: credential_key.to_owned(),
            attempt_id: generate_attempt_id(),
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
            Self::Create {
                credential_key,
                attempt_id,
            } => {
                credential_key.zeroize();
                attempt_id.zeroize();
            },
            Self::ReauthorizeExisting {
                credential_id,
                observed_version,
                observed_material_epoch,
                credential_key,
                attempt_id,
            } => {
                credential_id.zeroize();
                *observed_version = 0;
                *observed_material_epoch = 0;
                credential_key.zeroize();
                attempt_id.zeroize();
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

#[derive(Serialize, Deserialize)]
pub(crate) struct AcquisitionPending<P: Zeroize> {
    intent: AcquisitionIntent,
    protocol: P,
}

impl<P: Zeroize> AcquisitionPending<P> {
    pub(crate) fn new(intent: AcquisitionIntent, protocol: P) -> Self {
        Self { intent, protocol }
    }

    pub(crate) fn intent_matches(&self, expected: &AcquisitionExpectation) -> bool {
        &self.intent.expectation() == expected
    }

    pub(crate) fn protocol(&self) -> &P {
        &self.protocol
    }

    pub(crate) fn intent(&self) -> AcquisitionIntent {
        self.intent.clone()
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

fn generate_attempt_id() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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

    #[test]
    fn reauthorization_intent_is_exact_and_debug_is_redacted() {
        let intent = AcquisitionIntent::ReauthorizeExisting {
            credential_id: "credential-canary".to_owned(),
            observed_version: 7,
            observed_material_epoch: 3,
            credential_key: "provider.secret-key".to_owned(),
            attempt_id: "attempt-canary".to_owned(),
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
