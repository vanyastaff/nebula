//! Public-API snapshots of the SDK façade, for review visibility.
//!
//! This is **not** a SemVer freeze: a changed snapshot is expected whenever
//! the curated surface changes on purpose. It makes such a change visible in
//! review, as a diff of two committed `.snap` files:
//!
//! - `sdk_export_map` — one sorted line per public path of `nebula_sdk`
//!   (`public::path => origin::path [use]`, `[local]` items, `[glob]`s),
//!   with `cfg` gates and `[hidden]` (`#[doc(hidden)]`) recorded textually.
//!   The macro namespace `__private` is owned by the explicit allowlist in
//!   `public_perimeter_external_contract.rs` and skipped here.
//! - `sdk_resource_signatures` — every item re-exported through
//!   `nebula_sdk::integration::resource` (and its `http` adapter, whose items
//!   the SDK defines) and every `nebula_resource` item in the prelude,
//!   resolved through the `pub use` chain to its definition,
//!   rendered as a signature (generics, bounds, derives, `non_exhaustive`,
//!   public fields and variants, trait items) plus the impls in the defining
//!   crate: inherent `pub` methods and trait impl headers. Items and methods
//!   documented as `**Interim surface**` carry `[interim]`.
//!
//! The walker (`public_api/mod.rs`) parses sources with `syn`; it needs no
//! rustdoc or nightly toolchain and prints no file paths or line numbers.
//! Its known limits are listed there. rustdoc JSON (`cargo public-api`) is
//! the upgrade path once the public surface is frozen for a release.
//!
//! Bless an intended change with `task sdk:api:bless`, check with
//! `task sdk:api:check` (CI runs this test in the `nebula-sdk` job; insta
//! fails on a mismatch when `CI` is set).

mod public_api;

use std::collections::BTreeMap;
use std::path::Path;

use public_api::{Target, Workspace, exports, render_target, target_heading};

/// Crates the walker loads, with their source roots relative to the workspace.
const CRATES: &[(&str, &str)] = &[
    ("nebula_sdk", "crates/sdk/src"),
    ("nebula_resource", "crates/resource/src"),
    ("nebula_resource_macros", "crates/resource/macros/src"),
    ("nebula_core", "crates/core/src"),
    // `rate_limit::Rate` is the resilience GCRA rate, re-exported.
    ("nebula_resilience", "crates/resilience/src"),
    // `CredentialGuard`, re-exported for credential slots.
    ("nebula_credential", "crates/credential/src"),
];

fn workspace() -> Workspace {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/sdk sits two levels below the workspace root");
    Workspace::load(root, CRATES)
}

fn export_map(workspace: &Workspace) -> String {
    let mut out = String::new();
    for export in exports(workspace, "nebula_sdk", "__private") {
        out.push_str(&export.line);
        out.push('\n');
    }
    out
}

/// Resolved resource targets with the SDK paths that expose each one.
fn resource_targets(workspace: &Workspace) -> BTreeMap<Target, Vec<String>> {
    let resource_module = ["integration".to_owned(), "resource".to_owned()];
    let http_module = [
        "integration".to_owned(),
        "resource".to_owned(),
        "http".to_owned(),
    ];
    let mut targets: BTreeMap<Target, Vec<String>> = BTreeMap::new();
    for export in exports(workspace, "nebula_sdk", "__private") {
        let Some(origin) = &export.origin else {
            continue;
        };
        // The HTTP adapter's items are the SDK's own, defined locally.
        let in_resource_module = export.module == resource_module || export.module == http_module;
        let resource_in_prelude = export.module == ["prelude".to_owned()]
            && origin
                .first()
                .is_some_and(|krate| krate == "nebula_resource");
        if !(in_resource_module || resource_in_prelude) {
            continue;
        }
        let target = workspace.resolve_path("nebula_sdk", &export.module, origin, 0);
        targets.entry(target).or_default().push(export.public_path);
    }
    targets
}

fn resource_signatures(workspace: &Workspace) -> String {
    let mut sections: Vec<(String, String)> = resource_targets(workspace)
        .into_iter()
        .map(|(target, mut public_paths)| {
            public_paths.sort();
            let heading = target_heading(workspace, &target);
            let mut section = format!("## {heading}\n");
            for public_path in public_paths {
                section.push_str(&format!("exported as {public_path}\n"));
            }
            for line in render_target(workspace, &target) {
                section.push_str(&line);
                section.push('\n');
            }
            (heading, section)
        })
        .collect();
    sections.sort();
    sections
        .into_iter()
        .map(|(_, section)| section)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines of the section whose heading ends with `suffix`.
fn section<'a>(text: &'a str, suffix: &str) -> Vec<&'a str> {
    let mut lines = text.lines();
    let found = lines
        .by_ref()
        .any(|line| line.starts_with("## ") && line.split(' ').nth(2) == Some(suffix));
    assert!(found, "no section for {suffix}");
    lines.take_while(|line| !line.is_empty()).collect()
}

