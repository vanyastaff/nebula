use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    XtaskError,
    workspace::{Owner, Workspace},
};

const MAX_PLAN_ENTRIES: usize = 256;
const MAX_PLAN_BYTES: usize = 450 * 1024;
const MAX_INPUT_PATHS: usize = 4096;

#[derive(Debug, Serialize)]
struct Plan {
    schema_version: u8,
    packages: BTreeSet<String>,
    standalone_manifests: BTreeSet<String>,
    fixtures: Vec<Fixture>,
}

#[derive(Debug, Serialize)]
struct Fixture {
    manifest_path: String,
    owner: String,
    test_target: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FixturePolicy {
    owner: Option<String>,
    test_target: String,
}

/// Failures while binding changed paths to their Cargo-owned verification targets.
#[derive(Debug, Error)]
pub enum PlanError {
    /// A path cannot be represented safely in the relative-path protocol.
    #[error("pre-commit path must be a nonempty workspace-relative path: {0}")]
    InvalidPath(PathBuf),
    /// Cargo metadata provides no unique enclosing package.
    #[error("pre-commit path has no unambiguous Cargo workspace owner: {0}")]
    MissingOwner(PathBuf),
    /// A nested manifest is not an independently checkable workspace.
    #[error("pre-commit manifest is neither a workspace member nor a standalone package: {0}")]
    UnregisteredManifest(PathBuf),
    /// A fixture declaration violates the ownership policy.
    #[error("fixture policy in `{path}` is invalid: {detail}")]
    InvalidFixture {
        /// Manifest containing the declaration.
        path: PathBuf,
        /// Violated ownership invariant.
        detail: String,
    },
    /// A fixture declaration cannot be decoded into the closed policy shape.
    #[error("fixture policy in `{path}` cannot be decoded: {source}")]
    FixturePolicyParse {
        /// Manifest containing the declaration.
        path: PathBuf,
        /// Original structured TOML decoder error.
        #[source]
        source: toml::de::Error,
    },
    /// An optional owner assertion disagrees with Cargo-derived ownership.
    #[error("fixture `{path}` declares owner `{declared}`, but Cargo ownership is `{actual}`")]
    OwnerMismatch {
        /// Manifest containing the declaration.
        path: PathBuf,
        /// Owner asserted by the declaration.
        declared: String,
        /// Owner derived from workspace containment.
        actual: String,
    },
    /// The owner has no integration-test target with the declared name.
    #[error("fixture `{path}` names nonexistent integration test `{target}` in `{owner}`")]
    UnknownTestTarget {
        /// Manifest containing the declaration.
        path: PathBuf,
        /// Cargo-derived package name.
        owner: String,
        /// Declared integration-test target.
        target: String,
    },
    /// Input count or serialized plan size exceeds the protocol budget.
    #[error("pre-commit plan exceeds its bounded input or output size")]
    TooLarge,
}

pub(crate) fn plan(cwd: &Path, paths: &[PathBuf]) -> Result<Vec<u8>, XtaskError> {
    if paths.len() > MAX_INPUT_PATHS {
        return Err(PlanError::TooLarge.into());
    }
    let workspace = Workspace::load(cwd)?;
    let mut packages = BTreeSet::new();
    let mut standalone_manifests = BTreeSet::new();
    let mut fixtures = BTreeMap::new();

    for path in paths {
        let path = relative_path(path)?;
        let (manifest_path, manifest) = nearest_manifest(workspace.root(), &path)?;
        let policy = fixture_policy(&manifest_path, &manifest)?;
        let owner = match workspace.owner(&path) {
            Owner::Package(id) => Some(workspace.package(id)?),
            Owner::Ambiguous => return Err(PlanError::MissingOwner(path).into()),
            Owner::None => None,
        };
        let is_member = owner
            .is_some_and(|package| package.manifest_directory.join("Cargo.toml") == manifest_path);
        if is_member {
            if policy.is_some() {
                return Err(PlanError::InvalidFixture {
                    path: manifest_path,
                    detail: "fixture declarations require an isolated standalone manifest"
                        .to_owned(),
                }
                .into());
            }
            let owner = owner.ok_or_else(|| PlanError::MissingOwner(path.clone()))?;
            packages.insert(owner.name.clone());
            continue;
        }
        if !manifest.get("workspace").is_some_and(toml::Value::is_table) {
            return Err(PlanError::UnregisteredManifest(manifest_path).into());
        }
        let manifest_text = wire_path(&manifest_path)?;
        if let Some(policy) = policy {
            let owner = owner.ok_or_else(|| PlanError::MissingOwner(path.clone()))?;
            if let Some(declared) = policy.owner
                && declared != owner.name
            {
                return Err(PlanError::OwnerMismatch {
                    path: manifest_path,
                    declared,
                    actual: owner.name.clone(),
                }
                .into());
            }
            if !owner.test_targets.contains(&policy.test_target) {
                return Err(PlanError::UnknownTestTarget {
                    path: manifest_path,
                    owner: owner.name.clone(),
                    target: policy.test_target,
                }
                .into());
            }
            packages.insert(owner.name.clone());
            fixtures.insert(
                manifest_text.clone(),
                Fixture {
                    manifest_path: manifest_text,
                    owner: owner.name.clone(),
                    test_target: policy.test_target,
                },
            );
        } else {
            standalone_manifests.insert(manifest_text);
        }
    }

    if packages.len() + standalone_manifests.len() + fixtures.len() > MAX_PLAN_ENTRIES {
        return Err(PlanError::TooLarge.into());
    }
    let plan = Plan {
        schema_version: 1,
        packages,
        standalone_manifests,
        fixtures: fixtures.into_values().collect(),
    };
    let mut output = serde_json::to_vec(&plan)?;
    output.push(b'\n');
    if output.len() > MAX_PLAN_BYTES {
        return Err(PlanError::TooLarge.into());
    }
    Ok(output)
}

fn relative_path(path: &Path) -> Result<PathBuf, PlanError> {
    // Validate the caller's spelling before PathBuf inserts native separators.
    let raw = path
        .to_str()
        .filter(|text| !text.is_empty() && !text.chars().any(|ch| ch.is_control() || ch == '\\'))
        .ok_or_else(|| PlanError::InvalidPath(path.to_path_buf()))?;
    let mut relative = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {},
            _ => return Err(PlanError::InvalidPath(path.to_path_buf())),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(PlanError::InvalidPath(path.to_path_buf()));
    }
    Ok(relative)
}

