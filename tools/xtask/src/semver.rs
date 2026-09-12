use std::{
    collections::BTreeSet,
    process::{Command, Stdio},
};

use cargo_metadata::MetadataCommand;
use serde::Serialize;

use crate::{
    XtaskError,
    changes::{self, Comparison},
    model::{Scope, Selection},
    workspace::Workspace,
};

const SCHEMA_VERSION: u8 = 1;
const MAX_ENTRIES: usize = 256;
const MAX_SHARDS: usize = 2;
const MAX_OUTPUT_BYTES: usize = 450 * 1024;

#[derive(Debug, Serialize)]
struct SemverPlan {
    schema_version: u8,
    scope: Scope,
    reason: String,
    package_count: usize,
    shard_count: usize,
    include: Vec<SemverShard>,
}

#[derive(Debug, Serialize)]
struct SemverShard {
    shard: usize,
    packages: Vec<String>,
}

pub(crate) fn plan(
    workspace: &Workspace,
    base: &str,
    head: &str,
    comparison: Comparison,
) -> Result<Vec<u8>, XtaskError> {
    let changes = changes::git_diff(workspace.root(), base, head, comparison)?;
    let selection = Selection::from_changes(workspace, changes);
    let package_names = workspace.semver_package_names(&selection.package_ids)?;
    if package_names.len() > MAX_ENTRIES {
        return Err(XtaskError::TooManyEntries {
            count: package_names.len(),
            maximum: MAX_ENTRIES,
        });
    }

    if !package_names.is_empty() {
        let baseline_package_names = load_baseline_package_names(workspace, base)?;
        if let Some(missing_package) = package_names
            .iter()
            .find(|package_name| !baseline_package_names.contains(*package_name))
        {
            return Err(XtaskError::MissingBaselinePackage(missing_package.clone()));
        }
    }

    let package_count = package_names.len();
    let include = shard_packages(package_names);
    let plan = SemverPlan {
        schema_version: SCHEMA_VERSION,
        scope: selection.scope,
        reason: selection.reason,
        package_count,
        shard_count: include.len(),
        include,
    };
    serialize_plan(&plan)
}

fn shard_packages(package_names: Vec<String>) -> Vec<SemverShard> {
    match package_names.len() {
        0 => Vec::new(),
        1 => vec![SemverShard {
            shard: 0,
            packages: package_names,
        }],
        package_count => {
            let mut first_packages = Vec::with_capacity(package_count.div_ceil(MAX_SHARDS));
            let mut second_packages = Vec::with_capacity(package_count / MAX_SHARDS);
            for (index, package_name) in package_names.into_iter().enumerate() {
                if index.is_multiple_of(MAX_SHARDS) {
                    first_packages.push(package_name);
                } else {
                    second_packages.push(package_name);
                }
            }
            vec![
                SemverShard {
                    shard: 0,
                    packages: first_packages,
                },
                SemverShard {
                    shard: 1,
                    packages: second_packages,
                },
            ]
        },
    }
}

fn load_baseline_package_names(
    workspace: &Workspace,
    base: &str,
) -> Result<BTreeSet<String>, XtaskError> {
    let baseline_tree = tempfile::tempdir().map_err(XtaskError::BaselineTempDirectory)?;
    extract_baseline(workspace, base, &baseline_tree)?;

    let mut command = MetadataCommand::new();
    command
        .current_dir(baseline_tree.path())
        .no_deps()
        .other_options(vec!["--locked".to_owned()]);
    let metadata = command.exec()?;
    let workspace_members = metadata
        .workspace_members
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut package_names = BTreeSet::new();
    for package in metadata
        .packages
        .into_iter()
        .filter(|package| workspace_members.contains(&package.id))
    {
        let package_name = package.name.to_string();
        if !package_names.insert(package_name.clone()) {
            return Err(XtaskError::DuplicateBaselinePackageName(package_name));
        }
    }
    Ok(package_names)
}

fn extract_baseline(
    workspace: &Workspace,
    base: &str,
    destination: &tempfile::TempDir,
) -> Result<(), XtaskError> {
    let mut archive = Command::new("git")
        .current_dir(workspace.root())
        .args(["archive", "--format=tar", "--", base])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| XtaskError::BaselineCommandStart {
            command: "git archive",
            source,
        })?;
    let archive_stream = if let Some(archive_stream) = archive.stdout.take() {
        archive_stream
    } else {
        let _archive_kill_result = archive.kill();
        let _archive_wait_result = archive.wait();
        return Err(XtaskError::MissingArchiveStream {
            command: "git archive",
        });
    };
    let extraction = Command::new("tar")
        .args(["--extract", "--file", "-", "--directory"])
        .arg(destination.path())
        .args(["--no-same-owner", "--no-same-permissions"])
        .env_remove("TAR_OPTIONS")
        .stdin(archive_stream)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut extraction = match extraction {
        Ok(extraction) => extraction,
        Err(source) => {
            let _archive_kill_result = archive.kill();
            let _archive_wait_result = archive.wait();
            return Err(XtaskError::BaselineCommandStart {
                command: "tar",
                source,
            });
        },
    };

    let archive_result = archive.wait();
    let extraction_result = extraction.wait();
    let archive_status = archive_result.map_err(|source| XtaskError::BaselineCommandWait {
        command: "git archive",
        source,
    })?;
    let extraction_status =
        extraction_result.map_err(|source| XtaskError::BaselineCommandWait {
            command: "tar",
            source,
        })?;
    if !archive_status.success() {
        return Err(XtaskError::BaselineCommandFailed {
            command: "git archive",
            status: archive_status,
        });
    }
    if !extraction_status.success() {
        return Err(XtaskError::BaselineCommandFailed {
            command: "tar",
            status: extraction_status,
        });
    }
    Ok(())
}

fn serialize_plan(plan: &SemverPlan) -> Result<Vec<u8>, XtaskError> {
    let mut output = serde_json::to_vec(plan)?;
    output.push(b'\n');
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(XtaskError::OutputTooLarge {
            size: output.len(),
            maximum: MAX_OUTPUT_BYTES,
        });
    }
    Ok(output)
}
