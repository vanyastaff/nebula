//! Thin binary entry point for the environment-driven Nebula server.

#![expect(
    clippy::print_stderr,
    reason = "binary edge: startup failures must reach stderr outside the tracing lifetime"
)]

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match nebula_server::run_from_env().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = std::error::Error::source(cause);
            }
            std::process::ExitCode::FAILURE
        },
    }
}
