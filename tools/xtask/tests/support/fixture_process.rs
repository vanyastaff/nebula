//! Keep hook-local repository selection out of disposable fixture processes.

use std::{ffi::OsStr, process::Command, sync::OnceLock};

pub(crate) fn fixture_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    for variable in local_git_env_vars() {
        command.env_remove(variable);
    }
    command
}

fn local_git_env_vars() -> &'static [String] {
    static VARIABLES: OnceLock<Vec<String>> = OnceLock::new();
    VARIABLES.get_or_init(|| {
        // Git documents this read-only query for commands aimed at a foreign
        // repository from a hook. Ask the installed Git once, rather than
        // maintaining a list that can drift between Git versions.
        let output = Command::new("git")
            .args(["rev-parse", "--local-env-vars"])
            .output()
            .expect("query Git's repository-local environment variable names");
        assert!(
            output.status.success(),
            "Git's local environment query failed"
        );
        String::from_utf8(output.stdout)
            .expect("Git environment variable names are UTF-8")
            .lines()
            .map(str::to_owned)
            .collect()
    })
}
