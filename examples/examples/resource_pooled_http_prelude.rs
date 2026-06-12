//! Minimal pooled HTTP client using `nebula_resource::prelude`.
//!
//! Demonstrates the v4 author path: `Provider` + `Pooled` topology +
//! `RegistrationSpec` funnel + typed `acquire_pooled`.
//!
//! ```shell
//! cargo run -p nebula-examples --example resource_pooled_http_prelude
//! ```

use nebula_core::{ExecutionId, scope::Scope};
use nebula_resource::prelude::*;
use nebula_resource::topology::pooled::{BrokenCheck, PoolProvider};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
struct HttpConfig {
    base_url: String,
}

nebula_schema::impl_empty_has_schema!(HttpConfig);

impl ResourceConfig for HttpConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.base_url.is_empty() {
            return Err(Error::permanent("base_url must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.base_url.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone, Debug)]
struct HttpClient {
    base_url: String,
}

#[derive(Clone)]
struct HttpResource;

#[async_trait::async_trait]
impl Provider for HttpResource {
    type Config = HttpConfig;
    type Instance = HttpClient;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("http.client.prelude")
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _ctx: &ResourceContext,
    ) -> Result<HttpClient, Error> {
        Ok(HttpClient {
            base_url: config.base_url.clone(),
        })
    }
}

impl HasCredentialSlots for HttpResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl PoolProvider for HttpResource {
    fn is_broken(&self, _runtime: &HttpClient) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

fn test_ctx() -> ResourceContext {
    ResourceContext::minimal(
        Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    )
}

#[tokio::main]
async fn main() {
    let manager = Manager::new();
    let config = HttpConfig {
        base_url: "https://api.example.com".into(),
    };
    let fingerprint = config.fingerprint();

    manager
        .register(RegistrationSpec {
            resource: HttpResource,
            config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::<HttpResource>::new(PoolConfig::default(), fingerprint),
            recovery_gate: None,
        })
        .expect("register");

    let ctx = test_ctx();
    let guard = manager
        .acquire_pooled::<HttpResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire");

    println!("using client at {}", guard.base_url);

    drop(guard);
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("shutdown");
}
