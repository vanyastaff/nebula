use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

/// Nebula workflow engine CLI.
///
/// Run, validate, and manage workflows from the terminal.
#[derive(Parser)]
#[command(name = "nebula", version, about, propagate_version = true)]
pub struct Cli {
    /// Enable verbose logging (debug level).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress all output except errors.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Execute a workflow from a YAML or JSON file.
    Run(RunArgs),

    /// Validate a workflow definition without executing it.
    Validate(ValidateArgs),

    /// Replay a previous execution from a specific node.
    Replay(ReplayArgs),

    /// Watch a workflow file and re-run on changes.
    Watch(WatchArgs),

    /// List and inspect available actions.
    Actions {
        #[command(subcommand)]
        command: ActionsCommand,
    },

    /// Manage plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },

    /// Developer tools: scaffolding, project setup.
    Dev {
        #[command(subcommand)]
        command: DevCommand,
    },

    /// Manage CLI configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Generate shell completions.
    Completion(CompletionArgs),
}

// ── Config ───────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show the resolved configuration.
    Show,
    /// Generate a default config file.
    Init(ConfigInitArgs),
}

#[derive(Parser)]
pub struct ConfigInitArgs {
    /// Create global config at ~/.nebula/config.toml instead of ./nebula.toml.
    #[arg(long)]
    pub global: bool,
}

// ── Actions ──────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ActionsCommand {
    /// List all registered actions.
    List(ActionsListArgs),
    /// Show detailed info about an action.
    Info(ActionsInfoArgs),
    /// Test a single action with sample input.
    Test(ActionsTestArgs),
}

#[derive(Parser)]
pub struct ActionsTestArgs {
    /// Action key to test (e.g. "echo", "http.get").
    pub key: String,

    /// Input data as JSON string.
    #[arg(short, long, default_value = "{}")]
    pub input: String,

    /// Output format.
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser)]
pub struct ActionsListArgs {
    /// Output format (auto-detects: text for terminal, json for pipes).
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser)]
pub struct ActionsInfoArgs {
    /// Action key (e.g. "echo", "delay").
    pub key: String,

    /// Output format.
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

// ── Plugin ────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum PluginCommand {
    /// List loaded plugins (built-in + external from plugins/ dir).
    List,
    /// Create a new plugin project.
    New(PluginNewArgs),
}

/// Arguments for the `plugin new` command.
#[derive(Parser)]
pub struct PluginNewArgs {
    /// Plugin name (e.g. "telegram", "slack", "csv-parser").
    pub name: String,

    /// Number of actions to scaffold.
    #[arg(long, default_value = "1")]
    pub actions: usize,

    /// Target directory (defaults to nebula-plugin-<name>).
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

// ── Dev ──────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum DevCommand {
    /// Initialize a new Nebula project in the current directory.
    Init(DevInitArgs),
    /// Scaffold a new action crate.
    Action {
        #[command(subcommand)]
        command: DevActionCommand,
    },
}

#[derive(Subcommand)]
pub enum DevActionCommand {
    /// Create a new action project.
    New(DevActionNewArgs),
}

/// Arguments for the `dev init` command.
#[derive(Parser)]
pub struct DevInitArgs {
    /// Project name (defaults to directory name).
    #[arg(short, long)]
    pub name: Option<String>,

    /// Target directory (defaults to current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

/// Arguments for the `dev action new` command.
#[derive(Parser)]
pub struct DevActionNewArgs {
    /// Action name (e.g. "http-request", "slack-send").
    pub name: String,

    /// Target directory (defaults to current directory).
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

// ── Completion ───────────────────────────────────────────────────────────

#[derive(Parser)]
pub struct CompletionArgs {
    /// Shell to generate completions for.
    pub shell: clap_complete::Shell,
}

// ── Run / Validate ───────────────────────────────────────────────────────

/// Arguments for the `run` command.
#[derive(Parser)]
pub struct RunArgs {
    /// Path to the workflow file (YAML or JSON).
    pub workflow: PathBuf,

