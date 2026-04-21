//! Credential instance identifier.
//!
//! `CredentialId` is the system-generated ULID that identifies a specific
//! credential instance in storage (Stripe-style prefix: `cred_01J9ABCDEF...`).
//! Convention: `FooId` = system-generated ULID, `FooKey` = author-defined string.

use domain_key::define_ulid;

// System-generated ULID for credential instances.
define_ulid!(pub CredentialIdDomain => CredentialId, prefix = "cred");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_id_new_is_unique() {
        let a = CredentialId::new();
        let b = CredentialId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn credential_id_display_has_prefix() {
        let id = CredentialId::new();
        let s = id.to_string();
        assert!(s.starts_with("cred_"), "expected 'cred_' prefix, got: {s}");
    }

    #[test]
    fn credential_id_parse_roundtrip() {
        let id = CredentialId::new();
        let s = id.to_string();
        let parsed: CredentialId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn credential_id_serde_json_roundtrip() {
        let id = CredentialId::new();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: CredentialId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn credential_id_parse_invalid_string_returns_error() {
        let result: Result<CredentialId, _> = "not-a-ulid".parse();
        assert!(result.is_err());
    }

    #[test]
    fn credential_id_copy_semantics() {
        let id1 = CredentialId::new();
        let id2 = id1; // Copy
        assert_eq!(id1, id2);
    }
}
