//! Catalog primitive boundaries, canonical wire formats, and diagnostic privacy.

use std::collections::{BTreeSet, HashSet};
use std::error::Error as _;
use std::fmt::Debug;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use chrono::{Datelike, NaiveDate};
use nebula_core::{ActionKey, CredentialKey, PluginKey, ResourceKey};
use nebula_error::{Classify, ErrorCategory};
use nebula_metadata::{
    CatalogCategoryKey, CatalogLink, CatalogLinkRelation, CatalogLinkTarget, CatalogReference,
    CatalogValueError, DocumentationOrigin, MAX_SHARED_METADATA_BYTES, RemovalDate,
    RemovalMilestone, RemovalSchedule,
};
use proptest::prelude::*;
use semver::{Comparator, Op, Prerelease, Version, VersionReq};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use tracing_subscriber::fmt::{MakeWriter, format::FmtSpan};
use url::Url;

const CANARY: &str = "CANARY_PRIVATE_CATALOG_PAYLOAD";
const REQUIREMENT_CANARY: &str = "PRIVATE-CATALOG-REQUIREMENT-CANARY";

fn assert_round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let wire = serde_json::to_string(value).expect("catalog value serializes");
    assert_eq!(
        &serde_json::from_str::<T>(&wire).expect("borrowed JSON decodes"),
        value
    );
    assert_eq!(
        &serde_json::from_reader::<_, T>(wire.as_bytes()).expect("reader JSON decodes"),
        value
    );
    let owned = serde_json::to_value(value).expect("owned JSON encodes");
    assert_eq!(
        &serde_json::from_value::<T>(owned).expect("owned JSON decodes"),
        value
    );
}

fn assert_rejected<T: DeserializeOwned + Debug>(wire: &str) {
    for error in [
        serde_json::from_str::<T>(wire).expect_err("invalid borrowed JSON is rejected"),
        serde_json::from_reader::<_, T>(wire.as_bytes())
            .expect_err("invalid reader JSON is rejected"),
    ] {
        assert!(
            !error.to_string().contains(CANARY),
            "display exposed a supplied payload"
        );
        assert!(
            !format!("{error:?}").contains(CANARY),
            "debug exposed a supplied payload"
        );
        assert!(error.source().is_none(), "parser sources are not retained");
    }
}

fn assert_value_rejected<T: DeserializeOwned + Debug>(value: serde_json::Value) {
    assert_rejected::<T>(&serde_json::to_string(&value).expect("test JSON encodes"));
    let error = serde_json::from_value::<T>(value).expect_err("invalid owned JSON is rejected");
    assert!(!format!("{error}: {error:?}").contains(CANARY));
    assert!(error.source().is_none());
}

#[test]
fn category_keys_preserve_checked_text_and_lexical_order() {
    let texts = [
        "a",
        "0",
        "database",
        "database.relational",
        "data-tools.sql_v2",
    ];
    for text in texts {
        let category = CatalogCategoryKey::try_from(text).expect("structured lowercase category");
        assert_eq!(category.as_str(), text);
        assert_eq!(category.to_string(), text);
        assert_eq!(
            CatalogCategoryKey::try_from(text.to_owned()).unwrap(),
            category
        );
        assert_eq!(text.parse::<CatalogCategoryKey>().unwrap(), category);
        assert_eq!(serde_json::to_value(&category).unwrap(), json!(text));
        assert_round_trip(&category);
    }

    let keys: Vec<CatalogCategoryKey> = ["z", "a.b", "a", "a.b", "a-a", "a_a"]
        .into_iter()
        .map(|text| text.parse().unwrap())
        .collect();
    let sorted: BTreeSet<_> = keys.iter().cloned().collect();
    assert_eq!(
        sorted
            .iter()
            .map(CatalogCategoryKey::as_str)
            .collect::<Vec<_>>(),
        ["a", "a-a", "a.b", "a_a", "z"]
    );
    assert_eq!(keys.into_iter().collect::<HashSet<_>>().len(), 5);
}

#[test]
fn categories_reject_noncanonical_segments_without_normalizing() {
    for text in [
        "Database",
        "DATABASE",
        " database",
        "database ",
        "data base",
        ".database",
        "database.",
        "_database",
        "-database",
        "data_",
        "data-",
        "data..base",
        "data__base",
        "data--base",
        "data._base",
        "data-.base",
        "data_-base",
        "data.-base",
        "data/base",
        "data:base",
        "d\u{00e1}ta",
        "data\nbase",
        "data\0base",
        "data\u{200b}base",
    ] {
        assert_eq!(
            CatalogCategoryKey::try_from(text),
            Err(CatalogValueError::InvalidCategoryKey),
            "{text:?}"
        );
        assert_value_rejected::<CatalogCategoryKey>(json!(text));
    }
    assert_eq!(
        CatalogCategoryKey::try_from(""),
        Err(CatalogValueError::EmptyCategoryKey)
    );
    assert_value_rejected::<CatalogCategoryKey>(json!(""));
}

#[test]
fn category_byte_limit_is_inclusive_and_applies_during_serde() {
    let boundary = "a".repeat(96);
    let category = CatalogCategoryKey::try_from(boundary.clone()).unwrap();
    assert_eq!(category.as_str(), boundary);
    assert_round_trip(&category);
    let oversized = "a".repeat(97);
    assert_eq!(
        CatalogCategoryKey::try_from(oversized.clone()),
        Err(CatalogValueError::CategoryKeyTooLong)
    );
    assert_value_rejected::<CatalogCategoryKey>(json!(oversized));
}

