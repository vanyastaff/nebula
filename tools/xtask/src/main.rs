use std::{io::Write as _, process::ExitCode};

fn main() -> ExitCode {
    match nebula_xtask::execute(std::env::args_os()) {
        Ok(output) => {
            if let Err(error) = std::io::stdout().write_all(&output) {
                let _ = writeln!(std::io::stderr(), "nebula-xtask: {error}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        },
        Err(nebula_xtask::XtaskError::CommandLine(error)) => {
            let exit_code = error.exit_code();
            if let Err(print_error) = error.print() {
                let _ = writeln!(std::io::stderr(), "nebula-xtask: {print_error}");
                return ExitCode::FAILURE;
            }
            u8::try_from(exit_code).map_or(ExitCode::FAILURE, ExitCode::from)
        },
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "nebula-xtask: {error}");
            ExitCode::FAILURE
        },
    }
}
