//! An SDK-only HTTP API integration: a derived bearer-token resource whose
//! instance is the HTTP transport, and the action-side code that sends a
//! buffered request and opens a streamed one.

use nebula_sdk::integration::credential::BearerTokenCredential;
use nebula_sdk::integration::resource::{
    CredentialSlot, Effect, Error, Lease, Operation, OperationError, Provider, Resident,
    ResidentProvider, Resource, ResourceContext, ResourceKey, ResourceMetadataDraft,
    http::{
        AsWrite, Authorize, Delete, Get, HttpApi, HttpConfig, HttpTransport, Keyed, Patch, Post,
        Put, Request, open_stream,
    },
    resource_key,
};
use nebula_sdk::prelude::Value;

#[derive(Resource)]
struct GitHub {
    #[credential(key = "token")]
    token: CredentialSlot<BearerTokenCredential>,
}

#[async_trait::async_trait]
impl Provider for GitHub {
    type Config = HttpConfig;
    type Instance = HttpTransport;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("example.github")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_sdk::prelude::metadata_name!("GitHub"),
            "",
        )
    }

    async fn create(
        &self,
        config: &HttpConfig,
        _: &ResourceContext,
    ) -> Result<HttpTransport, Error> {
        HttpTransport::new(config)
    }
}

impl ResidentProvider for GitHub {}

impl HttpApi for GitHub {
    fn authorize(slots: &Self::Pinned, auth: &mut Authorize<'_>) -> Result<(), OperationError> {
        auth.bearer(slots.token())
    }
}

/// What action code does with a managed lease.
async fn action_code(github: &Lease<GitHub>) -> Result<(Value, usize), OperationError> {
    let user: Value = github.submit(Request::get("/user")?).await?.json()?;
    let mut events = open_stream(github, Request::get("/events")?).await?;
    let mut streamed = 0;
    while let Some(chunk) = events.next().await {
        streamed += chunk?.len();
    }
    Ok((user, streamed))
}

fn effect<M>(_: &Request<M>) -> Effect
where
    M: nebula_sdk::integration::resource::http::Method,
    Request<M>: Operation<GitHub>,
{
    <Request<M> as Operation<GitHub>>::EFFECT
}

fn main() -> Result<(), OperationError> {
    let _action_code = action_code;
    assert_eq!(effect(&Request::get("/user")?), Effect::Read);
    assert_eq!(effect(&Request::put("/user")?), Effect::Idempotent);
    assert_eq!(effect(&Request::post("/issues")?), Effect::Write);
    let keyed: Request<Keyed<Post>> = Request::post("/issues")?.idempotency_key("issue-1");
    assert_eq!(effect(&keyed), Effect::Idempotent);
    let keyed: Request<Keyed<Patch>> = Request::patch("/issues/1")?.idempotency_key("edit-1");
    assert_eq!(effect(&keyed), Effect::Idempotent);
    let write: Request<AsWrite<Delete>> = Request::delete("/counter")?.as_write();
    assert_eq!(effect(&write), Effect::Write);
    let _: Request<Put> = Request::put("/user")?;
    let _: Request<Get> = Request::get("/user")?;

    let request = Request::get("/repos/acme/secret-repo")?
        .query(&[("token", "query-secret")])
        .header("x-trace", "trace-secret")?;
    let text = format!("{request:?}");
    assert!(text.contains("GET") && text.contains("x-trace"));
    assert!(!text.contains("secret"), "{text}");

    let config = HttpConfig::new("https://api.example.com/v3");
    assert!(!format!("{config:?}").contains("example.com"));
    assert!(
        Request::get("/user")?
            .header("Authorization", "Bearer x")
            .is_err(),
        "credentials come from slots only"
    );
    Ok(())
}