#[test]
fn link_relations_have_closed_snake_case_names_and_declared_order() {
    let relations = [
        (CatalogLinkRelation::Overview, "overview"),
        (CatalogLinkRelation::Setup, "setup"),
        (CatalogLinkRelation::Reference, "reference"),
        (CatalogLinkRelation::Migration, "migration"),
        (CatalogLinkRelation::Troubleshooting, "troubleshooting"),
    ];
    for (relation, text) in relations {
        assert_eq!(serde_json::to_value(relation).unwrap(), json!(text));
        assert_eq!(text.parse::<CatalogLinkRelation>().unwrap(), relation);
        assert_round_trip(&relation);
        assert_value_rejected::<CatalogLinkRelation>(json!({(text):null}));
    }
    let sorted: BTreeSet<_> = relations
        .into_iter()
        .rev()
        .map(|(relation, _)| relation)
        .collect();
    assert_eq!(
        sorted.into_iter().collect::<Vec<_>>(),
        relations.map(|(relation, _)| relation)
    );
    for unknown in ["Overview", "overview ", "unknown", CANARY] {
        assert_eq!(
            unknown.parse::<CatalogLinkRelation>(),
            Err(CatalogValueError::InvalidLinkRelation)
        );
        assert_value_rejected::<CatalogLinkRelation>(json!(unknown));
    }
}

