use std::{collections::BTreeSet, path::Path};

use cargo_metadata::PackageId;
use serde::Serialize;

use crate::{
    XtaskError,
    changes::{ChangeKind, Changes, PathRole},
    workspace::{Owner, Workspace},
};

const SCHEMA_VERSION: u8 = 1;
const MAX_ENTRIES: usize = 256;
// GitHub accounts job outputs approximately as UTF-16. Keeping the complete
// UTF-8 plan below 450 KiB leaves headroom below its 1 MiB per-job limit after
// the matrix wrapper, output keys, and worst-case two-byte accounting.
const MAX_OUTPUT_BYTES: usize = 450 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct Plan {
    schema_version: u8,
    scope: Scope,
    reason: String,
    count: usize,
    pub(crate) include: Vec<PlanEntry>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Scope {
    Full,
    Diff,
}

#[derive(Debug, Serialize)]
pub(crate) struct PlanEntry {
    pub(crate) package: String,
    pub(crate) test_features: Vec<String>,
}

impl Plan {
    pub(crate) fn full(workspace: &Workspace, reason: &str) -> Result<Self, XtaskError> {
        Self::new(Scope::Full, reason, workspace.all_entries()?)
    }

    pub(crate) fn from_changes(
        workspace: &Workspace,
        changes: Changes,
    ) -> Result<Self, XtaskError> {
        if let Some(reason) = changes.conservative_reason {
            return Self::full(workspace, &format!("conservative-diff:{reason}"));
        }
        if let Some(path) = first_path_of_kind(&changes, ChangeKind::Deleted) {
            return Self::full(workspace, &format!("deleted-path:{path}"));
        }
        if let Some(path) = changes
            .paths
            .iter()
            .map(|change| change.path.as_str())
            .find(|path| is_bootstrap_path(path))
        {
            return Self::full(workspace, &format!("bootstrap-change:{path}"));
        }
        if let Some(path) = changes
            .paths
            .iter()
            .map(|change| change.path.as_str())
            .find(|path| is_excluded_fuzz_path(path))
        {
            return Self::full(workspace, &format!("excluded-fuzz-change:{path}"));
        }

        let mut owners = BTreeSet::<PackageId>::new();
        let mut saw_docs_or_assets = false;
        for change in &changes.paths {
            match workspace.owner(Path::new(&change.path)) {
                Owner::Package(id) => {
                    owners.insert(id.clone());
                },
                Owner::Ambiguous => {
                    return Self::full(
                        workspace,
                        &format!("ambiguous-package-owner:{}", change.path),
                    );
                },
                Owner::None if is_docs_or_assets_path(&change.path) => {
                    saw_docs_or_assets = true;
                },
                Owner::None => {
                    let prefix = if change.role == PathRole::Old {
                        "unresolved-old-owner"
                    } else {
                        "unowned-path"
                    };
                    return Self::full(workspace, &format!("{prefix}:{}", change.path));
                },
            }
        }

        if owners.is_empty() {
            let reason = if changes.paths.is_empty() {
                "no-changes"
            } else if saw_docs_or_assets {
                "docs-assets-only"
            } else {
                "no-package-changes"
            };
            return Self::new(Scope::Diff, reason, Vec::new());
        }
        Self::new(
            Scope::Diff,
            "workspace-packages-changed",
            workspace.entries(owners)?,
        )
    }

    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, XtaskError> {
        let mut output = serde_json::to_vec(self)?;
        output.push(b'\n');
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(XtaskError::OutputTooLarge {
                size: output.len(),
                maximum: MAX_OUTPUT_BYTES,
            });
        }
        Ok(output)
    }

    fn new(
        scope: Scope,
        reason: impl Into<String>,
        mut include: Vec<PlanEntry>,
    ) -> Result<Self, XtaskError> {
        include.sort_by(|left, right| left.package.cmp(&right.package));
        include.dedup_by(|left, right| left.package == right.package);
        if include.len() > MAX_ENTRIES {
            return Err(XtaskError::TooManyEntries {
                count: include.len(),
                maximum: MAX_ENTRIES,
            });
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            scope,
            reason: reason.into(),
            count: include.len(),
            include,
        })
    }
}

