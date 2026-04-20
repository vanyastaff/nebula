use clap::CommandFactory;
use clap_complete::generate;

use crate::cli::{Cli, CompletionArgs};

/// Execute the `completion` command.
pub(crate) fn execute(args: CompletionArgs) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_owned();
    generate(args.shell, &mut cmd, name, &mut std::io::stdout());
}
