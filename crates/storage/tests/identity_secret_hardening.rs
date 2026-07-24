use std::sync::Arc;

use nebula_crypto::EncryptionKey;
use nebula_storage::credential::{KeyProvider, KeySnapshot, ProviderError};
use nebula_storage::identity_secret::{
    IdentitySecretCodec, OpenedIdentitySecret, TotpSecretPurpose,
};

struct TestKeyProvider {
    key: Arc<EncryptionKey>,
    version: &'static str,
}

impl TestKeyProvider {
    fn new(bytes: [u8; 32], version: &'static str) -> Self {
        Self {
            key: Arc::new(EncryptionKey::from_bytes(bytes)),
            version,
        }
    }
}

impl KeyProvider for TestKeyProvider {
    fn current(&self) -> Result<KeySnapshot, ProviderError> {
        KeySnapshot::new(self.version, Arc::clone(&self.key))
    }
}

fn codec(bytes: [u8; 32], version: &'static str) -> IdentitySecretCodec {
    IdentitySecretCodec::new(Arc::new(TestKeyProvider::new(bytes, version)))
        .expect("valid test key")
}

#[test]
fn active_totp_round_trips_without_rotation() {
    let codec = codec([0x11; 32], "identity-key-1");
    let user_id = [0x21; 16];
    let envelope = codec
        .seal_totp_seed(
            TotpSecretPurpose::Active,
            &user_id,
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        )
        .expect("seal active seed");

    let OpenedIdentitySecret {
        plaintext,
        replacement_envelope,
    } = codec
        .open_totp_seed(TotpSecretPurpose::Active, &user_id, &envelope)
        .expect("open active seed");

    assert_eq!(plaintext.as_slice(), b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP");
    assert!(replacement_envelope.is_none());
    assert_ne!(envelope.as_slice(), plaintext.as_slice());
}

#[test]
fn envelope_is_bound_to_user_and_lifecycle_purpose() {
    let codec = codec([0x22; 32], "identity-key-1");
    let owner = [0x31; 16];
    let other_user = [0x32; 16];
    let envelope = codec
        .seal_totp_seed(
            TotpSecretPurpose::EnrollmentCandidate,
            &owner,
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        )
        .expect("seal candidate seed");

    assert!(
        codec
            .open_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                &other_user,
                &envelope,
            )
            .is_err(),
        "copying a candidate envelope to another user must fail authentication"
    );
    assert!(
        codec
            .open_totp_seed(TotpSecretPurpose::Active, &owner, &envelope)
            .is_err(),
        "a pending candidate cannot be installed as the active factor unchanged"
    );
}

#[test]
fn tamper_and_unrelated_key_fail_closed() {
    let owner = [0x41; 16];
    let writer = codec([0x33; 32], "identity-key-1");
    let envelope = writer
        .seal_totp_seed(
            TotpSecretPurpose::Active,
            &owner,
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        )
        .expect("seal active seed");

    let mut tampered = envelope.clone();
    let last = tampered
        .last_mut()
        .expect("serialized envelope is non-empty");
    *last ^= 1;
    assert!(
        writer
            .open_totp_seed(TotpSecretPurpose::Active, &owner, &tampered)
            .is_err()
    );

    let unrelated = codec([0x44; 32], "identity-key-2");
    assert!(
        unrelated
            .open_totp_seed(TotpSecretPurpose::Active, &owner, &envelope)
            .is_err(),
        "an envelope whose key id is not current or explicitly legacy must fail"
    );
}

#[test]
fn explicit_legacy_key_open_returns_current_key_replacement() {
    let owner = [0x51; 16];
    let old_key = Arc::new(EncryptionKey::from_bytes([0x55; 32]));
    let old_codec = IdentitySecretCodec::new(Arc::new(TestKeyProvider {
        key: Arc::clone(&old_key),
        version: "identity-key-1",
    }))
    .expect("old codec");
    let old_envelope = old_codec
        .seal_totp_seed(
            TotpSecretPurpose::Active,
            &owner,
            b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        )
        .expect("seal old envelope");

    let rotated = IdentitySecretCodec::with_legacy_keys(
        Arc::new(TestKeyProvider::new([0x66; 32], "identity-key-2")),
        vec![("identity-key-1".to_owned(), old_key)],
    )
    .expect("rotation codec");
    let opened = rotated
        .open_totp_seed(TotpSecretPurpose::Active, &owner, &old_envelope)
        .expect("open legacy envelope");
    assert_eq!(
        opened.plaintext.as_slice(),
        b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
    );
    let replacement = opened
        .replacement_envelope
        .expect("legacy opening must request persisted rotation");

    let current_only = codec([0x66; 32], "identity-key-2");
    let reopened = current_only
        .open_totp_seed(TotpSecretPurpose::Active, &owner, &replacement)
        .expect("replacement opens under current key only");
    assert_eq!(
        reopened.plaintext.as_slice(),
        b"JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
    );
    assert!(reopened.replacement_envelope.is_none());
}