#[test]
fn links_serialize_as_strict_relation_target_objects() {
    let target: CatalogLinkTarget = "/docs/setup".parse().unwrap();
    let link = CatalogLink::new(CatalogLinkRelation::Setup, target.clone());
    assert_eq!(link.relation(), CatalogLinkRelation::Setup);
    assert_eq!(link.target(), &target);
    assert_eq!(
        serde_json::to_value(&link).unwrap(),
        json!({"relation":"setup","target":"/docs/setup"})
    );
    assert_round_trip(&link);

    for value in [
        json!({"relation":"setup"}),
        json!({"target":"/docs/setup"}),
        json!({"relation":CANARY,"target":"/docs/setup"}),
        json!({"relation":"setup","target":"/docs/setup",(CANARY):true}),
        json!({"relation":"setup","target":{(CANARY):true}}),
        json!(["setup", "/docs/setup"]),
    ] {
        assert_value_rejected::<CatalogLink>(value);
    }
    assert_rejected::<CatalogLink>(
        r#"{"relation":"setup","relation":"overview","target":"/docs"}"#,
    );
    assert_rejected::<CatalogLink>(r#"{"relation":"setup","target":"/a","target":"/b"}"#);
}

#[test]
fn url_parser_canonicalizes_absolute_and_relative_targets() {
    for (input, expected) in [
        (
            "HTTPS://Docs.Example:443/a/../guide",
            "https://docs.example/guide",
        ),
        ("https://docs.example", "https://docs.example/"),
        (
            "https://docs.example:8443/guide?mode=full#install",
            "https://docs.example:8443/guide?mode=full#install",
        ),
        (
            "https://[2001:db8::1]:443/guide",
            "https://[2001:db8::1]/guide",
        ),
        (
            "https://m\u{00fc}nich.example/caf\u{00e9}",
            "https://xn--mnich-kva.example/caf%C3%A9",
        ),
        (
            "/docs/old/../setup?lang=en#install",
            "/docs/setup?lang=en#install",
        ),
        ("/docs/%2e%2e/setup", "/setup"),
        ("/docs/hello world", "/docs/hello%20world"),
        ("/\u{00e9}", "/%C3%A9"),
        ("/", "/"),
        ("/?q=setup#example", "/?q=setup#example"),
    ] {
        let target: CatalogLinkTarget = input.parse().unwrap();
        assert_eq!(target.as_str(), expected, "{input:?}");
        assert_eq!(target.to_string(), expected);
        assert_eq!(
            CatalogLinkTarget::try_from(input.to_owned()).unwrap(),
            target
        );
        assert_eq!(serde_json::to_value(&target).unwrap(), json!(expected));
        assert_eq!(
            target.as_str().parse::<CatalogLinkTarget>().unwrap(),
            target
        );
        assert_round_trip(&target);
    }
}

#[test]
fn root_relative_targets_resolve_only_against_the_explicit_origin() {
    for origin_text in [
        "https://docs.example",
        "https://docs.example:8443",
        "https://[2001:db8::1]:8443",
    ] {
        let origin: DocumentationOrigin = origin_text.parse().unwrap();
        let expected_origin = Url::parse(origin_text).unwrap().origin();
        for path in [
            "/guide",
            "/../guide",
            "/docs/%2e%2e/guide",
            "/https://external.example",
            "/%2f%2fexternal.example",
        ] {
            let target: CatalogLinkTarget = path.parse().unwrap();
            assert!(target.is_root_relative());
            assert!(target.as_str().starts_with('/'));
            let resolved = target.resolve(&origin).unwrap();
            assert!(!resolved.is_root_relative());
            assert_eq!(
                Url::parse(resolved.as_str()).unwrap().origin(),
                expected_origin
            );
            assert_eq!(
                resolved.as_str(),
                Url::parse(origin_text)
                    .unwrap()
                    .join(target.as_str())
                    .unwrap()
                    .as_str()
            );
            let decoded: CatalogLinkTarget =
                serde_json::from_value(serde_json::to_value(&target).unwrap()).unwrap();
            assert_eq!(decoded.resolve(&origin).unwrap(), resolved);
        }
    }
    let unresolved: CatalogLinkTarget = "/guide".parse().unwrap();
    assert_eq!(unresolved.as_str(), "/guide");
    assert_eq!(serde_json::to_value(&unresolved).unwrap(), json!("/guide"));

    let origin: DocumentationOrigin = "https://docs.example".parse().unwrap();
    let external: CatalogLinkTarget = "https://external.example:8443/guide".parse().unwrap();
    assert!(!external.is_root_relative());
    assert_eq!(
        external.resolve(&origin).unwrap().as_str(),
        "https://external.example:8443/guide"
    );
    assert_eq!(external.resolve(&origin).unwrap(), external);
}

#[test]
fn resolved_targets_preserve_the_absolute_target_byte_limit() {
    let origin: DocumentationOrigin = "https://docs.example".parse().unwrap();
    let authority_bytes = "https://docs.example".len();
    let boundary: CatalogLinkTarget = format!("/{}", "a".repeat(2048 - authority_bytes - 1))
        .parse()
        .unwrap();
    let resolved = boundary.resolve(&origin).unwrap();
    assert!(!resolved.is_root_relative());
    assert_eq!(resolved.as_str().len(), 2048);
    assert_round_trip(&resolved);
    let oversized: CatalogLinkTarget = format!("{}a", boundary.as_str()).parse().unwrap();
    assert_eq!(
        oversized.resolve(&origin),
        Err(CatalogValueError::LinkTargetTooLong)
    );
}

#[test]
fn link_targets_reject_authority_changing_and_executable_spellings() {
    for input in [
        "",
        "guide",
        "./guide",
        "../guide",
        "?guide",
        "#guide",
        "//evil.example/guide",
        "///evil.example",
        "/\\evil.example",
        "\\evil.example",
        "https:\\evil.example",
        "https://docs.example\\@evil.example",
        "/.//evil.example",
        "/a/..//evil.example",
        "/%2e//evil.example",
        "/%2e%2e//evil.example",
        "https:evil.example",
        "https:/evil.example",
        "https:///evil.example",
        "https://",
        "https://?q=evil",
        "http://docs.example",
        "ftp://docs.example",
        "file:///etc/passwd",
        "data:text/html,hello",
        "javascript:alert(1)",
        "mailto:user@example.com",
        "https://user:password@docs.example",
        "https://user@docs.example",
        "https://@docs.example",
        "https://:@docs.example",
        "https://docs.example:65536",
        "https://[not-an-ipv6-address]/",
        " https://docs.example",
        "https://docs.example ",
        "/docs/guide ",
        "/docs/\u{007f}guide",
        "/docs/\u{0085}guide",
        "/\n/evil.example",
        "/\t/evil.example",
        "https://docs.example/\0guide",
    ] {
        assert_eq!(
            input.parse::<CatalogLinkTarget>(),
            Err(CatalogValueError::InvalidLinkTarget),
            "{input:?}"
        );
        assert_value_rejected::<CatalogLinkTarget>(json!(input));
    }
}

#[test]
fn every_ascii_control_and_backslash_is_rejected_before_url_normalization() {
    for character in (0..=0x7f)
        .filter_map(char::from_u32)
        .filter(|character| character.is_control() || *character == '\\')
    {
        for input in [
            format!("/docs/{character}guide"),
            format!("https://docs.example/{character}guide"),
        ] {
            assert_eq!(
                input.parse::<CatalogLinkTarget>(),
                Err(CatalogValueError::InvalidLinkTarget)
            );
        }
        let origin = format!("https://docs{character}.example");
        assert_eq!(
            origin.parse::<DocumentationOrigin>(),
            Err(CatalogValueError::InvalidDocumentationOrigin)
        );
    }
}

#[test]
fn link_limits_cover_input_and_percent_encoded_canonical_bytes() {
    for prefix in ["/", "https://docs.example/"] {
        let boundary = format!("{prefix}{}", "a".repeat(2048 - prefix.len()));
        let target: CatalogLinkTarget = boundary.parse().unwrap();
        assert_eq!(target.as_str(), boundary);
        assert_round_trip(&target);
        let oversized = format!("{boundary}a");
        assert_eq!(
            oversized.parse::<CatalogLinkTarget>(),
            Err(CatalogValueError::LinkTargetTooLong)
        );
        assert_value_rejected::<CatalogLinkTarget>(json!(oversized));
    }
    let expanded_boundary = format!("/{}a", "\u{00e9}".repeat(341));
    let target: CatalogLinkTarget = expanded_boundary.parse().unwrap();
    assert_eq!(target.as_str(), format!("/{}a", "%C3%A9".repeat(341)));
    assert_eq!(target.as_str().len(), 2048);
    assert_round_trip(&target);
    let oversized_after_encoding = format!("{expanded_boundary}a");
    assert!(oversized_after_encoding.len() < 2048);
    assert_eq!(
        oversized_after_encoding.parse::<CatalogLinkTarget>(),
        Err(CatalogValueError::LinkTargetTooLong)
    );
    assert_value_rejected::<CatalogLinkTarget>(json!(oversized_after_encoding));
}

#[test]
fn canonical_link_equality_deduplicates_relation_target_pairs() {
    let links = [
        CatalogLink::new(
            CatalogLinkRelation::Overview,
            "HTTPS://DOCS.EXAMPLE:443/a/../".parse().unwrap(),
        ),
        CatalogLink::new(
            CatalogLinkRelation::Overview,
            "https://docs.example/".parse().unwrap(),
        ),
        CatalogLink::new(
            CatalogLinkRelation::Setup,
            "https://docs.example/".parse().unwrap(),
        ),
    ];
    assert_eq!(links[0], links[1]);
    assert_ne!(links[0], links[2]);
    assert_eq!(links.iter().cloned().collect::<HashSet<_>>().len(), 2);
    assert_eq!(links.into_iter().collect::<BTreeSet<_>>().len(), 2);
}

#[test]
fn documentation_origins_canonicalize_https_authorities() {
    for (input, expected) in [
        ("https://docs.example", "https://docs.example/"),
        ("HTTPS://Docs.Example:443/", "https://docs.example/"),
        ("https://docs.example:8443/", "https://docs.example:8443/"),
        ("https://[2001:db8::1]:8443", "https://[2001:db8::1]:8443/"),
    ] {
        let origin: DocumentationOrigin = input.parse().unwrap();
        assert_eq!(origin.as_str(), expected);
        assert_eq!(origin.to_string(), expected);
        assert_eq!(
            DocumentationOrigin::try_from(input.to_owned()).unwrap(),
            origin
        );
        assert_eq!(serde_json::to_value(&origin).unwrap(), json!(expected));
        assert_round_trip(&origin);
    }
}

#[test]
fn documentation_origins_reject_non_origin_components_even_if_normalized_away() {
    for input in [
        "",
        "/",
        "//docs.example",
        "http://docs.example",
        "https:docs.example",
        "https:///docs.example",
        "https://user:password@docs.example",
        "https://@docs.example",
        "https://:@docs.example",
        "https://docs.example/docs",
        "https://docs.example//",
        "https://docs.example/./",
        "https://docs.example/docs/..",
        "https://docs.example/%2e/",
        "https://docs.example?",
        "https://docs.example/?",
        "https://docs.example#",
        "https://docs.example/#",
        "https://docs.example?x=1",
        "https://docs.example#guide",
        " https://docs.example",
        "https://docs.example ",
        "https://docs.example/\n",
        "https://docs.example\\",
    ] {
        assert_eq!(
            input.parse::<DocumentationOrigin>(),
            Err(CatalogValueError::InvalidDocumentationOrigin),
            "{input:?}"
        );
        assert_value_rejected::<DocumentationOrigin>(json!(input));
    }
    assert_eq!(
        format!("https://{}.example", "a".repeat(2048)).parse::<DocumentationOrigin>(),
        Err(CatalogValueError::InvalidDocumentationOrigin)
    );
}

#[test]
fn all_reference_families_preserve_typed_keys_and_optional_version_requirements() {
    let references = [
        (
            CatalogReference::action(ActionKey::new("postgres.connect").unwrap()),
            "action",
            "postgres.connect",
        ),
        (
            CatalogReference::credential(CredentialKey::new("postgres.auth").unwrap()),
            "credential",
            "postgres.auth",
        ),
        (
            CatalogReference::resource(ResourceKey::new("postgres.client").unwrap()),
            "resource",
            "postgres.client",
        ),
        (
            CatalogReference::plugin(PluginKey::new("postgres").unwrap()),
            "plugin",
            "postgres",
        ),
    ];
    for (reference, kind, key) in references {
        assert_eq!(reference.validate(), Ok(()));
        assert_eq!(reference.key(), key);
        assert_eq!(reference.version_requirement(), None);
        assert_eq!(
            serde_json::to_value(&reference).unwrap(),
            json!({"kind":kind,"key":key})
        );
        assert_round_trip(&reference);
        let requirement: VersionReq = ">=2.0.0, <3.0.0".parse().unwrap();
        let constrained = reference.with_version_requirement(requirement.clone());
        assert_eq!(constrained.validate(), Ok(()));
        assert_eq!(constrained.key(), key);
        assert_eq!(constrained.version_requirement(), Some(&requirement));
        assert_eq!(
            serde_json::to_value(&constrained).unwrap(),
            json!({"kind":kind,"key":key,"version_requirement":">=2.0.0, <3.0.0"})
        );
        assert_round_trip(&constrained);
        let replacement: VersionReq = "^3".parse().unwrap();
        let replaced = constrained.with_version_requirement(replacement.clone());
        assert_eq!(replaced.version_requirement(), Some(&replacement));
    }
}

fn references_with_native_requirement(requirement: &VersionReq) -> [CatalogReference; 8] {
    [
        CatalogReference::action("postgres.connect".parse().unwrap())
            .with_version_requirement(requirement.clone()),
        CatalogReference::credential("postgres.auth".parse().unwrap())
            .with_version_requirement(requirement.clone()),
        CatalogReference::resource("postgres.client".parse().unwrap())
            .with_version_requirement(requirement.clone()),
        CatalogReference::plugin("postgres".parse().unwrap())
            .with_version_requirement(requirement.clone()),
        CatalogReference::Action {
            key: "postgres.connect".parse().unwrap(),
            version_requirement: Some(requirement.clone()),
        },
        CatalogReference::Credential {
            key: "postgres.auth".parse().unwrap(),
            version_requirement: Some(requirement.clone()),
        },
        CatalogReference::Resource {
            key: "postgres.client".parse().unwrap(),
            version_requirement: Some(requirement.clone()),
        },
        CatalogReference::Plugin {
            key: "postgres".parse().unwrap(),
            version_requirement: Some(requirement.clone()),
        },
    ]
}

fn assert_reference_serialization_rejected(reference: &CatalogReference) {
    let validation = reference.validate();
    assert_eq!(
        validation,
        Err(CatalogValueError::InvalidReferenceVersionRequirement)
    );
    assert!(!format!("{validation:?}").contains(REQUIREMENT_CANARY));
    let mut output = Vec::new();
    let error = serde_json::to_writer(&mut output, reference)
        .expect_err("unrepresentable native requirement must fail before writing a reference");
    assert!(
        output.is_empty(),
        "invalid intent must not write a partial reference"
    );
    assert!(
        error
            .to_string()
            .contains(&CatalogValueError::InvalidReferenceVersionRequirement.to_string())
    );
    assert!(!format!("{error}: {error:?}").contains(REQUIREMENT_CANARY));
    assert!(error.source().is_none());
    let error = serde_json::to_value(reference)
        .expect_err("owned JSON must reject unrepresentable native requirements");
    assert!(!format!("{error}: {error:?}").contains(REQUIREMENT_CANARY));
    assert!(error.source().is_none());
}

#[test]
fn native_requirements_with_33_comparators_are_rejected_before_serialization() {
    let requirement = VersionReq {
        comparators: vec!["^2".parse::<Comparator>().unwrap(); 33],
    };
    assert!(requirement.to_string().parse::<VersionReq>().is_err());
    for reference in references_with_native_requirement(&requirement) {
        assert_reference_serialization_rejected(&reference);
    }
}

#[test]
fn native_comparator_fields_must_survive_serialization_exactly() {
    let comparisons = [
        Comparator {
            op: Op::Exact,
            major: 2,
            minor: None,
            patch: Some(1),
            pre: Prerelease::EMPTY,
        },
        Comparator {
            op: Op::Exact,
            major: 2,
            minor: Some(0),
            patch: None,
            pre: REQUIREMENT_CANARY.parse().unwrap(),
        },
        Comparator {
            op: Op::Wildcard,
            major: 2,
            minor: Some(0),
            patch: Some(1),
            pre: Prerelease::EMPTY,
        },
        Comparator {
            op: Op::Wildcard,
            major: 2,
            minor: None,
            patch: None,
            pre: REQUIREMENT_CANARY.parse().unwrap(),
        },
    ];
    for comparator in comparisons {
        let requirement = VersionReq {
            comparators: vec![comparator],
        };
        let reparsed: VersionReq = requirement.to_string().parse().unwrap();
        assert_ne!(
            requirement, reparsed,
            "the fixture must lose typed evidence in native Display"
        );
        for reference in references_with_native_requirement(&requirement) {
            assert_reference_serialization_rejected(&reference);
        }
    }
}

#[test]
fn native_requirements_at_32_comparators_round_trip_in_every_reference_family() {
    let requirement = VersionReq {
        comparators: vec!["^2.0.0-rc.1".parse::<Comparator>().unwrap(); 32],
    };
    assert_eq!(
        requirement.to_string().parse::<VersionReq>().unwrap(),
        requirement
    );
    for reference in references_with_native_requirement(&requirement) {
        assert_eq!(reference.validate(), Ok(()));
        assert_round_trip(&reference);
    }
}

#[test]
fn native_requirement_formatting_respects_the_shared_byte_ceiling() {
    let prefix = "^2.0.0-";
    let boundary = format!(
        "{prefix}{}",
        "a".repeat(MAX_SHARED_METADATA_BYTES - prefix.len())
    );
    let requirement: VersionReq = boundary.parse().unwrap();
    for reference in references_with_native_requirement(&requirement) {
        assert_eq!(reference.validate(), Ok(()));
        assert_round_trip(&reference);
    }
    let oversized: VersionReq = format!("{boundary}a").parse().unwrap();
    for reference in references_with_native_requirement(&oversized) {
        assert_reference_serialization_rejected(&reference);
    }
    for kind in ["action", "credential", "resource", "plugin"] {
        assert_value_rejected::<CatalogReference>(json!({
            "kind": kind, "key": "postgres", "version_requirement": format!("{boundary}a")
        }));
    }
}

#[test]
fn native_representable_operators_and_wildcards_preserve_exact_requirements() {
    for text in [
        "*",
        "=2",
        ">2.1",
        ">=2.1.3",
        "<3",
        "<=2.9.9",
        "~2.1",
        "^2.0.0-rc.1",
        "2.*",
        "2.1.*",
    ] {
        let requirement: VersionReq = text.parse().unwrap();
        for reference in references_with_native_requirement(&requirement) {
            assert_eq!(reference.validate(), Ok(()));
            assert_round_trip(&reference);
        }
    }
}

#[test]
fn native_requirement_validation_and_serialization_traces_omit_comparator_payloads() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_writer(capture.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let valid: VersionReq = format!("^2.0.0-{REQUIREMENT_CANARY}").parse().unwrap();
        let mut invalid = valid.clone();
        invalid.comparators[0].patch = None;
        for reference in references_with_native_requirement(&invalid) {
            assert_reference_serialization_rejected(&reference);
        }
        for reference in references_with_native_requirement(&valid) {
            assert_eq!(reference.validate(), Ok(()));
            assert_round_trip(&reference);
        }
    });
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("metadata.validate_catalog_reference"));
    assert!(output.contains("metadata.serialize_catalog_reference"));
    assert!(output.contains("metadata.check_version_requirement"));
    assert!(!output.contains(REQUIREMENT_CANARY));
}

