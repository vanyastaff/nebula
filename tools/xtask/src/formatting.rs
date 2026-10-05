//! Full local-source formatting, bounded to one Cargo package per invocation.

use std::{io::Write as _, path::Path, process::Command};

use cargo_metadata::{CargoOpt, MetadataCommand};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormattingError {
    #[error("cannot report formatter progress: {0}")]
    Report(std::io::Error),
    #[error("cannot start formatting package `{package}`: {source}")]
    Start {
        package: String,
        #[source]
        source: std::io::Error,
    },
    #[error("formatting package `{package}` failed with status {status}")]
    Failed {
        package: String,
        status: std::process::ExitStatus,
    },
}

pub(crate) fn check(cwd: &Path) -> Result<(), crate::XtaskError> {
    // Unlike CI selection, cargo fmt --all also visits local path dependencies
    // outside the workspace. Full metadata retains those packages; registry and
    // Git dependencies have a source and are excluded by cargo fmt as well.
    let metadata = MetadataCommand::new()
        .current_dir(cwd)
        .features(CargoOpt::AllFeatures)
        .other_options(vec!["--locked".to_owned()])
        .exec()?;
    let mut packages = metadata
        .packages
        .iter()
        .filter(|package| package.source.is_none())
        .collect::<Vec<_>>();
    packages.sort_by(|left, right| left.manifest_path.cmp(&right.manifest_path));
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    for package in packages {
        writeln!(std::io::stderr(), "fmt-check: {}", package.name)
            .map_err(FormattingError::Report)?;
        // Keep Cargo's target discovery and rustfmt configuration resolution.
        // The explicit package prevents a member manifest selecting workspace
        // default members, and bounds Windows' rustfmt command-line length.
        let status = Command::new(&cargo)
            .current_dir(&metadata.workspace_root)
            .args(["fmt", "--manifest-path"])
            .arg(&package.manifest_path)
            .args(["-p", package.name.as_ref(), "--", "--check"])
            .status()
            .map_err(|source| FormattingError::Start {
                package: package.name.to_string(),
                source,
            })?;
        if !status.success() {
            return Err(FormattingError::Failed {
                package: package.name.to_string(),
                status,
            }
            .into());
        }
    }
    Ok(())
}
