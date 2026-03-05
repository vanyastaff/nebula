#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Reference Postgres adapter for `nebula-resource`.
//!
//! This crate demonstrates how to package a driver-specific resource adapter.
//! It intentionally keeps the runtime instance simple (`PostgresHandle`) so the
//! adapter can be used in CI and docs without requiring a running database.

use nebula_core::ResourceKey;
use nebula_resource::{Config, Context, Error, Resource, ResourceMetadata, Result};

/// Driver configuration for a Postgres resource.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PostgresConfig {
    /// Postgres connection string, e.g. `postgres://user:pass@host/db`.
    pub dsn: String,
    /// Logical connect timeout in milliseconds.
    pub connect_timeout_ms: u64,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            dsn: "postgres://postgres:postgres@localhost:5432/postgres".to_string(),
            connect_timeout_ms: 3_000,
        }
    }
}

impl Config for PostgresConfig {
    fn validate(&self) -> Result<()> {
        if self.dsn.trim().is_empty() {
            return Err(Error::configuration("dsn must not be empty"));
        }
        if !self.dsn.starts_with("postgres://") && !self.dsn.starts_with("postgresql://") {
            return Err(Error::configuration(
                "dsn must start with postgres:// or postgresql://",
            ));
        }
        if self.connect_timeout_ms == 0 {
            return Err(Error::configuration(
                "connect_timeout_ms must be greater than zero",
            ));
        }
        Ok(())
    }
}

/// Runtime handle produced by the reference adapter.
#[derive(Debug, Clone)]
pub struct PostgresHandle {
    /// Normalized DSN used for this handle.
    pub dsn: String,
}

/// Reference Postgres resource.
///
/// This adapter is intentionally lightweight: `create()` validates and returns
/// a typed handle. Real TCP/database connection management can be layered on
/// top while keeping the same public driver shape.
#[derive(Debug, Clone, Default)]
pub struct PostgresResource;

impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Instance = PostgresHandle;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceKey::try_from("postgres").expect("valid resource key"),
            "Postgres",
            "Reference Postgres adapter for nebula-resource",
        )
        .with_tag("driver:postgres")
        .with_tag("adapter:reference")
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        config.validate()?;
        Ok(PostgresHandle {
            dsn: config.dsn.clone(),
        })
    }

    async fn is_reusable(&self, instance: &Self::Instance) -> Result<bool> {
        Ok(!instance.dsn.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::{ExecutionId, Manager, PoolConfig, Scope, WorkflowId};

    fn ctx() -> Context {
        Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
    }

    #[test]
    fn config_validation_rejects_invalid_dsn() {
        let cfg = PostgresConfig {
            dsn: "http://not-postgres".to_string(),
            connect_timeout_ms: 1000,
        };
        assert!(cfg.validate().is_err());
    }

    #[tokio::test]
    async fn adapter_register_and_acquire_end_to_end() {
        let manager = Manager::new();
        manager
            .register(
                PostgresResource,
                PostgresConfig::default(),
                PoolConfig::default(),
            )
            .expect("register adapter resource");

        let key = ResourceKey::try_from("postgres").expect("valid key");
        let guard = manager.acquire(&key, &ctx()).await.expect("acquire");
        let handle = guard
            .as_any()
            .downcast_ref::<PostgresHandle>()
            .expect("downcast to PostgresHandle");
        assert!(handle.dsn.starts_with("postgres://"));
    }
}