#[test]
fn reference_serde_reparses_keys_and_rejects_unknown_structure() {
    for (kind, key_error) in [
        ("action", CatalogValueError::InvalidActionReferenceKey),
        (
            "credential",
            CatalogValueError::InvalidCredentialReferenceKey,
        ),
        ("resource", CatalogValueError::InvalidResourceReferenceKey),
        ("plugin", CatalogValueError::InvalidPluginReferenceKey),
    ] {
        for key in [
            String::new(),
            "bad!key".to_owned(),
            "bad__key".to_owned(),
            "a".repeat(1024),
            format!("{CANARY}!"),
        ] {
            let wire = json!({"kind":kind,"key":key});
            let error = serde_json::from_value::<CatalogReference>(wire.clone()).unwrap_err();
            assert!(error.to_string().contains(&key_error.to_string()));
            assert_value_rejected::<CatalogReference>(wire);
        }
        for value in [
            json!({"kind":{(kind):null},"key":"postgres"}),
            json!({"kind":kind,"key":"postgres","version_requirement":CANARY}),
            json!({"kind":kind,"key":"postgres","version_requirement":""}),
            json!({"kind":kind,"key":"postgres","version_requirement":{(CANARY):true}}),
            json!({"kind":kind,"key":"postgres",(CANARY):true}),
            json!({"kind":kind,"key":{(CANARY):true}}),
            json!({"kind":kind}),
        ] {
            assert_value_rejected::<CatalogReference>(value);
        }
    }
    assert_value_rejected::<CatalogReference>(json!({"kind":CANARY,"key":"postgres"}));
    assert_value_rejected::<CatalogReference>(json!({"key":"postgres"}));
    assert_value_rejected::<CatalogReference>(json!(["action", "postgres", null]));
    assert_rejected::<CatalogReference>(r#"{"kind":"action","kind":"plugin","key":"postgres"}"#);
    assert_rejected::<CatalogReference>(r#"{"kind":"action","key":"postgres","key":"other"}"#);
    assert_rejected::<CatalogReference>(
        r#"{"kind":"action","key":"postgres","version_requirement":"^1","version_requirement":"^2"}"#,
    );
    let escaped =
        r#"{"kind":"resource","key":"postgr\u0065s.client","version_requirement":"\u005e2"}"#;
    let reference: CatalogReference = serde_json::from_reader(escaped.as_bytes()).unwrap();
    assert_eq!(
        reference,
        CatalogReference::resource(ResourceKey::new("postgres.client").unwrap())
            .with_version_requirement("^2".parse().unwrap())
    );
}

#[test]
fn removal_dates_require_canonical_calendar_text_and_real_leap_days() {
    for text in [
        "0001-01-01",
        "1900-02-28",
        "2000-02-29",
        "2028-02-29",
        "2100-03-01",
        "9999-12-31",
    ] {
        let date: RemovalDate = text.parse().unwrap();
        let expected = NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(date.to_string(), text);
        assert_eq!(
            (date.year(), date.month(), date.day()),
            (expected.year(), expected.month(), expected.day())
        );
        assert_eq!(
            RemovalDate::new(expected.year(), expected.month(), expected.day()).unwrap(),
            date
        );
        assert_eq!(RemovalDate::try_from(text.to_owned()).unwrap(), date);
        assert_eq!(serde_json::to_value(date).unwrap(), json!(text));
        assert_round_trip(&date);
    }
    for text in [
        "",
        "0000-01-01",
        "10000-01-01",
        "-0001-01-01",
        "+2028-02-29",
        "2028-2-29",
        "2028-02-9",
        "2028-02-029",
        "2028/02/29",
        "2028-00-01",
        "2028-13-01",
        "2028-01-00",
        "2028-04-31",
        "1900-02-29",
        "2027-02-29",
        "2100-02-29",
        "2028-02-30",
        "2028-02-29T00:00:00Z",
        "2028-02-29 ",
        " 2028-02-29",
        "2028-02-29\n",
        "\u{ff12}028-02-29",
        CANARY,
    ] {
        assert_eq!(
            text.parse::<RemovalDate>(),
            Err(CatalogValueError::InvalidRemovalDate),
            "{text:?}"
        );
        assert_value_rejected::<RemovalDate>(json!(text));
    }
}

#[test]
fn removal_date_components_reject_invalid_calendar_values_and_numeric_extremes() {
    for (year, month, day) in [
        (i32::MIN, 1, 1),
        (-1, 1, 1),
        (0, 1, 1),
        (10000, 1, 1),
        (i32::MAX, 1, 1),
        (2028, 0, 1),
        (2028, 13, 1),
        (2028, u32::MAX, 1),
        (2028, 1, 0),
        (2028, 1, 32),
        (2028, 1, u32::MAX),
        (2028, 4, 31),
        (1900, 2, 29),
        (2027, 2, 29),
        (2100, 2, 29),
        (2028, 2, 30),
    ] {
        assert_eq!(
            RemovalDate::new(year, month, day),
            Err(CatalogValueError::InvalidRemovalDate),
            "{year}, {month}, {day}"
        );
    }
}

#[test]
fn milestones_trim_unicode_whitespace_and_bound_utf8_bytes() {
    for (input, expected) in [
        ("  Next release  ", "Next release"),
        (
            "\u{3000}General availability\u{00a0}",
            "General availability",
        ),
        ("a  b", "a  b"),
        ("\u{00e9}tape finale", "\u{00e9}tape finale"),
    ] {
        let milestone: RemovalMilestone = input.parse().unwrap();
        assert_eq!(milestone.as_str(), expected);
        assert_eq!(milestone.to_string(), expected);
        assert_eq!(
            RemovalMilestone::try_from(input.to_owned()).unwrap(),
            milestone
        );
        assert_eq!(serde_json::to_value(&milestone).unwrap(), json!(expected));
        assert_round_trip(&milestone);
    }
    for blank in ["", " ", "\t\r\n", "\u{3000}\u{00a0}\u{2003}"] {
        assert_eq!(
            blank.parse::<RemovalMilestone>(),
            Err(CatalogValueError::BlankRemovalMilestone)
        );
        assert_value_rejected::<RemovalMilestone>(json!(blank));
    }
    for boundary in ["a".repeat(256), "\u{00e9}".repeat(128)] {
        let milestone: RemovalMilestone = boundary.parse().unwrap();
        assert_eq!(milestone.as_str(), boundary);
        assert_round_trip(&milestone);
        let oversized = format!("{boundary}a");
        assert_eq!(
            oversized.parse::<RemovalMilestone>(),
            Err(CatalogValueError::RemovalMilestoneTooLong)
        );
        assert_value_rejected::<RemovalMilestone>(json!(oversized));
    }
}

#[test]
fn removal_schedules_preserve_the_closed_kind_and_canonical_value() {
    for (schedule, expected) in [
        (
            RemovalSchedule::OnDate("0001-01-01".parse().unwrap()),
            json!({"kind":"on_date","value":"0001-01-01"}),
        ),
        (
            RemovalSchedule::AtVersion("3.0.0-rc.1+build.2".parse().unwrap()),
            json!({"kind":"at_version","value":"3.0.0-rc.1+build.2"}),
        ),
        (
            RemovalSchedule::Milestone("  General availability  ".parse().unwrap()),
            json!({"kind":"milestone","value":"General availability"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&schedule).unwrap(), expected);
        assert_round_trip(&schedule);
    }
    // Syntax admission has no dependency on today's date or source chronology.
    let past: RemovalSchedule =
        serde_json::from_value(json!({"kind":"on_date","value":"1900-01-01"})).unwrap();
    assert_eq!(past, RemovalSchedule::OnDate("1900-01-01".parse().unwrap()));
    let zero: RemovalSchedule =
        serde_json::from_value(json!({"kind":"at_version","value":"0.0.0"})).unwrap();
    assert_eq!(zero, RemovalSchedule::AtVersion(Version::new(0, 0, 0)));
}

#[test]
fn removal_schedule_ingress_rejects_invalid_values_and_extra_evidence() {
    for value in [
        json!({"kind":{"on_date":null},"value":"2028-02-29"}),
        json!({"kind":{"at_version":null},"value":"3.0.0"}),
        json!({"kind":{"milestone":null},"value":"next release"}),
        json!({"kind":"on_date","value":"1900-02-29"}),
        json!({"kind":"on_date","value":"2028-2-29"}),
        json!({"kind":"at_version","value":"^3"}),
        json!({"kind":"at_version","value":"3.0"}),
        json!({"kind":"at_version","value":CANARY}),
        json!({"kind":"milestone","value":"  "}),
        json!({"kind":"milestone","value":"a".repeat(257)}),
        json!({"kind":CANARY,"value":"2028-02-29"}),
        json!({"kind":"on_date","value":"2028-02-29",(CANARY):true}),
        json!({"kind":"on_date","value":{(CANARY):true}}),
        json!({"kind":"on_date"}),
        json!({"value":"2028-02-29"}),
        json!(["on_date", "2028-02-29"]),
    ] {
        assert_value_rejected::<RemovalSchedule>(value);
    }
    assert_rejected::<RemovalSchedule>(
        r#"{"kind":"on_date","kind":"milestone","value":"2028-02-29"}"#,
    );
    assert_rejected::<RemovalSchedule>(
        r#"{"kind":"on_date","value":"2028-02-29","value":"2028-03-01"}"#,
    );
}

#[test]
fn primitive_wrong_types_never_echo_untrusted_payloads() {
    for value in [
        json!(null),
        json!(true),
        json!(123),
        json!([CANARY]),
        json!({(CANARY):CANARY}),
    ] {
        assert_value_rejected::<CatalogCategoryKey>(value.clone());
        assert_value_rejected::<CatalogLinkRelation>(value.clone());
        assert_value_rejected::<CatalogLinkTarget>(value.clone());
        assert_value_rejected::<DocumentationOrigin>(value.clone());
        assert_value_rejected::<RemovalDate>(value.clone());
        assert_value_rejected::<RemovalMilestone>(value.clone());
        assert_value_rejected::<CatalogLink>(value.clone());
        assert_value_rejected::<CatalogReference>(value.clone());
        assert_value_rejected::<RemovalSchedule>(value);
    }
    for wire in [
        format!("\"{CANARY}\""),
        format!("\"{CANARY}"),
        format!("{{\"{CANARY}\":"),
    ] {
        assert_rejected::<CatalogLink>(&wire);
        assert_rejected::<CatalogReference>(&wire);
        assert_rejected::<RemovalSchedule>(&wire);
    }
}

#[test]
fn catalog_value_errors_have_codes_but_no_payloads_or_parser_sources() {
    for error in [
        CatalogValueError::EmptyCategoryKey,
        CatalogValueError::CategoryKeyTooLong,
        CatalogValueError::InvalidCategoryKey,
        CatalogValueError::InvalidLinkRelation,
        CatalogValueError::InvalidLink,
        CatalogValueError::InvalidLinkTarget,
        CatalogValueError::LinkTargetTooLong,
        CatalogValueError::InvalidDocumentationOrigin,
        CatalogValueError::LinkOriginMismatch,
        CatalogValueError::InvalidReference,
        CatalogValueError::InvalidActionReferenceKey,
        CatalogValueError::InvalidCredentialReferenceKey,
        CatalogValueError::InvalidResourceReferenceKey,
        CatalogValueError::InvalidPluginReferenceKey,
        CatalogValueError::InvalidReferenceVersionRequirement,
        CatalogValueError::InvalidRemovalSchedule,
        CatalogValueError::InvalidRemovalDate,
        CatalogValueError::InvalidRemovalVersion,
        CatalogValueError::BlankRemovalMilestone,
        CatalogValueError::RemovalMilestoneTooLong,
        CatalogValueError::InvalidRemovalMilestone,
    ] {
        assert!(error.code().as_str().starts_with("CATALOG:"));
        assert_eq!(error.category(), ErrorCategory::Validation);
        assert!(!error.is_retryable());
        assert!(error.source().is_none());
        assert!(!format!("{error}: {error:?}").contains(CANARY));
    }
}

#[derive(Clone, Default)]
struct TraceCapture(Arc<Mutex<Vec<u8>>>);

impl Write for TraceCapture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("trace capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for TraceCapture {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn parsing_and_resolution_traces_exclude_payloads_on_success_and_failure() {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_writer(capture.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        CANARY
            .parse::<CatalogCategoryKey>()
            .expect_err("uppercase category is rejected");
        let invalid_target = format!("https://{CANARY}:password@docs.example");
        invalid_target
            .parse::<CatalogLinkTarget>()
            .expect_err("credentials are rejected");
        invalid_target
            .parse::<DocumentationOrigin>()
            .expect_err("origin credentials are rejected");
        let valid_target = format!("https://docs.example/{CANARY}?private={CANARY}");
        let target: CatalogLinkTarget = valid_target.parse().unwrap();
        let origin: DocumentationOrigin = "https://docs.example".parse().unwrap();
        assert_eq!(target.resolve(&origin).unwrap().as_str(), valid_target);
        assert_eq!(CANARY.parse::<RemovalMilestone>().unwrap().as_str(), CANARY);
        CANARY
            .parse::<RemovalDate>()
            .expect_err("invalid date is rejected");
        assert_value_rejected::<CatalogReference>(
            json!({"kind":"action","key":format!("{CANARY}!" )}),
        );
        assert_value_rejected::<CatalogReference>(json!({"kind":CANARY,"key":"valid"}));
        assert_value_rejected::<CatalogLink>(json!({"relation":CANARY,"target":"/docs"}));
        assert_value_rejected::<RemovalSchedule>(json!({"kind":"at_version","value":CANARY}));
    });
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("metadata.parse_link_target"));
    assert!(output.contains("metadata.resolve_link_target"));
    assert!(output.contains("metadata.parse_catalog_reference"));
    assert!(output.contains("metadata.parse_removal_schedule"));
    assert!(!output.contains(CANARY));
    assert!(!output.contains("password"));
    assert!(!output.contains("https://docs.example"));
}

