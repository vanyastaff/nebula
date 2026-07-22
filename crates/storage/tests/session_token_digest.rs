use nebula_storage::session_token::session_token_digest;

#[test]
fn lookup_digest_is_stable_domain_separated_sha256() {
    let digest = session_token_digest(b"SESSION_TOKEN_CANARY-2a73");

    assert_eq!(
        digest.as_bytes(),
        &[
            0x08, 0x64, 0x6a, 0xa6, 0xc9, 0xa6, 0xbb, 0x66, 0x0b, 0x27, 0x68, 0xf3, 0xca, 0x5e,
            0x70, 0x41, 0xc2, 0x55, 0x5b, 0x2d, 0xff, 0x7b, 0x35, 0xe0, 0x7f, 0xee, 0x4a, 0x9d,
            0x5a, 0x3b, 0xc7, 0xc0,
        ]
    );
    assert_ne!(digest, session_token_digest(b"SESSION_TOKEN_CANARY-2a74"));
}

#[test]
fn digest_debug_never_prints_lookup_material() {
    let digest = session_token_digest(b"SESSION_TOKEN_CANARY-2a73");
    let debug = format!("{digest:?}");

    assert_eq!(debug, "SessionTokenDigest([redacted])");
    assert!(!debug.contains("ef94b432"));
}