fn first_path_of_kind(changes: &Changes, kind: ChangeKind) -> Option<&str> {
    changes
        .paths
        .iter()
        .find(|change| change.kind == kind)
        .map(|change| change.path.as_str())
}

fn is_bootstrap_path(path: &str) -> bool {
    matches!(
        path,
        "Cargo.toml"
            | "Cargo.lock"
            | "Taskfile.yml"
            | "deny.toml"
            | ".github/workflows/test-matrix.yml"
            | "scripts/pre-push-crate-diff.sh"
    ) || path.starts_with(".cargo/")
        || path.starts_with("tools/xtask/")
        || path == "rust-toolchain"
        || path.starts_with("rust-toolchain.")
}

fn is_excluded_fuzz_path(path: &str) -> bool {
    let mut components = path.split('/');
    matches!(components.next(), Some("crates"))
        && components.next().is_some()
        && matches!(components.next(), Some("fuzz"))
}

fn is_docs_or_assets_path(path: &str) -> bool {
    let has_documentation_extension = Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            ["md", "png", "jpg", "jpeg", "svg"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        });
    has_documentation_extension
        || path.starts_with("docs/")
        || path.starts_with("assets/")
        || path.starts_with(".vscode/")
        || path.starts_with(".idea/")
        || path.starts_with(".github/ISSUE_TEMPLATE/")
        || (path.starts_with("LICENSE") && !path.contains('/'))
        || matches!(
            path,
            "LICENSE"
                | "LICENSE-APACHE"
                | "LICENSE-MIT"
                | ".gitignore"
                | ".gitattributes"
                | ".editorconfig"
                | "CODEOWNERS"
                | ".github/PULL_REQUEST_TEMPLATE.md"
                | ".github/labeler.yml"
                | ".github/dependabot.yml"
        )
}

#[cfg(test)]
mod tests {
    use crate::XtaskError;

    use super::{
        MAX_OUTPUT_BYTES, Plan, PlanEntry, Scope, is_bootstrap_path, is_docs_or_assets_path,
        is_excluded_fuzz_path,
    };

    #[test]
    fn bootstrap_paths_are_explicit_and_tooling_is_always_full() {
        assert!(is_bootstrap_path("Cargo.toml"));
        assert!(is_bootstrap_path(".cargo/config.toml"));
        assert!(is_bootstrap_path("tools/xtask/README.md"));
        assert!(!is_bootstrap_path(".github/workflows/docs.yml"));
    }

    #[test]
    fn fuzz_and_docs_paths_are_classified_without_package_inference() {
        assert!(is_excluded_fuzz_path(
            "crates/parser/fuzz/fuzz_targets/value.rs"
        ));
        assert!(!is_excluded_fuzz_path("crates/parser/src/fuzz.rs"));
        assert!(is_docs_or_assets_path("crates/parser/README.md"));
        assert!(is_docs_or_assets_path("assets/architecture.svg"));
        assert!(is_docs_or_assets_path("LICENSE-BSD"));
        assert!(!is_docs_or_assets_path("scripts/release.sh"));
        assert!(!is_docs_or_assets_path(".claude/hooks/edit-guard.sh"));
    }

    #[test]
    fn serialized_output_has_conservative_github_size_limit() {
        let plan = Plan::new(
            Scope::Full,
            "size-test",
            vec![PlanEntry {
                package: "fixture".to_owned(),
                test_features: vec!["x".repeat(MAX_OUTPUT_BYTES)],
            }],
        )
        .expect("entry count is valid");

        assert!(matches!(
            plan.to_json_line(),
            Err(XtaskError::OutputTooLarge { .. })
        ));
    }
}
