//! The supported SDK's lockstep publication boundary and actual Cargo archives.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::Command,
};

use cargo_metadata::{DependencyKind, Metadata, MetadataCommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_SET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DECODED_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REPORT_BYTES: u64 = 4 * 1024 * 1024;

fn sha256(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    encoded
}

#[derive(Debug, Error)]
pub enum PackagingError {
    #[error("publication contract: {0}")]
    Contract(String),
    #[error("publication metadata: {0}")]
    Metadata(#[from] cargo_metadata::Error),
    #[error("publication I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("publication manifest: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("publication report: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicationPlan {
    contract: String,
    producer_version: u32,
    sdk_version: String,
    sdk_features: Vec<String>,
    workspace_packages: Vec<String>,
    packages: Vec<PublicationPackage>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationPackage {
    name: String,
    version: String,
    manifest: PathBuf,
    internal_dependencies: Vec<PublicationDependency>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationDependency {
    package: String,
    key: String,
    kind: String,
    target: Option<String>,
    version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationReport {
    contract: String,
    producer_version: u32,
    plan: PublicationPlan,
    verification_mode: String,
    commands: Vec<CommandObservation>,
    archives: Vec<ArchiveObservation>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandObservation {
    arguments: Vec<String>,
    exit_code: i32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchiveObservation {
    package: String,
    version: String,
    sha256: String,
    bytes: u64,
    normalized_manifest: String,
}

pub(crate) fn plan(root: &Path) -> Result<PublicationPlan, PackagingError> {
    let workspace = MetadataCommand::new().current_dir(root).no_deps().exec()?;
    plan_from_metadata(&workspace)
}

fn plan_from_metadata(workspace: &Metadata) -> Result<PublicationPlan, PackagingError> {
    let members = workspace.workspace_members.iter().collect::<BTreeSet<_>>();
    let local = workspace
        .packages
        .iter()
        .filter(|p| members.contains(&p.id))
        .collect::<Vec<_>>();
    let names = local
        .iter()
        .map(|p| p.name.to_string())
        .collect::<BTreeSet<_>>();
    let by_name = local
        .iter()
        .map(|p| (p.name.to_string(), *p))
        .collect::<BTreeMap<_, _>>();
    let sdk = by_name
        .get("nebula-sdk")
        .ok_or_else(|| PackagingError::Contract("missing supported nebula-sdk package".into()))?;
    let sdk_features = sdk.features.keys().cloned().collect();
    // All declared normal/build path edges are publication dependencies, even
    // optional or target-specific edges: Cargo retains them in the normalized
    // archive and must resolve their registry versions. Dev-only edges do not
    // enlarge the product boundary, but their retained archive pins are checked.
    let mut pending = vec!["nebula-sdk".to_owned()];
    let mut selected = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !selected.insert(name.clone()) {
            continue;
        }
        let package = by_name.get(&name).ok_or_else(|| {
            PackagingError::Contract(format!(
                "local publication dependency `{name}` is outside the workspace"
            ))
        })?;
        pending.extend(
            package
                .dependencies
                .iter()
                .filter(|d| {
                    (d.path.is_some() || names.contains(&d.name))
                        && d.kind != DependencyKind::Development
                })
                .map(|d| d.name.clone()),
        );
    }
    let version = sdk.version.to_string();
    let mut packages = Vec::new();
    for package in local {
        let included = selected.contains(package.name.as_str());
        let publishable = package
            .publish
            .as_ref()
            .is_none_or(|registries| !registries.is_empty());
        if included != publishable {
            return Err(PackagingError::Contract(format!(
                "package `{}` publish flag must be {} for the SDK closure",
                package.name, included
            )));
        }
        if !included {
            continue;
        }
        if package.version.to_string() != version {
            return Err(PackagingError::Contract(format!(
                "package `{}` must use SDK lockstep version {version}",
                package.name
            )));
        }
        let mut dependencies = Vec::new();
        for dependency in &package.dependencies {
            if dependency.path.is_none() && !names.contains(&dependency.name) {
                continue;
            }
            if !names.contains(&dependency.name) {
                return Err(PackagingError::Contract(format!(
                    "package `{}` has non-workspace path dependency `{}`",
                    package.name, dependency.name
                )));
            }
            let wanted = format!("={version}");
            if dependency.req.to_string() != wanted {
                return Err(PackagingError::Contract(format!(
                    "package `{}` dependency `{}` requires `{}`, expected exact `{wanted}` alongside its path",
                    package.name, dependency.name, dependency.req
                )));
            }
            let kind = match dependency.kind {
                DependencyKind::Normal => "dependencies",
                DependencyKind::Build => "build-dependencies",
                DependencyKind::Development => "dev-dependencies",
                _ => {
                    return Err(PackagingError::Contract(
                        "unknown Cargo dependency kind".into(),
                    ));
                },
            };
            dependencies.push(PublicationDependency {
                package: dependency.name.clone(),
                key: dependency
                    .rename
                    .clone()
                    .unwrap_or_else(|| dependency.name.clone()),
                kind: kind.into(),
                target: dependency.target.as_ref().map(ToString::to_string),
                version: wanted,
            });
        }
        dependencies.sort();
        packages.push(PublicationPackage {
            name: package.name.to_string(),
            version: version.clone(),
            manifest: package.manifest_path.clone().into(),
            internal_dependencies: dependencies,
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(PublicationPlan {
        contract: "published-manifest-precision".into(),
        producer_version: 1,
        sdk_version: version,
        sdk_features,
        workspace_packages: names.into_iter().collect(),
        packages,
    })
}

pub(crate) fn verify(root: &Path, output: &Path) -> Result<Vec<u8>, PackagingError> {
    let plan = plan(root)?;
    fs::create_dir(output)?;
    let output = output.canonicalize()?;
    let target = output.join("cargo-package-target");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut commands = Vec::new();
    for (label, prefix) in [
        (
            "package",
            vec!["package", "--locked", "--allow-dirty", "--all-features"],
        ),
        (
            "publish-dry-run",
            vec![
                "publish",
                "--dry-run",
                "--locked",
                "--allow-dirty",
                "--all-features",
            ],
        ),
    ] {
        let mut arguments = prefix.into_iter().map(str::to_owned).collect::<Vec<_>>();
        for package in &plan.packages {
            arguments.extend(["-p".into(), package.name.clone()]);
        }
        // Cargo packages the entire selected dependency graph into its temporary
        // registry overlay, then verifies the real archives without publication.
        let result = Command::new(&cargo)
            .current_dir(root)
            .env("CARGO_TARGET_DIR", &target)
            .args(&arguments)
            .output()?;
        fs::write(output.join(format!("{label}.stdout.log")), &result.stdout)?;
        fs::write(output.join(format!("{label}.stderr.log")), &result.stderr)?;
        if !result.status.success() {
            return Err(PackagingError::Contract(format!(
                "cargo {label} failed with {}; diagnostics in {}",
                result.status,
                output.display()
            )));
        }
        commands.push(CommandObservation {
            arguments,
            exit_code: 0,
        });
    }
    let mut archives = Vec::new();
    let archive_inventory = output.join("archives");
    fs::create_dir(&archive_inventory)?;
    for package in &plan.packages {
        let archive_path = target
            .join("package")
            .join(format!("{}-{}.crate", package.name, package.version));
        let bytes = fs::read(&archive_path)?;
        fs::copy(
            &archive_path,
            archive_inventory.join(format!("{}-{}.crate", package.name, package.version)),
        )?;
        let normalized_manifest = archive_manifest(package, &bytes)?;
        verify_normalized_manifest(package, &normalized_manifest, &plan.workspace_packages)?;
        archives.push(ArchiveObservation {
            package: package.name.clone(),
            version: package.version.clone(),
            sha256: sha256(&bytes),
            bytes: bytes.len() as u64,
            normalized_manifest,
        });
    }
    let report = PublicationReport {
        contract: "published-manifest-precision".into(),
        producer_version: 1,
        plan,
        verification_mode: "cargo-workspace-temporary-registry-overlay".into(),
        commands,
        archives,
    };
    let mut json = serde_json::to_vec_pretty(&report)?;
    json.push(b'\n');
    verify_archives(root, &serde_json::from_slice(&json)?, &archive_inventory)?;
    fs::write(output.join("published-manifest-precision.json"), &json)?;
    Ok(json)
}

fn verify_normalized_manifest(
    package: &PublicationPackage,
    source: &str,
    known_packages: &[String],
) -> Result<(), PackagingError> {
    let manifest: toml::Value = toml::from_str(source)?;
    let identity = manifest.get("package").ok_or_else(|| {
        PackagingError::Contract("normalized manifest lacks package identity".into())
    })?;
    if identity.get("name").and_then(toml::Value::as_str) != Some(package.name.as_str())
        || identity.get("version").and_then(toml::Value::as_str) != Some(package.version.as_str())
    {
        return Err(PackagingError::Contract(format!(
            "archive identity differs for `{}`",
            package.name
        )));
    }
    fn dependencies(
        value: &toml::Value,
        target: Option<&str>,
        known_packages: &[String],
        found: &mut Vec<PublicationDependency>,
    ) -> Result<(), PackagingError> {
        for kind in ["dependencies", "build-dependencies", "dev-dependencies"] {
            let Some(deps) = value.get(kind).and_then(toml::Value::as_table) else {
                continue;
            };
            for (key, dependency) in deps {
                if dependency.get("path").is_some() || dependency.get("workspace").is_some() {
                    return Err(PackagingError::Contract(format!(
                        "normalized dependency `{key}` retains a local path/workspace reference"
                    )));
                }
                let actual = dependency
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                if known_packages.iter().any(|name| name == actual) {
                    let version = dependency
                        .get("version")
                        .and_then(toml::Value::as_str)
                        .ok_or_else(|| {
                            PackagingError::Contract(format!(
                                "normalized dependency `{actual}` has no version"
                            ))
                        })?;
                    found.push(PublicationDependency {
                        package: actual.into(),
                        key: key.clone(),
                        kind: kind.into(),
                        target: target.map(str::to_owned),
                        version: version.into(),
                    });
                }
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    dependencies(&manifest, None, known_packages, &mut found)?;
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, value) in targets {
            dependencies(value, Some(target), known_packages, &mut found)?;
        }
    }
    found.sort();
    if found != package.internal_dependencies {
        return Err(PackagingError::Contract(format!(
            "archive `{}` internal dependency declarations differ in package, alias, kind, target, or exact lockstep version",
            package.name
        )));
    }
    Ok(())
}

/// Rechecks producer observations against this revision's metadata boundary.
/// Runner provenance must be admitted separately by the CP3 artifact verifier.
pub(crate) fn verify_observation(
    root: &Path,
    value: &serde_json::Value,
) -> Result<(), PackagingError> {
    let report: PublicationReport = serde_json::from_value(value.clone())?;
    let expected = plan(root)?;
    if report.contract != "published-manifest-precision"
        || report.producer_version != 1
        || report.plan.contract != expected.contract
        || report.plan.producer_version != 1
        || report.plan.sdk_version != expected.sdk_version
        || report.plan.sdk_features != expected.sdk_features
        || report.plan.workspace_packages != expected.workspace_packages
        || report.verification_mode != "cargo-workspace-temporary-registry-overlay"
        || report.plan.packages.len() != expected.packages.len()
        || report.archives.len() != expected.packages.len()
    {
        return Err(PackagingError::Contract(
            "observation publication boundary differs from checked-out metadata".into(),
        ));
    }
    for (actual, wanted) in report.plan.packages.iter().zip(&expected.packages) {
        if actual.name != wanted.name
            || actual.version != wanted.version
            || actual.internal_dependencies != wanted.internal_dependencies
        {
            return Err(PackagingError::Contract(format!(
                "observation package `{}` differs from metadata",
                wanted.name
            )));
        }
    }
    let prefixes = [
        vec!["package", "--locked", "--allow-dirty", "--all-features"],
        vec![
            "publish",
            "--dry-run",
            "--locked",
            "--allow-dirty",
            "--all-features",
        ],
    ];
    if report.commands.len() != prefixes.len() {
        return Err(PackagingError::Contract(
            "observation requires both actual Cargo package and publish dry-run".into(),
        ));
    }
    for (command, prefix) in report.commands.iter().zip(prefixes) {
        let mut arguments = prefix.into_iter().map(str::to_owned).collect::<Vec<_>>();
        for package in &expected.packages {
            arguments.extend(["-p".into(), package.name.clone()]);
        }
        if command.arguments != arguments || command.exit_code != 0 {
            return Err(PackagingError::Contract(
                "observation command selection, verification flags, or exit status differs".into(),
            ));
        }
    }
    for (archive, expected_package) in report.archives.iter().zip(&expected.packages) {
        let observed_identity = (&archive.package, &archive.version);
        let expected_identity = (&expected_package.name, &expected_package.version);
        if observed_identity != expected_identity
            || archive.bytes == 0
            || archive.sha256.len() != 64
            || !archive
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(PackagingError::Contract(format!(
                "observation archive identity/digest differs for `{}`",
                expected_package.name
            )));
        }
        verify_normalized_manifest(
            expected_package,
            &archive.normalized_manifest,
            &expected.workspace_packages,
        )?;
    }
    Ok(())
}

/// Verifies the actual binary archive inventory after trusted CP3 provenance
/// admission. The directory must contain exactly the selected `.crate` files.
pub(crate) fn verify_archives(
    root: &Path,
    value: &serde_json::Value,
    archive_root: &Path,
) -> Result<(), PackagingError> {
    verify_observation(root, value)?;
    let report: PublicationReport = serde_json::from_value(value.clone())?;
    let expected = plan(root)?;
    let root_metadata = fs::symlink_metadata(archive_root)?;
    if !root_metadata.is_dir() || is_link(&root_metadata) {
        return Err(PackagingError::Contract(
            "archive inventory root is not a regular directory".into(),
        ));
    }
    let filenames = expected
        .packages
        .iter()
        .map(|p| format!("{}-{}.crate", p.name, p.version))
        .collect::<BTreeSet<_>>();
    let actual = fs::read_dir(archive_root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual
        != filenames
            .iter()
            .map(|name| std::ffi::OsString::from(name.as_str()))
            .collect()
    {
        return Err(PackagingError::Contract(
            "actual archive inventory differs from selected publication packages".into(),
        ));
    }
    let mut total = 0_u64;
    for (package, observation) in expected.packages.iter().zip(&report.archives) {
        let path = archive_root.join(format!("{}-{}.crate", package.name, package.version));
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || is_link(&metadata) || metadata.len() > MAX_ARCHIVE_BYTES {
            return Err(PackagingError::Contract(format!(
                "archive `{}` is not a bounded regular file",
                package.name
            )));
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(MAX_ARCHIVE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        total += bytes.len() as u64;
        if bytes.len() as u64 > MAX_ARCHIVE_BYTES || total > MAX_ARCHIVE_SET_BYTES {
            return Err(PackagingError::Contract(
                "archive inventory exceeds the bounded verification limit".into(),
            ));
        }
        if observation.bytes != bytes.len() as u64 || observation.sha256 != sha256(&bytes) {
            return Err(PackagingError::Contract(format!(
                "actual archive `{}` digest/size differs from admitted observation",
                package.name
            )));
        }
        let source = archive_manifest(package, &bytes)?;
        if source != observation.normalized_manifest {
            return Err(PackagingError::Contract(format!(
                "actual archive `{}` manifest differs from admitted observation",
                package.name
            )));
        }
        verify_normalized_manifest(package, &source, &expected.workspace_packages)?;
    }
    Ok(())
}

pub(crate) fn verify_archive_report(
    root: &Path,
    report: &Path,
    archive_root: &Path,
) -> Result<Vec<u8>, PackagingError> {
    let metadata = fs::symlink_metadata(report)?;
    if !metadata.is_file() || is_link(&metadata) || metadata.len() > MAX_REPORT_BYTES {
        return Err(PackagingError::Contract(
            "publication report is not a bounded regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(report)?
        .take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err(PackagingError::Contract(
            "publication report exceeds verification limit".into(),
        ));
    }
    verify_archives(root, &serde_json::from_slice(&bytes)?, archive_root)?;
    Ok(b"{\"contract\":\"published-manifest-precision\",\"status\":\"verified\"}\n".to_vec())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn archive_manifest(package: &PublicationPackage, bytes: &[u8]) -> Result<String, PackagingError> {
    let wanted = format!("{}-{}/Cargo.toml", package.name, package.version);
    let mut decoded = Vec::new();
    flate2::read::MultiGzDecoder::new(bytes)
        .take(MAX_DECODED_ARCHIVE_BYTES + 1)
        .read_to_end(&mut decoded)?;
    if decoded.len() as u64 > MAX_DECODED_ARCHIVE_BYTES {
        return Err(PackagingError::Contract(format!(
            "archive `{}` exceeds decoded verification limit",
            package.name
        )));
    }
    let mut archive = tar::Archive::new(decoded.as_slice());
    let mut normalized = None;
    for entry in archive.entries()? {
        let entry = entry?;
        if entry.path()?.as_ref() != Path::new(&wanted) {
            continue;
        }
        if normalized.is_some()
            || !entry.header().entry_type().is_file()
            || entry.size() > 1_048_576
        {
            return Err(PackagingError::Contract(format!(
                "invalid normalized manifest in archive `{}`",
                package.name
            )));
        }
        let mut source = String::new();
        entry.take(1_048_577).read_to_string(&mut source)?;
        if source.len() > 1_048_576 {
            return Err(PackagingError::Contract(
                "normalized manifest exceeds verification limit".into(),
            ));
        }
        normalized = Some(source);
    }
    normalized.ok_or_else(|| {
        PackagingError::Contract(format!(
            "archive `{}` lacks normalized Cargo.toml",
            package.name
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_archive_limit_rejects_oversized_data_after_valid_manifest() {
        use std::io::Write as _;
        let package = PublicationPackage {
            name: "nebula-sdk".into(),
            version: "0.32.0".into(),
            manifest: PathBuf::new(),
            internal_dependencies: Vec::new(),
        };
        let manifest = "[package]\nname='nebula-sdk'\nversion='0.32.0'\n";
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                "nebula-sdk-0.32.0/Cargo.toml",
                manifest.as_bytes(),
            )
            .expect("valid manifest first");
        let decoded = builder.into_inner().expect("valid complete tar");
        let mut valid = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        valid.write_all(&decoded).expect("valid compressed archive");
        assert_eq!(
            archive_manifest(&package, &valid.finish().expect("gzip"))
                .expect("valid archive accepted"),
            manifest
        );
        let mut oversized = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        oversized
            .write_all(&decoded)
            .expect("valid tar precedes oversized padding");
        let padding = vec![0_u8; 1024 * 1024];
        for _ in 0..=MAX_DECODED_ARCHIVE_BYTES / padding.len() as u64 {
            oversized
                .write_all(&padding)
                .expect("oversized decoded padding");
        }
        let error = archive_manifest(&package, &oversized.finish().expect("oversized gzip"))
            .expect_err("valid manifest cannot hide oversized decoded archive")
            .to_string();
        assert!(error.contains("decoded verification limit"), "{error}");
    }

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("fixture directory");
        fs::write(root.path().join("Cargo.toml"), "[workspace]\nresolver='3'\nmembers=['sdk','leaf','optional','build','target','dev','outside']\n").expect("workspace manifest");
        for name in [
            "sdk", "leaf", "optional", "build", "target", "dev", "outside",
        ] {
            fs::create_dir_all(root.path().join(name).join("src")).expect("source directory");
            fs::write(root.path().join(name).join("src/lib.rs"), "").expect("source");
            let publish = !matches!(name, "dev" | "outside");
            let mut manifest = format!(
                "[package]\nname='nebula-{name}'\nversion='0.32.0'\nedition='2024'\npublish={publish}\n"
            );
            if name == "sdk" {
                manifest.push_str("[dependencies]\nnebula-leaf={path='../leaf',version='=0.32.0'}\n[build-dependencies]\nnebula-build={path='../build',version='=0.32.0'}\n[target.'cfg(windows)'.dependencies]\nnebula-target={path='../target',version='=0.32.0'}\n[dev-dependencies]\nnebula-dev={path='../dev',version='=0.32.0'}\n[features]\ndefault=[]\nclient=[]\nembedded=[]\n");
            }
            if name == "leaf" {
                manifest.push_str("[dependencies]\nnebula-optional={path='../optional',version='=0.32.0',optional=true}\n");
            }
            fs::write(root.path().join(name).join("Cargo.toml"), manifest)
                .expect("package manifest");
        }
        root
    }

    #[test]
    fn publication_closure_covers_retained_optional_build_and_target_edges() {
        let root = workspace();
        let result = plan(root.path()).expect("valid exact publication boundary");
        let names = result
            .packages
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "nebula-build",
                "nebula-leaf",
                "nebula-optional",
                "nebula-sdk",
                "nebula-target"
            ]
        );
        assert_eq!(result.sdk_features, ["client", "default", "embedded"]);
    }

    #[test]
    fn missing_path_version_names_owner_and_dependency() {
        let root = workspace();
        let path = root.path().join("sdk/Cargo.toml");
        let source = fs::read_to_string(&path)
            .expect("manifest")
            .replace("path='../leaf',version='=0.32.0'", "path='../leaf'");
        fs::write(path, source).expect("modified manifest");
        let error = plan(root.path())
            .expect_err("path-only dependency rejected")
            .to_string();
        assert!(error.contains("nebula-sdk"), "{error}");
        assert!(error.contains("nebula-leaf"), "{error}");
        assert!(error.contains("=0.32.0"), "{error}");
    }

    #[test]
    fn publish_boundary_rejects_extras_and_missing_members() {
        for (name, before, after) in [
            ("outside", "publish=false", "publish=true"),
            ("optional", "publish=true", "publish=false"),
        ] {
            let root = workspace();
            let path = root.path().join(name).join("Cargo.toml");
            let source = fs::read_to_string(&path)
                .expect("manifest")
                .replace(before, after);
            fs::write(path, source).expect("modified manifest");
            let error = plan(root.path())
                .expect_err("wrong publication boundary rejected")
                .to_string();
            assert!(error.contains(&format!("nebula-{name}")), "{error}");
        }
    }

    #[test]
    fn normalized_archive_rejects_inexact_conditional_and_local_dependencies() {
        let package = PublicationPackage {
            name: "nebula-sdk".into(),
            version: "0.32.0".into(),
            manifest: PathBuf::new(),
            internal_dependencies: vec![PublicationDependency {
                package: "nebula-leaf".into(),
                key: "nebula-leaf".into(),
                kind: "dependencies".into(),
                target: None,
                version: "=0.32.0".into(),
            }],
        };
        let base = "[package]\nname='nebula-sdk'\nversion='0.32.0'\n[dependencies.nebula-leaf]\nversion='=0.32.0'\n";
        let known = ["nebula-leaf".to_owned(), "nebula-sdk".to_owned()];
        verify_normalized_manifest(&package, base, &known).expect("normalized exact dependency");
        for invalid in [
            base.replace("version='=0.32.0'", "version='0.32.0'"),
            format!("{base}path='../leaf'\n"),
            format!("{base}[target.'cfg(windows)'.dependencies.nebula-leaf]\nversion='0.32.0'\n"),
            format!("{base}[dependencies.nebula-sdk]\nversion='=0.32.0'\n"),
        ] {
            assert!(verify_normalized_manifest(&package, &invalid, &known).is_err());
        }
    }

    #[test]
    fn normalized_archive_requires_every_conditional_and_renamed_build_edge() {
        let mut edges = vec![
            PublicationDependency {
                package: "nebula-leaf".into(),
                key: "nebula-leaf".into(),
                kind: "dependencies".into(),
                target: None,
                version: "=0.32.0".into(),
            },
            PublicationDependency {
                package: "nebula-leaf".into(),
                key: "leaf-build".into(),
                kind: "build-dependencies".into(),
                target: None,
                version: "=0.32.0".into(),
            },
            PublicationDependency {
                package: "nebula-leaf".into(),
                key: "nebula-leaf".into(),
                kind: "dependencies".into(),
                target: Some("cfg(windows)".into()),
                version: "=0.32.0".into(),
            },
        ];
        edges.sort();
        let package = PublicationPackage {
            name: "nebula-sdk".into(),
            version: "0.32.0".into(),
            manifest: PathBuf::new(),
            internal_dependencies: edges,
        };
        let base = "[package]\nname='nebula-sdk'\nversion='0.32.0'\n[dependencies.nebula-leaf]\nversion='=0.32.0'\n";
        let build = "[build-dependencies.leaf-build]\npackage='nebula-leaf'\nversion='=0.32.0'\n";
        let target = "[target.'cfg(windows)'.dependencies.nebula-leaf]\nversion='=0.32.0'\n";
        let known = ["nebula-leaf".to_owned(), "nebula-sdk".to_owned()];
        verify_normalized_manifest(&package, &format!("{base}{build}{target}"), &known)
            .expect("all exact declaration identities");
        for invalid in [
            format!("{base}{build}"),
            format!("{base}{target}"),
            format!("{base}{build}{target}").replace("leaf-build]", "wrong-alias]"),
        ] {
            assert!(
                verify_normalized_manifest(&package, &invalid, &known).is_err(),
                "missing/renamed duplicated edge rejected"
            );
        }
    }

    fn packageable_workspace() -> tempfile::TempDir {
        let root = workspace();
        let path = root.path().join("sdk/Cargo.toml");
        let source = fs::read_to_string(&path).expect("manifest").replace(
            "[dev-dependencies]\nnebula-dev={path='../dev',version='=0.32.0'}\n",
            "",
        );
        fs::write(path, source).expect("packageable SDK manifest");
        MetadataCommand::new()
            .current_dir(root.path())
            .other_options(vec!["--offline".into()])
            .exec()
            .expect("fixture lockfile");
        root
    }

    #[test]
    fn actual_unpublished_lockstep_archives_and_dry_run_are_verified() {
        let root = packageable_workspace();
        let output_parent = tempfile::tempdir().expect("output parent");
        let output = output_parent.path().join("observation");
        let json =
            verify(root.path(), &output).expect("actual Cargo overlay packaging and dry-run");
        let mut value: serde_json::Value = serde_json::from_slice(&json).expect("report");
        assert_eq!(value["archives"].as_array().expect("archives").len(), 5);
        assert!(output.join("published-manifest-precision.json").is_file());
        let inventory = output.join("archives");
        verify_archives(root.path(), &value, &inventory)
            .expect("independent actual archive admission");
        verify_archive_report(
            root.path(),
            &output.join("published-manifest-precision.json"),
            &inventory,
        )
        .expect("protected CLI file-input interface");
        let first = &value["archives"][0];
        let path = inventory.join(format!(
            "{}-{}.crate",
            first["package"].as_str().expect("package"),
            first["version"].as_str().expect("version")
        ));
        let mut mutated = fs::read(&path).expect("actual .crate");
        mutated[0] ^= 1;
        fs::write(&path, &mutated).expect("mutated actual archive");
        assert!(
            verify_archives(root.path(), &value, &inventory).is_err(),
            "changed archive bytes cannot use original provenance report"
        );
        let mut forged_digest = value.clone();
        forged_digest["archives"][0]["sha256"] = serde_json::json!(sha256(&mutated));
        assert!(
            verify_archives(root.path(), &forged_digest, &inventory).is_err(),
            "matching forged digest cannot bypass actual archive parsing"
        );
        value["commands"][1]["exit_code"] = serde_json::json!(101);
        assert!(
            verify_observation(root.path(), &value).is_err(),
            "failed dry-run cannot become evidence"
        );
    }

    #[test]
    fn actual_package_build_failure_preserves_logs_without_observation() {
        let root = packageable_workspace();
        fs::write(
            root.path().join("sdk/src/lib.rs"),
            "compile_error!(\"publication failure sentinel\");",
        )
        .expect("invalid package source");
        let output_parent = tempfile::tempdir().expect("output parent");
        let output = output_parent.path().join("failed");
        let error = verify(root.path(), &output)
            .expect_err("actual Cargo package build failure")
            .to_string();
        assert!(error.contains("cargo package failed"), "{error}");
        assert!(!output.join("published-manifest-precision.json").exists());
        assert!(
            fs::read_to_string(output.join("package.stderr.log"))
                .expect("failure log")
                .contains("publication failure sentinel")
        );
    }
}