    /// Input data as a JSON string.
    #[arg(short, long, default_value = "{}")]
    pub input: String,

    /// Read input data from a file (use "-" for stdin).
    #[arg(long, conflicts_with = "input")]
    pub input_file: Option<PathBuf>,

    /// Override node parameters (e.g. --set "fetch.params.url=https://staging.api.com").
    /// Format: <node_name>.params.<param_key>=<value>
    #[arg(long = "set", value_name = "NODE.PARAMS.KEY=VALUE")]
    pub overrides: Vec<String>,

    /// Maximum execution duration (e.g. "30s", "5m").
    #[arg(long, value_parser = parse_duration)]
    pub timeout: Option<Duration>,

    /// Maximum concurrent nodes.
    #[arg(long, default_value = "10")]
    pub concurrency: usize,

    /// Show execution plan without running (validate + resolve DAG).
    #[arg(long)]
    pub dry_run: bool,

    /// Stream node progress to stderr during execution.
    #[arg(long)]
    pub stream: bool,

    /// Launch interactive TUI dashboard for execution monitoring.
    #[cfg(feature = "tui")]
    #[arg(long)]
    pub tui: bool,

    /// Output format (auto-detects: text for terminal, json for pipes).
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

/// Arguments for the `watch` command.
#[derive(Parser)]
pub struct WatchArgs {
    /// Path to the workflow file (YAML or JSON).
    pub workflow: PathBuf,

    /// Input data as a JSON string.
    #[arg(short, long, default_value = "{}")]
    pub input: String,

    /// Override node parameters.
    #[arg(long = "set", value_name = "NODE.PARAMS.KEY=VALUE")]
    pub overrides: Vec<String>,

    /// Maximum concurrent nodes.
    #[arg(long, default_value = "10")]
    pub concurrency: usize,
}

/// Arguments for the `replay` command.
#[derive(Parser)]
pub struct ReplayArgs {
    /// Path to the workflow file (YAML or JSON).
    pub workflow: PathBuf,

    /// Node name to replay from (re-execute this node and all downstream).
    #[arg(long)]
    pub from: String,

    /// JSON file with stored outputs from previous execution (pinned nodes).
    #[arg(long)]
    pub outputs_file: Option<PathBuf>,

    /// Override input for the replay-from node.
    #[arg(short, long, default_value = "{}")]
    pub input: String,

    /// Output format.
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

/// Arguments for the `validate` command.
#[derive(Parser)]
pub struct ValidateArgs {
    /// Path to the workflow file (YAML or JSON).
    pub workflow: PathBuf,

    /// Output format (auto-detects: text for terminal, json for pipes).
    #[arg(long)]
    pub format: Option<OutputFormat>,
}

/// Supported output formats.
#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    /// JSON output (machine-readable).
    Json,
    /// Human-readable table/text output.
    Text,
}

/// Resolve output format: explicit flag > TTY detection.
/// Terminal → text, pipe → json.
pub fn resolve_format(explicit: Option<OutputFormat>) -> OutputFormat {
    explicit.unwrap_or_else(|| {
        if std::io::stdout().is_terminal() {
            OutputFormat::Text
        } else {
            OutputFormat::Json
        }
    })
}

/// Parse a human-friendly duration string like "30s", "5m", "1h".
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration cannot be empty".to_owned());
    }

    let (num_str, suffix) = match s.find(|c: char| c.is_alphabetic()) {
        Some(pos) => (&s[..pos], &s[pos..]),
        None => return Err(format!("missing unit suffix (s, m, h): {s}")),
    };

    let value: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {num_str}"))?;

    let secs = match suffix {
        "s" | "sec" | "secs" => value,
        "m" | "min" | "mins" => value * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => value * 3600,
        other => return Err(format!("unknown duration unit: {other}")),
    };

    Ok(Duration::from_secs(secs))
}