fn wire_path(path: &Path) -> Result<String, PlanError> {
    let mut encoded = String::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(PlanError::InvalidPath(path.to_path_buf()));
        };
        let text = part
            .to_str()
            .filter(|text| !text.chars().any(|ch| ch.is_control() || ch == '\\'))
            .ok_or_else(|| PlanError::InvalidPath(path.to_path_buf()))?;
        if !encoded.is_empty() {
            encoded.push('/');
        }
        encoded.push_str(text);
    }
    if encoded.is_empty() {
        return Err(PlanError::InvalidPath(path.to_path_buf()));
    }
    Ok(encoded)
}

fn nearest_manifest(root: &Path, path: &Path) -> Result<(PathBuf, toml::Table), XtaskError> {
    // Windows canonical paths may carry an extended prefix absent from Cargo metadata.
    let canonical_root =
        root.canonicalize()
            .map_err(|source| XtaskError::WorkspaceManifestRead {
                path: root.to_path_buf(),
                source,
            })?;
    let parent = path
        .parent()
        .ok_or_else(|| PlanError::InvalidPath(path.to_path_buf()))?;
    for directory in parent.ancestors() {
        let relative = directory.join("Cargo.toml");
        let absolute = root.join(&relative);
        let canonical = match absolute.canonicalize() {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(XtaskError::WorkspaceManifestRead {
                    path: absolute,
                    source,
                });
            },
        };
        if !canonical.starts_with(&canonical_root) {
            return Err(PlanError::InvalidPath(relative).into());
        }
        let source = std::fs::read_to_string(&absolute).map_err(|source| {
            XtaskError::WorkspaceManifestRead {
                path: absolute.clone(),
                source,
            }
        })?;
        let manifest = toml::from_str::<toml::Table>(&source).map_err(|source| {
            XtaskError::WorkspaceManifestParse {
                path: absolute,
                source,
            }
        })?;
        if manifest.get("package").is_some_and(toml::Value::is_table) {
            return Ok((relative, manifest));
        }
    }
    Err(PlanError::MissingOwner(path.to_path_buf()).into())
}

fn fixture_policy(path: &Path, manifest: &toml::Table) -> Result<Option<FixturePolicy>, PlanError> {
    let invalid = |detail| PlanError::InvalidFixture {
        path: path.to_path_buf(),
        detail,
    };
    let mut table = manifest;
    for key in ["package", "metadata", "nebula"] {
        let Some(value) = table.get(key) else {
            return Ok(None);
        };
        table = value
            .as_table()
            .ok_or_else(|| invalid(format!("`{key}` must be a table")))?;
    }
    table
        .get("fixture")
        .map(|value| {
            value.clone().try_into::<FixturePolicy>().map_err(|source| {
                PlanError::FixturePolicyParse {
                    path: path.to_path_buf(),
                    source,
                }
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::{PlanError, relative_path, wire_path};
    use std::path::{Path, PathBuf};

    #[test]
    fn native_components_encode_as_git_paths() {
        let native: PathBuf = ["tools", "independent probe", "Cargo.toml"]
            .iter()
            .collect();
        assert_eq!(
            wire_path(&native).expect("normal components have a portable wire spelling"),
            "tools/independent probe/Cargo.toml"
        );
    }

    #[test]
    fn git_paths_normalize_to_native_paths_without_changing_components() {
        let expected: PathBuf = ["crates", "author", "src", "lib.rs"].iter().collect();
        for source in ["crates/author/src/lib.rs", "./crates//author/./src/lib.rs"] {
            let native = relative_path(Path::new(source)).expect("Git-relative path is valid");
            assert_eq!(native, expected);
            assert_eq!(wire_path(&native).unwrap(), "crates/author/src/lib.rs");
        }
    }

    #[test]
    fn raw_protocol_rejects_backslashes_controls_roots_and_traversal() {
        for source in [
            "",
            ".",
            "../file.rs",
            "crates/../file.rs",
            "/crates/file.rs",
            "crates\\author\\file.rs",
            "crates/author\\file.rs",
            "crates/bad\nname/file.rs",
            "crates/bad\u{0085}name/file.rs",
        ] {
            std::assert_matches!(
                relative_path(Path::new(source)),
                Err(PlanError::InvalidPath(_))
            );
        }
    }

    #[test]
    fn wire_encoding_rejects_invalid_components() {
        for source in [
            "",
            "../Cargo.toml",
            "/Cargo.toml",
            "bad\nname/Cargo.toml",
            "bad\u{0085}name/Cargo.toml",
        ] {
            std::assert_matches!(wire_path(Path::new(source)), Err(PlanError::InvalidPath(_)));
        }
    }
}
