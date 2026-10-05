//! Active feature views derive from the existing union snapshot, not a second baseline.

#[expect(
    dead_code,
    reason = "the profile test uses only export-map portions of the shared signature walker"
)]
mod public_api;

use std::{collections::BTreeSet, path::Path};

use public_api::{Workspace, active_export_lines, exports};

#[test]
fn profile_identity_matches_compiled_sdk_and_canonical_api() {
    // These are actual compiler cfgs, not producer-submitted pass flags. A new
    // declared feature needs an explicit cfg probe before this evidence qualifies.
    let compiled: BTreeSet<String> = [
        ("default", cfg!(feature = "default")),
        ("derive", cfg!(feature = "derive")),
        ("testing", cfg!(feature = "testing")),
        ("http", cfg!(feature = "http")),
        ("resource-http", cfg!(feature = "resource-http")),
    ]
    .into_iter()
    .filter(|(_, enabled)| *enabled)
    .map(|(name, _)| name.to_owned())
    .collect();
    if let Ok(producer_features) = std::env::var("NEBULA_SDK_ACTIVE_FEATURES") {
        let declared: BTreeSet<String> = producer_features
            .split(',')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        assert_eq!(
            declared, compiled,
            "producer profile must match compiler cfg"
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("SDK source is a workspace member");
    let workspace = Workspace::load(root, &[("nebula_sdk", "crates/sdk/src")]);
    let actual = exports(&workspace, "nebula_sdk", "__private")
        .into_iter()
        .map(|export| export.line)
        .collect::<Vec<_>>()
        .join("\n");
    let canonical = include_str!("snapshots/public_api_snapshot__sdk_export_map.snap")
        .split("---")
        .nth(2)
        .expect("existing Insta union snapshot has a body")
        .trim();
    let actual = active_export_lines(&actual, &compiled);
    let expected = active_export_lines(canonical, &compiled);
    assert_eq!(
        actual, expected,
        "active API differs from the canonical union baseline"
    );
    println!(
        "SDK feature profile: {compiled:?}; active exported paths: {}",
        actual.len()
    );
}
