mod changes;
mod model;
mod north_star;
mod pre_commit;
mod runtime_repair_red;
mod workspace;

pub use pre_commit::PlanError as PreCommitPlanError;

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
    /// Plan owner-scoped pre-commit checks without changing CI selection.
    PreCommitPlan {
        /// Workspace-relative staged paths, passed after `--`.
        paths: Vec<PathBuf>,
    },
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
    /// Verify bounded runtime-authority artifacts; fails closed until semantic policy is complete.
    VerifyRuntimeAuthority {
        /// Immutable directory containing observation artifacts.
        #[arg(long)]
        artifact_root: PathBuf,
        /// Trusted runner-supplied provenance, kept outside the artifact directory.
        #[arg(long)]
        expected_provenance: PathBuf,
        /// Exact 40-character revision this verifying job is running.
        #[arg(long)]
        source_revision: String,
        /// Repository identity of this verifying job.
        #[arg(long)]
        repository: String,
        /// Numeric workflow run identity of this verifying job.
        #[arg(long)]
        run_id: String,
        /// Positive workflow attempt of this verifying job.
        #[arg(long)]
        run_attempt: u32,
    },
    /// Build and verify provenance-bound runtime-authority artifacts from raw reports.
    BuildRuntimeAuthorityBundle {
        /// Directory containing raw behavior reports from the required jobs.
        #[arg(long)]
        observation_root: PathBuf,
        /// New directory that will receive the immutable verification artifacts.
        #[arg(long)]
        artifact_root: PathBuf,
        /// New trusted provenance manifest outside the artifact directory.
        #[arg(long)]
        expected_provenance: PathBuf,
        /// Exact 40-character source revision tested by the runner.
        #[arg(long)]
        source_revision: String,
        /// Repository identity supplied by the trusted runner.
        #[arg(long)]
        repository: String,
        /// Workflow path supplied by the trusted runner.
        #[arg(long)]
        workflow_path: String,
        /// Producer job identity supplied by the trusted runner.
        #[arg(long)]
        job_id: String,
        /// Numeric workflow run identity supplied by the trusted runner.
        #[arg(long)]
        run_id: String,
        /// Positive workflow attempt supplied by the trusted runner.
        #[arg(long)]
        run_attempt: u32,
        /// Exact compiler/toolchain description used to build the producers.
        #[arg(long)]
        toolchain: String,
    },
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
        TopLevelCommand::PreCommitPlan { paths } => pre_commit::plan(cwd, &paths),
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
        TopLevelCommand::NorthStarGates {
            command:
                NorthStarGatesCommand::VerifyRuntimeAuthority {
                    artifact_root,
                    expected_provenance,
                    source_revision,
                    repository,
                    run_id,
                    run_attempt,
                },
        } => north_star::verify_runtime_authority(
            &find_root(cwd)?,
            &artifact_root,
            &expected_provenance,
            &north_star::RunnerIdentity {
                source_revision,
                repository,
                run_id,
                run_attempt,
            },
        )
        .map_err(XtaskError::RuntimeAuthority),
        TopLevelCommand::NorthStarGates {
            command:
                NorthStarGatesCommand::BuildRuntimeAuthorityBundle {
                    observation_root,
                    artifact_root,
                    expected_provenance,
                    source_revision,
                    repository,
                    workflow_path,
                    job_id,
                    run_id,
                    run_attempt,
                    toolchain,
                },
        } => {
            north_star::build_runtime_authority_bundle(&north_star::RuntimeAuthorityBundleRequest {
                workspace_root: find_root(cwd)?,
                observation_root,
                artifact_root,
                expected_provenance,
                source_revision,
                repository,
                workflow_path,
                job_id,
                run_id,
                run_attempt,
                toolchain,
            })
            .map_err(XtaskError::RuntimeAuthority)
        },
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
    #[error(transparent)]
    PreCommit(#[from] PreCommitPlanError),
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
    #[error(transparent)]
    RuntimeAuthority(#[from] north_star::RuntimeAuthorityError),
    #[error("{0}")]
    RuntimeRepairRed(String),
}

impl From<runtime_repair_red::VerificationError> for XtaskError {
    fn from(error: runtime_repair_red::VerificationError) -> Self {
        Self::RuntimeRepairRed(error.to_string())
    }
}
