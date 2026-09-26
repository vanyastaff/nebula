use nebula_storage_port::{CredentialMaterialEpoch, CredentialVersion};

use super::*;

fn open(reauth_required: bool) -> CredentialOperationStatus {
    CredentialOperationStatus::Open {
        version: CredentialVersion::MIN,
        material_epoch: CredentialMaterialEpoch::MIN,
        reauth_required,
    }
}

fn incident() -> CredentialIncidentRef {
    serde_json::from_value(serde_json::json!("00000000-0000-0000-0000-000000000000"))
        .expect("an incident id deserializes from its UUID spelling")
}

#[test]
fn every_operation_status_has_one_use_decision() {
    use CredentialOperationKind::{LegacyUnclassified, Refresh, Revoke};

    let cases = [
        (open(false), CredentialUseAvailability::Admit),
        (
            open(true),
            CredentialUseAvailability::Denied(CredentialUseDenial::ReauthRequired),
        ),
        (
            CredentialOperationStatus::InFlight { operation: Refresh },
            CredentialUseAvailability::RefreshCrossing,
        ),
        (
            CredentialOperationStatus::InFlight { operation: Revoke },
            CredentialUseAvailability::Denied(CredentialUseDenial::OperationInFlight {
                operation: Revoke,
            }),
        ),
        (
            CredentialOperationStatus::InFlight {
                operation: LegacyUnclassified,
            },
            CredentialUseAvailability::Denied(CredentialUseDenial::OperationInFlight {
                operation: LegacyUnclassified,
            }),
        ),
    ];
    for (status, expected) in cases {
        assert_eq!(classify_use(status), expected, "{status:?}");
    }

    // An unknown outcome is never joined, whichever operation left it.
    for operation in [Refresh, Revoke, LegacyUnclassified] {
        assert_eq!(
            classify_use(CredentialOperationStatus::ReconciliationRequired {
                operation,
                incident: incident(),
            }),
            CredentialUseAvailability::Denied(CredentialUseDenial::Reconciliation {
                operation,
                incident: incident(),
            })
        );
    }
}