proptest! {
    #[test]
    fn category_round_trip_and_lexical_order_match_plain_strings(
        left in "[a-z][a-z0-9]{0,20}(\\.[a-z][a-z0-9]{0,20}){0,2}",
        right in "[a-z][a-z0-9]{0,20}(\\.[a-z][a-z0-9]{0,20}){0,2}",
    ) {
        let left_key: CatalogCategoryKey = left.parse().unwrap();
        let right_key: CatalogCategoryKey = right.parse().unwrap();
        prop_assert_eq!(left_key.cmp(&right_key), left.cmp(&right));
        assert_round_trip(&left_key);
        assert_round_trip(&right_key);
    }

    #[test]
    fn arbitrary_accepted_link_targets_are_canonical_and_resolve_stably(input in prop_oneof![
        ".{0,160}",
        ".{0,160}".prop_map(|path| format!("/{path}")),
        ".{0,160}".prop_map(|path| format!("https://docs.example/{path}")),
    ]) {
        if let Ok(target) = input.parse::<CatalogLinkTarget>() {
            let decoded: CatalogLinkTarget = serde_json::from_value(serde_json::to_value(&target).unwrap()).unwrap();
            prop_assert_eq!(&decoded, &target);
            let reparsed = target.as_str().parse::<CatalogLinkTarget>().unwrap();
            prop_assert_eq!(&reparsed, &target);
            let origin: DocumentationOrigin = "https://docs.example:8443".parse().unwrap();
            let resolved = target.resolve(&origin).unwrap();
            prop_assert!(!resolved.is_root_relative());
            if target.is_root_relative() {
                prop_assert_eq!(Url::parse(resolved.as_str()).unwrap().origin(), Url::parse(origin.as_str()).unwrap().origin());
            } else {
                prop_assert_eq!(resolved.as_str(), target.as_str());
            }
        }
    }

    #[test]
    fn removal_dates_agree_with_chrono_calendar(year in -1..=10000_i32, month in 0..=13_u32, day in 0..=32_u32) {
        let text = format!("{year:04}-{month:02}-{day:02}");
        let constructed = RemovalDate::new(year, month, day);
        if let Some(expected) = NaiveDate::from_ymd_opt(year, month, day)
            .filter(|date| (1..=9999).contains(&date.year()))
        {
            let actual: RemovalDate = text.parse().unwrap();
            prop_assert_eq!(
                (actual.year(), actual.month(), actual.day()),
                (expected.year(), expected.month(), expected.day())
            );
            prop_assert_eq!(constructed, Ok(actual));
            prop_assert_eq!(actual.to_string(), text);
            assert_round_trip(&actual);
        } else {
            prop_assert_eq!(constructed, Err(CatalogValueError::InvalidRemovalDate));
            prop_assert_eq!(text.parse::<RemovalDate>(), Err(CatalogValueError::InvalidRemovalDate));
        }
    }
}
