//! Credential authority and closed state shapes in the paired baseline.
//!
//! Structural backend parity is exercised by `schema_parity_postgres`; these
//! catalog checks pin the credential protocol's required identities and ranges.

use std::path::Path;

fn catalogs() -> [(String, &'static str); 2] {
    ["sqlite", "postgres"].map(|backend| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("migrations/{backend}/0006_credentials.sql"));
        let sql = std::fs::read_to_string(path).expect("read credential baseline");
        (
            sql.split_whitespace().collect::<Vec<_>>().join(" "),
            backend,
        )
    })
}

#[test]
fn claims_and_incidents_have_scoped_authority_and_durable_incident_identity() {
    for (sql, backend) in catalogs() {
        for invariant in [
            "CREATE TABLE credential_refresh_claims (",
            "CREATE TABLE credential_refresh_incidents (",
            "CONSTRAINT pk_credential_refresh_claims PRIMARY KEY (org_id, workspace_id, credential_id)",
            "CONSTRAINT pk_credential_refresh_incidents PRIMARY KEY (claim_id)",
            "CONSTRAINT fk_credential_refresh_claims__credentials",
            "CONSTRAINT fk_credential_refresh_incidents__credentials",
            "REFERENCES credentials (org_id, workspace_id, id) ON DELETE CASCADE",
            "CONSTRAINT ck_credential_refresh_claims__generation CHECK (generation >= 0)",
            "CONSTRAINT ck_credential_refresh_incidents__generation CHECK (generation >= 0)",
            "ix_credential_refresh_claims__expires_at",
            "ix_credential_refresh_incidents__credential_id_detected_at",
        ] {
            assert!(
                sql.contains(invariant),
                "{backend} lacks credential authority invariant {invariant}"
            );
        }
    }
}

#[test]
fn retry_gate_and_material_epochs_are_structural_closed_state() {
    for (sql, backend) in catalogs() {
        for invariant in [
            "CONSTRAINT ck_credentials__material_epoch",
            "CONSTRAINT ck_credentials__admission_epoch",
            "material_epoch >= 1",
            "admission_epoch >= 1",
            "CONSTRAINT ck_credentials__refresh_retry_gate CHECK (",
            "CONSTRAINT ck_credentials__record_shape CHECK (",
            "refresh_retry_not_before",
            "refresh_retry_mode",
            "refresh_retry_phase",
            "refresh_retry_kind",
            "refresh_retry_diagnostic_code",
            "'never'",
            "'not_before'",
            "'before_dispatch'",
            "'provider_confirmed_not_applied'",
            "'transient_network'",
            "'provider_unavailable'",
            "'protocol_error'",
        ] {
            assert!(
                sql.contains(invariant),
                "{backend} lacks retry/material invariant {invariant}"
            );
        }
        assert!(
            !sql.contains("$.refresh_retry"),
            "{backend} user metadata must not grant retry authority"
        );
    }
}

#[test]
fn reconciliation_declares_unresolved_shape_and_decision_and_digest_checks() {
    for (sql, backend) in catalogs() {
        for invariant in [
            "CONSTRAINT ck_credential_refresh_incidents__adjudication CHECK (",
            "adjudicated_at IS NULL",
            "adjudication_decision IS NULL",
            "adjudication_evidence IS NULL",
            "adjudication_evidence_digest IS NULL",
            "adjudicated_at IS NOT NULL",
            "adjudication_evidence IS NOT NULL",
            "'provider_applied'",
            "'provider_not_applied'",
            "'provider_revoked'",
            "'provider_not_revoked'",
        ] {
            assert!(
                sql.contains(invariant),
                "{backend} lacks reconciliation invariant {invariant}"
            );
        }
        let digest_check = if backend == "postgres" {
            "octet_length(adjudication_evidence_digest) = 32"
        } else {
            "length(adjudication_evidence_digest) = 32"
        };
        assert!(
            sql.contains(digest_check),
            "{backend} evidence must carry a SHA-256 digest"
        );
    }
}
