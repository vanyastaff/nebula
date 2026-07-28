mod changes;
mod model;
mod north_star;
mod runtime_repair_red;
mod workspace;

use std::{ffi::OsString, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use thiserror::Error;

use crate::{
    changes::Comparison,
    model::Plan,
    workspace::{Workspace, find_root},
};

#[derive(Debug, Parser)]
#[command(name = "nebula-xtask", version, about = "Nebula repository automation")]
struct Cli {
    #[command(subcommand)]
    command: TopLevelCommand,
}

#[derive(Debug, Subcommand)]
enum TopLevelCommand {
    /// Build a deterministic CI package plan.
    CiPlan {
        #[command(subcommand)]
        command: CiPlanCommand,
    },
    /// Validate versioned North Star post-selection gate policy.
    NorthStarGates {
        #[command(subcommand)]
        command: NorthStarGatesCommand,
    },
    /// Validate or reconcile the runtime-repair expected-failure profile.
    RuntimeRepairRed {
        #[command(subcommand)]
        command: RuntimeRepairRedCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CiPlanCommand {
    /// Select every Cargo workspace member.
    Full,
    /// Select changed packages and every reverse workspace dependent.
    Diff {
        /// Base Git revision. An empty or omitted value selects the full workspace.
        #[arg(long, default_value = "")]
        base: String,
        /// Head Git revision. An empty or omitted value selects the full workspace.
        #[arg(long, default_value = "")]
        head: String,
        /// Whether to compare from the merge base or directly between tips.
        #[arg(long, value_enum, default_value_t = ComparisonArg::MergeBase)]
        comparison: ComparisonArg,
    },
}

#[derive(Debug, Subcommand)]
enum NorthStarGatesCommand {
    /// Validate the registry, evidence schema, and required-CI bindings.
    Validate,
}

#[derive(Debug, Subcommand)]
enum RuntimeRepairRedCommand {
    /// Validate the versioned expected-case manifest without running RED tests.
    ValidateManifest,
    /// Verify raw nextest status and JUnit against the exact expected failures.
    Verify {
        /// Raw cargo-nextest exit code. Only 100 (TEST_RUN_FAILED) is accepted.
        #[arg(long)]
        nextest_exit_code: u8,
        /// JUnit report emitted by the runtime-repair-red nextest profile.
        #[arg(long)]
        junit: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ComparisonArg {
    MergeBase,
    Direct,
}

impl From<ComparisonArg> for Comparison {
    fn from(value: ComparisonArg) -> Self {
        match value {
            ComparisonArg::MergeBase => Self::MergeBase,
            ComparisonArg::Direct => Self::Direct,
        }
    }
}

/// Executes the xtask and returns its complete stdout payload.
///
/// The payload is constructed and validated in memory so failures never emit
/// a partial CI plan.
pub fn execute<I, T>(args: I) -> Result<Vec<u8>, XtaskError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    let cwd = std::env::current_dir().map_err(XtaskError::CurrentDirectory)?;
    execute_in(&cwd, cli)
}

fn execute_in(cwd: &std::path::Path, cli: Cli) -> Result<Vec<u8>, XtaskError> {
    match cli.command {
        TopLevelCommand::CiPlan { command } => {
            let workspace = Workspace::load(cwd)?;
            let plan = match command {
                CiPlanCommand::Full => Plan::full(&workspace, "full-request")?,
                CiPlanCommand::Diff {
                    base,
                    head,
                    comparison,
                } => {
                    if base.trim().is_empty() || head.trim().is_empty() {
                        Plan::full(&workspace, "missing-diff-sha")?
                    } else {
                        let changes = changes::git_diff(
                            workspace.root(),
                            base.trim(),
                            head.trim(),
                            comparison.into(),
                        )?;
                        Plan::from_changes(&workspace, changes)?
                    }
                },
            };
            plan.to_json_line()
        },
        TopLevelCommand::NorthStarGates {
            command: NorthStarGatesCommand::Validate,
        } => north_star::validate(&find_root(cwd)?)?
            .to_json_line()
            .map_err(XtaskError::Json),
        TopLevelCommand::RuntimeRepairRed { command } => {
            let workspace_root = find_root(cwd)?;
            match command {
                RuntimeRepairRedCommand::ValidateManifest => {
                    runtime_repair_red::validate_manifest(&workspace_root)?
                        .to_json_line()
                        .map_err(XtaskError::Json)
                },
                RuntimeRepairRedCommand::Verify {
                    nextest_exit_code,
                    junit,
                } => runtime_repair_red::verify(&workspace_root, nextest_exit_code, &junit)?
                    .to_json_line()
                    .map_err(XtaskError::Json),
            }
        },
    }
}

#[derive(Debug, Error)]
pub enum XtaskError {
    #[error("cannot determine current directory: {0}")]
    CurrentDirectory(std::io::Error),
    #[error("invalid command line: {0}")]
    CommandLine(#[from] clap::Error),
    #[error("cargo metadata failed: {0}")]
    Metadata(#[from] cargo_metadata::Error),
    #[error("cargo metadata did not contain a dependency resolve graph")]
    MissingResolve,
    #[error("workspace member `{0}` is absent from cargo metadata packages")]
    MissingWorkspacePackage(String),
    #[error("manifest `{manifest}` is outside workspace root `{root}`")]
    ManifestOutsideWorkspace { manifest: PathBuf, root: PathBuf },
    #[error("workspace has duplicate package name `{0}`")]
    DuplicatePackageName(String),
    #[error("package `{package}` has invalid metadata.nebula.ci policy: {detail}")]
    InvalidCiMetadata { package: String, detail: String },
    #[error("package `{package}` CI metadata names undeclared feature `{feature}`")]
    UnknownTestFeature { package: String, feature: String },
    #[error("failed to execute git diff: {0}")]
    GitIo(std::io::Error),
    #[error("git diff failed: {0}")]
    GitFailed(String),
    #[error("invalid git diff output: {0}")]
    InvalidGitOutput(String),
    #[error("CI plan contains {count} entries; maximum is {maximum}")]
    TooManyEntries { count: usize, maximum: usize },
    #[error("CI plan JSON is {size} bytes; conservative maximum is {maximum} bytes")]
    OutputTooLarge { size: usize, maximum: usize },
    #[error("CI plan JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot read workspace manifest `{path}`: {source}")]
    WorkspaceManifestRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("workspace manifest `{path}` is invalid TOML: {source}")]
    WorkspaceManifestParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("cannot find a Cargo workspace root above `{0}`")]
    WorkspaceRootNotFound(PathBuf),
    #[error(transparent)]
    NorthStarGates(#[from] north_star::ValidationError),
    #[error("{0}")]
    RuntimeRepairRed(String),
}

impl From<runtime_repair_red::VerificationError> for XtaskError {
    fn from(error: runtime_repair_red::VerificationError) -> Self {
        Self::RuntimeRepairRed(error.to_string())
    }
}
