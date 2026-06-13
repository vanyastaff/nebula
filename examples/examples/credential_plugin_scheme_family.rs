//! Open-world plugin auth scheme (F3).
//!
//! A third-party crate teaches Nebula a brand-new auth protocol — here RFC 9421
//! HTTP Message Signatures — by defining its own [`SchemeFamily`] and
//! [`AuthScheme`], with **zero edits to `nebula-core` or any framework crate**.
//! The framework consumes the plugin's mechanics (wire-egress shape, legitimate
//! refresh strategies) through the *open* `SchemeFamily` trait — it never
//! `match`es a closed family enum, which is exactly why no framework change is
//! required to add a protocol the 1.0 release never imagined.
//!
//! Run: `cargo run -p nebula-examples --example credential_plugin_scheme_family`

use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, PublicScheme, RefreshStrategyKind, SchemeFamily,
};

/// The plugin's mechanics family — RFC 9421 HTTP Message Signatures.
///
/// Not a variant of any framework enum: a local zero-sized marker type
/// implementing the open [`SchemeFamily`] trait. The only closed set it draws
/// from is [`EgressShape`] (the irreducible wire-primitive vocabulary), and
/// `NegotiatedSignature` already covers RFC 9421.
struct Rfc9421Signature;

impl SchemeFamily for Rfc9421Signature {
    // RFC 9421 negotiates which message components are covered, then signs them.
    const EGRESS: &'static [EgressShape] = &[EgressShape::NegotiatedSignature];

    fn refresh_classes() -> &'static [RefreshStrategyKind] {
        // A long-lived signing key; nothing to refresh.
        &[RefreshStrategyKind::Static]
    }

    fn pattern() -> AuthPattern {
        // No first-party cosmetic pattern fits a novel protocol — `Custom` is
        // the UI label, decoupled from the (sound, framework-checked) mechanics.
        AuthPattern::Custom
    }
}

/// The plugin's scheme: a public reference to a signing key. The private key
/// lives in an external signer (HSM / KMS), so this struct holds no secret —
/// it is a [`PublicScheme`].
struct HttpSignatureKey {
    key_id: String,
}

impl AuthScheme for HttpSignatureKey {
    type Family = Rfc9421Signature;

    fn pattern() -> AuthPattern {
        AuthPattern::Custom
    }
}

impl PublicScheme for HttpSignatureKey {}

fn main() {
    // The framework reads the plugin's mechanics through the trait — no `match`
    // over a closed family enum, so no framework edit was needed to support it.
    let egress = <HttpSignatureKey as AuthScheme>::Family::EGRESS;
    let refresh = <HttpSignatureKey as AuthScheme>::Family::refresh_classes();
    let pattern = HttpSignatureKey::pattern();

    let key = HttpSignatureKey {
        key_id: "key-2026".to_owned(),
    };

    println!(
        "plugin scheme for key '{}' is usable with zero framework edits",
        key.key_id
    );
    println!("  egress shapes  : {egress:?}");
    println!("  refresh classes: {refresh:?}");
    println!("  ui pattern     : {pattern:?}");

    // The mechanics the framework would drive (redaction / transport / audit)
    // are exactly what the plugin declared.
    assert_eq!(egress, &[EgressShape::NegotiatedSignature]);
    assert_eq!(refresh, &[RefreshStrategyKind::Static]);
    assert_eq!(pattern, AuthPattern::Custom);
}
