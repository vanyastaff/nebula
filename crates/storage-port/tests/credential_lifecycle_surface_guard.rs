const DTO_SOURCE: &str = include_str!("../src/dto/credential.rs");
const STORE_SOURCE: &str = include_str!("../src/store/credential.rs");
const LIB_SOURCE: &str = include_str!("../src/lib.rs");

fn struct_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("pub struct {name} {{");
    let after_marker = source
        .split_once(&marker)
        .unwrap_or_else(|| panic!("{name} must be a public private-field DTO"))
        .1;
    after_marker
        .split_once("\n}")
        .unwrap_or_else(|| panic!("{name} must have a braced body"))
        .0
}

fn field_names(body: &str) -> Vec<&str> {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("///") || trimmed.starts_with("#[") {
                return None;
            }
            trimmed
                .split_once(':')
                .map(|(name, _)| name.trim_start_matches("pub ").trim())
        })
        .collect()
}

#[test]
fn mutation_dtos_have_private_fields_and_no_authority_or_identity_smuggling() {
    let create = struct_body(DTO_SOURCE, "CredentialCreate");
    let replacement = struct_body(DTO_SOURCE, "CredentialReplacement");
    let tombstone = struct_body(DTO_SOURCE, "CredentialTombstone");

    for (name, body) in [
        ("CredentialCreate", create),
        ("CredentialReplacement", replacement),
        ("CredentialTombstone", tombstone),
    ] {
        assert!(
            !body
                .lines()
                .any(|line| line.trim_start().starts_with("pub")),
            "{name} fields must stay private"
        );
    }

    assert_eq!(
        field_names(create),
        [
            "credential_key",
            "data",
            "state_kind",
            "state_version",
            "name",
            "expires_at",
            "reauth_required",
            "metadata",
        ]
    );
    assert_eq!(
        field_names(replacement),
        [
            "expected_version",
            "data",
            "state_kind",
            "state_version",
            "name",
            "expires_at",
            "reauth_required",
            "metadata",
        ]
    );
    assert_eq!(field_names(tombstone), ["expected_version"]);
}

#[test]
fn tombstone_record_cannot_represent_live_only_data() {
    let tombstone = struct_body(DTO_SOURCE, "StoredTombstonedCredential");
    let fields = field_names(tombstone);

    for forbidden in ["data", "name", "expires_at", "reauth_required", "metadata"] {
        assert!(
            !fields.contains(&forbidden),
            "tombstone must not carry live-only field {forbidden}"
        );
    }
}

#[test]
fn live_record_debug_never_reports_secret_length() {
    let marker = "impl fmt::Debug for StoredLiveCredential";
    let implementation = DTO_SOURCE
        .split_once(marker)
        .unwrap_or_else(|| panic!("{marker} must exist"))
        .1
        .split_once("\n}")
        .unwrap_or_else(|| panic!("{marker} must have a body"))
        .0;

    assert!(
        !implementation.contains(".len()"),
        "secret-bearing record Debug must have one length-independent shape"
    );
}

#[test]
fn obsolete_write_modes_and_open_ended_errors_are_absent() {
    for forbidden in [
        "CredentialWriteMode",
        "AuditFailure",
        "InvalidRequest",
        "Backend(",
    ] {
        assert!(
            !DTO_SOURCE.contains(forbidden)
                && !STORE_SOURCE.contains(forbidden)
                && !LIB_SOURCE.contains(forbidden),
            "obsolete or open-ended credential surface remains nameable: {forbidden}"
        );
    }

    for forbidden_method in ["async fn put(", "async fn delete("] {
        assert!(
            !STORE_SOURCE.contains(forbidden_method),
            "obsolete credential mutation remains nameable: {forbidden_method}"
        );
    }
}
