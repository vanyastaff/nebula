//! Thin binary entry point for the environment-driven Nebula server.

#[tokio::main]
async fn main() -> Result<(), nebula_server::ServerRunError> {
    nebula_server::run_from_env().await
}
