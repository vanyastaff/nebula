use axum::Router;

use super::{ServerTransport, TransportInitError};
use crate::{ApiConfig, AppState, build_app};

/// Full REST API transport.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiTransport;

impl ServerTransport for ApiTransport {
    fn name(&self) -> &'static str {
        "api"
    }

    fn bind_override_var(&self) -> Option<&'static str> {
        Some("SERVER_BIND_ADDRESS")
    }

    fn build_router(
        &self,
        state: AppState,
        api_config: &ApiConfig,
    ) -> Result<Router, TransportInitError> {
        Ok(build_app(state, api_config))
    }
}
