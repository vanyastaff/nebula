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
            "material_transition",
        ]
    );
    assert_eq!(field_names(tombstone), ["expected_version"]);
}

#[test]
fn replacement_constructor_requires_an_explicit_material_transition() {
    let implementation = DTO_SOURCE
        .split_once("impl CredentialReplacement {")
        .expect("CredentialReplacement implementation must exist")
        .1
        .split_once("\n}")
        .expect("CredentialReplacement implementation must have a body")
        .0;

    assert!(
        implementation.contains("material_transition: CredentialMaterialTransition,"),
        "the constructor must explicitly choose whether material authority advances"
    );
    assert!(
        !implementation.contains("CredentialMaterialTransition::Preserve"),
        "the constructor must not hide a preserve-authority default"
    );
    assert!(
        !implementation.contains("with_material_transition"),
        "a post-construction builder would make authority-transition intent non-atomic"
    );
}

#[test]
fn live_row_constructor_requires_explicit_retry_gate_state() {
    let implementation = DTO_SOURCE
        .split_once("impl StoredLiveCredential {")
        .expect("StoredLiveCredential implementation must exist")
        .1
        .split_once("\n}")
        .expect("StoredLiveCredential implementation must have a body")
        .0;

    assert!(
        implementation.contains("refresh_retry_gate: Option<RefreshRetryGate>,"),
        "physical-row validation must receive decoded retry-gate state atomically"
    );
    assert!(
        !implementation.contains("refresh_retry_gate: None"),
        "the validation boundary must not silently drop a persisted retry gate"
    );
    assert!(
        !implementation.contains("with_refresh_retry_gate"),
        "a post-validation builder could silently omit persisted retry-gate state"
    );
}

#[test]
fn tombstone_record_cannot_represent_live_only_data() {
    let tombstone = struct_body(DTO_SOURCE, "StoredTombstonedCredential");
    let fields = field_names(tombstone);

    for forbidden in [
        "data",
        "name",
        "expires_at",
        "reauth_required",
        "metadata",
        "refresh_retry_gate",
    ] {
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

#[test]
fn refresh_retry_state_is_exposed_only_as_one_atomic_snapshot() {
    assert!(STORE_SOURCE.contains("async fn refresh_retry_snapshot("));
    assert!(
        !STORE_SOURCE.contains("async fn refresh_retry_admission("),
        "an admission-only method invites a TOCTOU combination with get"
    );
}
