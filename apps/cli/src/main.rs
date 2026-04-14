use std::process::ExitCode;

use clap::Parser;

mod cli;
mod commands;
mod config;
mod output;
mod plugins;
mod suggestions;
#[cfg(feature = "tui")]
mod tui;

use cli::{
    ActionsCommand, Cli, Command, ConfigCommand, DevActionCommand, DevCommand, PluginCommand,
};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(run(cli));

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        },
    }
}

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let cfg = config::CliConfig::load().await;
    let _guard = init_logging(cli.verbose, &cfg);
    dispatch(cli).await
}

fn init_logging(verbose: bool, cfg: &config::CliConfig) -> Option<nebula_log::LoggerGuard> {
    if std::env::var("RUST_LOG").is_ok() || std::env::var("NEBULA_LOG").is_ok() {
        return nebula_log::auto_init().ok();
    }

    let level = if verbose {
        "debug".to_owned()
    } else {
        cfg.log.level.clone()
    };

    let config = nebula_log::Config {
        level,
        writer: nebula_log::WriterConfig::Stderr,
        ..Default::default()
    };
    nebula_log::init_with(config).ok()
}

async fn dispatch(cli: Cli) -> anyhow::Result<ExitCode> {
    let quiet = cli.quiet;

    match cli.command {
        Command::Run(args) => commands::run::execute(args, quiet).await,
        Command::Validate(args) => commands::validate::execute(args, quiet),
        Command::Replay(args) => commands::replay::execute(args, quiet).await,
        Command::Watch(args) => commands::watch::execute(args).await,
        Command::Actions { command } => {
            match command {
                ActionsCommand::List(args) => commands::actions::list(args),
                ActionsCommand::Info(args) => commands::actions::info(args),
                ActionsCommand::Test(args) => commands::actions::test(args).await,
            }
            Ok(ExitCode::SUCCESS)
        },
        Command::Plugin { command } => {
            match command {
                PluginCommand::List => commands::plugin::list().await,
                PluginCommand::New(args) => commands::plugin_new::execute(args)?,
            }
            Ok(ExitCode::SUCCESS)
        },
        Command::Dev { command } => {
            match command {
                DevCommand::Init(args) => commands::dev::init::execute(args)?,
                DevCommand::Action { command } => match command {
                    DevActionCommand::New(args) => commands::dev::action::execute(args)?,
                },
            }
            Ok(ExitCode::SUCCESS)
        },
        Command::Config { command } => {
            match command {
                ConfigCommand::Show => commands::config::show().await,
                ConfigCommand::Init(args) => commands::config::init(args.global)?,
            }
            Ok(ExitCode::SUCCESS)
        },
        Command::Completion(args) => {
            commands::completion::execute(args);
            Ok(ExitCode::SUCCESS)
        },
    }
}
