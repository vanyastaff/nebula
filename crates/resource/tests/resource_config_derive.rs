//! Integration tests for `#[derive(ResourceConfig)]`.
//!
//! These tests exercise the emitted `ResourceConfig::fingerprint` implementation
//! via compilation + runtime assertions. They also cover the `validate` hook and
//! `skip_fingerprint` field attribute.

use nebula_resource::ResourceConfig;

// ── Unit struct ───────────────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig)]
struct UnitCfg;

#[test]
fn unit_struct_fingerprint_is_zero() {
    assert_eq!(UnitCfg.fingerprint(), 0);
}

#[test]
fn unit_struct_identical_instances_equal_fingerprint() {
    assert_eq!(UnitCfg.fingerprint(), UnitCfg.fingerprint());
}

// ── Named-field struct ────────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct NamedCfg {
    host: String,
    port: u16,
}

nebula_schema::impl_empty_has_schema!(NamedCfg);

#[test]
fn named_struct_identical_instances_equal_fingerprint() {
    let a = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "identical configs must produce equal fingerprints"
    );
}

#[test]
fn named_struct_differing_field_produces_different_fingerprint() {
    let a = NamedCfg {
        host: "host-a".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "host-b".into(),
        port: 5432,
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "configs differing in `host` must have different fingerprints"
    );
}

#[test]
fn named_struct_differing_port_produces_different_fingerprint() {
    let a = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "localhost".into(),
        port: 5433,
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "configs differing in `port` must have different fingerprints"
    );
}

#[test]
fn named_struct_fingerprint_nonzero_for_nonempty_fields() {
    let cfg = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    // With non-empty fields the fingerprint is very unlikely to hash to 0;
    // this assertion guards against a regression where the body returns a
    // literal 0 for structs with fields.
    assert_ne!(
        cfg.fingerprint(),
        0,
        "non-empty named config must not return 0"
    );
}

// ── skip_fingerprint field ────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct SkipFieldCfg {
    /// Included in fingerprint.
    endpoint: String,
    /// Excluded from fingerprint — changing this alone must not trigger hot-reload.
    // guard-justified: the field is under test precisely for being skipped by
    // the fingerprint fold; it is set but never read, which is the point.
    #[allow(
        dead_code,
        reason = "exercised via skip_fingerprint, intentionally never read"
    )]
    #[config(skip_fingerprint)]
    debug_label: String,
}

nebula_schema::impl_empty_has_schema!(SkipFieldCfg);

#[test]
fn skip_fingerprint_field_ignored_in_hash() {
    let a = SkipFieldCfg {
        endpoint: "https://api.example.com".into(),
        debug_label: "label-a".into(),
    };
    let b = SkipFieldCfg {
        endpoint: "https://api.example.com".into(),
        debug_label: "label-b".into(),
    };
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "changing only a skip_fingerprint field must not affect the fingerprint"
    );
}

#[test]
fn skip_fingerprint_included_field_still_differs() {
    let a = SkipFieldCfg {
        endpoint: "https://api-a.example.com".into(),
        debug_label: "same".into(),
    };
    let b = SkipFieldCfg {
        endpoint: "https://api-b.example.com".into(),
        debug_label: "same".into(),
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "changing the non-skipped field must still change the fingerprint"
    );
}

// ── validate hook ─────────────────────────────────────────────────────────────

fn validate_url_cfg(cfg: &UrlCfg) -> Result<(), nebula_resource::Error> {
    if cfg.url.is_empty() {
        Err(nebula_resource::Error::permanent("url must not be empty"))
    } else {
        Ok(())
    }
}

#[derive(Clone, ResourceConfig)]
#[config(validate = validate_url_cfg, schema = external)]
struct UrlCfg {
    url: String,
}

nebula_schema::impl_empty_has_schema!(UrlCfg);

#[test]
fn validate_hook_returns_ok_for_valid_config() {
    let cfg = UrlCfg {
        url: "https://example.com".into(),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn validate_hook_returns_err_for_invalid_config() {
    let cfg = UrlCfg { url: String::new() };
    let err = cfg.validate().unwrap_err();
    assert!(
        matches!(err.kind(), nebula_resource::error::ErrorKind::Permanent),
        "empty url should produce a Permanent error"
    );
}

// ── Tuple struct ──────────────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct TupleCfg(String, u32);

nebula_schema::impl_empty_has_schema!(TupleCfg);

#[test]
fn tuple_struct_identical_instances_equal_fingerprint() {
    let a = TupleCfg("a".into(), 1);
    let b = TupleCfg("a".into(), 1);
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn tuple_struct_different_first_field_differs() {
    let a = TupleCfg("a".into(), 1);
    let b = TupleCfg("b".into(), 1);
    assert_ne!(a.fingerprint(), b.fingerprint());
}