#[test]
fn sdk_export_map() {
    let workspace = workspace();
    insta::assert_snapshot!("sdk_export_map", export_map(&workspace));
}

#[test]
fn sdk_resource_signatures() {
    let workspace = workspace();
    let unresolved: Vec<String> = resource_targets(&workspace)
        .into_iter()
        .filter(|(target, _)| matches!(target, Target::External(_) | Target::Unresolved(_)))
        .map(|(target, paths)| format!("{target:?} <- {paths:?}"))
        .collect();
    assert!(
        unresolved.is_empty(),
        "resource exports the walker cannot resolve to a definition: {unresolved:#?}"
    );
    insta::assert_snapshot!("sdk_resource_signatures", resource_signatures(&workspace));
}

/// The managed call facade reaches the instance only through a granted
/// attempt: neither the lease facade nor a submission derefs.
#[test]
fn lease_and_submission_have_no_deref() {
    let text = resource_signatures(&workspace());
    for (item, method) in [
        ("nebula_resource::call::managed::Lease", "pub fn submit<"),
        (
            "nebula_resource::call::managed::Submission",
            "pub fn cancel(",
        ),
    ] {
        let lines = section(&text, item);
        assert!(
            lines.iter().any(|line| line.contains(method)),
            "{item}'s inherent methods must be rendered: {lines:#?}"
        );
        for line in &lines {
            assert!(
                !(line.starts_with("impl") && line.contains("Deref")),
                "{item} must not deref: {line}"
            );
        }
    }
}

/// The per-unit checkout facade holds no instance: it neither derefs nor
/// exposes one, and sessions borrow theirs only inside the body.
#[test]
fn resource_handle_has_no_deref() {
    let text = resource_signatures(&workspace());
    let lines = section(&text, "nebula_resource::call::row::ResourceHandle");
    for method in ["pub fn submit<", "pub fn session<"] {
        assert!(
            lines.iter().any(|line| line.contains(method)),
            "ResourceHandle's `{method}` must be rendered: {lines:#?}"
        );
    }
    for line in &lines {
        assert!(
            !(line.starts_with("impl") && line.contains("Deref")),
            "ResourceHandle must not deref: {line}"
        );
    }
}

/// The HTTP adapter's public signatures name no `reqwest` or `url` type,
/// and its transport exposes nothing but its constructor.
#[test]
fn the_http_adapter_leaks_no_client_type() {
    let text = resource_signatures(&workspace());
    let http: Vec<&str> = text
        .split("\n## ")
        .filter(|section| section.contains("nebula_sdk::integration::resource::http::"))
        .collect();
    assert!(http.len() > 10, "the http sections must be rendered");
    for section in &http {
        assert!(
            !section.contains("reqwest") && !section.contains("Url"),
            "a client type leaked: {section}"
        );
    }
    let transport = section(
        &text,
        "nebula_sdk::integration::resource::http::config::HttpTransport",
    );
    let methods: Vec<&&str> = transport
        .iter()
        .filter(|line| line.trim_start().starts_with("pub fn"))
        .collect();
    assert_eq!(
        methods,
        [&"    pub fn new(config: &HttpConfig) -> Result<Self, Error>;"]
    );
}

/// Pins the walker to the facts the snapshot exists to show.
#[test]
fn limited_has_no_deref_and_marks_interim_calls() {
    let text = resource_signatures(&workspace());
    let limited = section(&text, "nebula_resource::rate_limit::Limited");
    assert!(
        limited
            .iter()
            .any(|line| line.contains("pub async fn run<")),
        "Limited's inherent methods must be rendered: {limited:#?}"
    );
    for line in &limited {
        assert!(
            !(line.starts_with("impl") && line.contains("Deref")),
            "Limited must not deref to its client: {line}"
        );
    }
    assert!(
        text.contains("\n## struct nebula_resource::rate_limit::Limited [interim]\n"),
        "Limited itself is documented as interim surface"
    );
    assert!(
        limited.iter().any(|line| line.starts_with("#[deprecated")),
        "Limited is deprecated in favour of the managed call facade: {limited:#?}"
    );
    for method in [
        "fn run<",
        "fn run_until<",
        "fn run_for<",
        "fn run_for_until<",
        "fn unlimited(",
    ] {
        let line = limited
            .iter()
            .find(|line| line.contains(method))
            .unwrap_or_else(|| panic!("Limited has no `{method}`"));
        assert!(
            line.ends_with("[interim]"),
            "{method} must be [interim]: {line}"
        );
    }
}
