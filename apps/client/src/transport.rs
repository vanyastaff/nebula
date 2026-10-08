//! HTTP adapter shared by the two hosts. No redirects or automatic mutation replay.

use nebula_api_contract::v1::{
    auth::{LoginRequest, LoginResponse},
    execution::{
        ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse, StartExecutionRequest,
    },
    health::VersionInfo,
    me::MeResponse,
    problem::ProblemDetails,
    workflow::{
        ListWorkflowsResponse, UpdateWorkflowDocumentRequest, WorkflowDocumentResponse,
        WorkflowResponse,
    },
};
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, sync::Arc};
use url::Url;
use zeroize::Zeroizing;

const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    #[error("Use an HTTPS server address, or HTTP on loopback, without credentials or a query.")]
    Configuration,
    #[error(
        "The server could not be reached. Check the address, connection and browser CORS policy."
    )]
    ReadFailed,
    #[error("The write may have reached the server. Read its state before trying again.")]
    OutcomeUnknown,
    #[error("The server version changed. Read the current version and review your draft.")]
    Conflict,
    #[error("Sign in again; this session is no longer authorized.")]
    Unauthorized,
    #[error("Your account does not have permission for this operation.")]
    Forbidden,
    #[error("This server does not support the required client contract.")]
    Unsupported,
    #[error("The server response does not match the expected contract.")]
    InvalidResponse,
    #[error("Enter your authenticator code and sign in again.")]
    MfaRequired,
    #[error(
        "Browser password login requires the same origin as this page. Use a PAT for an operator-approved remote origin."
    )]
    #[cfg(target_arch = "wasm32")]
    BrowserOrigin,
    #[error("The server rejected the request (HTTP {0}).")]
    Rejected(u16),
}

pub(crate) enum SignIn {
    Password(LoginRequest),
    Token(Zeroizing<String>),
}

struct Authority {
    bearer: Option<Zeroizing<String>>,
    csrf: Option<Zeroizing<String>>,
    #[cfg(not(target_arch = "wasm32"))]
    cookie: Option<Zeroizing<String>>,
}

#[derive(Clone)]
pub(crate) struct Connection {
    endpoint: Url,
    authority: Arc<Authority>,
    #[cfg(not(target_arch = "wasm32"))]
    client: reqwest::Client,
}

pub(crate) struct SignedIn {
    pub(crate) connection: Connection,
    pub(crate) profile: MeResponse,
}

struct WireResponse {
    status: u16,
    content_type: String,
    body: Zeroizing<Vec<u8>>,
    #[cfg(not(target_arch = "wasm32"))]
    cookies: Vec<Zeroizing<String>>,
}

impl Connection {
    pub(crate) fn new(endpoint: &str) -> Result<Self, Failure> {
        let mut endpoint = Url::parse(endpoint.trim()).map_err(|_| Failure::Configuration)?;
        let loopback = match endpoint.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(host)) => host == "localhost",
            None => false,
        };
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback))
            || endpoint.host().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Failure::Configuration);
        }
        let path = format!("{}/", endpoint.path().trim_end_matches('/'));
        endpoint.set_path(&path);
        #[cfg(not(target_arch = "wasm32"))]
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| Failure::Configuration)?;
        Ok(Self {
            endpoint,
            authority: Arc::new(Authority {
                bearer: None,
                csrf: None,
                #[cfg(not(target_arch = "wasm32"))]
                cookie: None,
            }),
            #[cfg(not(target_arch = "wasm32"))]
            client,
        })
    }

    pub(crate) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    fn url(&self, segments: &[&str]) -> Result<Url, Failure> {
        let mut url = self.endpoint.clone();
        for segment in segments {
            if segment.is_empty()
                || matches!(*segment, "." | "..")
                || segment.chars().any(char::is_control)
            {
                return Err(Failure::Configuration);
            }
        }
        url.path_segments_mut()
            .map_err(|()| Failure::Configuration)?
            .pop_if_empty()
            .extend(["api", "v1"])
            .extend(segments.iter().copied());
        Ok(url)
    }

    pub(crate) async fn sign_in(mut self, intent: SignIn) -> Result<SignedIn, Failure> {
        let version: VersionInfo = self
            .read(
                self.endpoint
                    .join("version")
                    .map_err(|_| Failure::Configuration)?,
            )
            .await?;
        if version.name != "nebula" {
            return Err(Failure::Unsupported);
        }
        match intent {
            SignIn::Token(token) => {
                if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
                    return Err(Failure::Configuration);
                }
                self.authority = Arc::new(Authority {
                    bearer: Some(token),
                    csrf: None,
                    #[cfg(not(target_arch = "wasm32"))]
                    cookie: None,
                });
            },
            SignIn::Password(request) => {
                #[cfg(target_arch = "wasm32")]
                {
                    let origin = web_sys::window()
                        .ok_or(Failure::BrowserOrigin)?
                        .location()
                        .origin()
                        .map_err(|_| Failure::BrowserOrigin)?;
                    if self.endpoint.origin().ascii_serialization() != origin {
                        return Err(Failure::BrowserOrigin);
                    }
                }
                let body = Zeroizing::new(
                    serde_json::to_vec(&request).map_err(|_| Failure::Configuration)?,
                );
                let response = self
                    .exchange("POST", self.url(&["auth", "login"])?, Some(&body), None)
                    .await?;
                if response.status == 202 {
                    return Err(Failure::MfaRequired);
                }
                let login: LoginResponse = decode(&response, false, &[200])?;
                #[cfg(not(target_arch = "wasm32"))]
                let cookie = session_cookie(&response.cookies)?;
                self.authority = Arc::new(Authority {
                    bearer: None,
                    csrf: Some(Zeroizing::new(login.csrf_token.clone())),
                    #[cfg(not(target_arch = "wasm32"))]
                    cookie: Some(cookie),
                });
            },
        }
        let profile = self.read(self.url(&["me"])?).await?;
        Ok(SignedIn {
            connection: self,
            profile,
        })
    }

    pub(crate) async fn list(
        &self,
        org: &str,
        workspace: &str,
        page: usize,
    ) -> Result<ListWorkflowsResponse, Failure> {
        let mut url = self.url(&["orgs", org, "workspaces", workspace, "workflows"])?;
        url.query_pairs_mut()
            .append_pair("page", &page.to_string())
            .append_pair("page_size", "25");
        self.read(url).await
    }
    pub(crate) async fn load(
        &self,
        org: &str,
        workspace: &str,
        workflow: &str,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        let document: WorkflowDocumentResponse = self
            .read(self.url(&["orgs", org, "workspaces", workspace, "workflows", workflow])?)
            .await
            .map_err(|failure| {
                if failure == Failure::InvalidResponse {
                    Failure::Unsupported
                } else {
                    failure
                }
            })?;
        if document.workflow.id != workflow || document.revision == 0 {
            return Err(Failure::InvalidResponse);
        }
        Ok(document)
    }
    pub(crate) async fn save(
        &self,
        org: &str,
        workspace: &str,
        workflow: &str,
        request: &UpdateWorkflowDocumentRequest,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        let document: WorkflowDocumentResponse = self
            .write(
                "PUT",
                self.url(&["orgs", org, "workspaces", workspace, "workflows", workflow])?,
                request,
                None,
                &[200],
            )
            .await?;
        if document.workflow.id != workflow
            || request
                .expected_revision
                .and_then(|value| value.checked_add(1))
                != Some(document.revision)
            || request
                .update
                .definition
                .as_ref()
                .is_some_and(|patch| patch["nodes"] != document.definition["nodes"])
        {
            return Err(Failure::OutcomeUnknown);
        }
        Ok(document)
    }
    pub(crate) async fn publish(
        &self,
        org: &str,
        workspace: &str,
        workflow: &str,
        revision: u64,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        let mut url = self.url(&[
            "orgs",
            org,
            "workspaces",
            workspace,
            "workflows",
            workflow,
            "activate",
        ])?;
        url.query_pairs_mut()
            .append_pair("expected_revision", &revision.to_string());
        let _: WorkflowResponse = self
            .write("POST", url, &serde_json::json!({}), None, &[200])
            .await?;
        // Activation changes the storage revision; obtain it before another edit/save.
        self.load(org, workspace, workflow)
            .await
            .map_err(|_| Failure::OutcomeUnknown)
    }
    pub(crate) async fn run(
        &self,
        org: &str,
        workspace: &str,
        workflow: &str,
        key: &str,
    ) -> Result<ExecutionResponse, Failure> {
        self.write(
            "POST",
            self.url(&[
                "orgs",
                org,
                "workspaces",
                workspace,
                "workflows",
                workflow,
                "executions",
            ])?,
            &StartExecutionRequest { input: None },
            Some(key),
            &[202],
        )
        .await
    }
    pub(crate) async fn status(
        &self,
        org: &str,
        workspace: &str,
        execution: &str,
    ) -> Result<ExecutionDetailResponse, Failure> {
        let detail: ExecutionDetailResponse = self
            .read(self.url(&[
                "orgs",
                org,
                "workspaces",
                workspace,
                "executions",
                execution,
            ])?)
            .await?;
        if detail.execution.id != execution {
            return Err(Failure::InvalidResponse);
        }
        Ok(detail)
    }
    pub(crate) async fn history(
        &self,
        org: &str,
        workspace: &str,
        workflow: &str,
    ) -> Result<ListExecutionsResponse, Failure> {
        self.read(self.url(&[
            "orgs",
            org,
            "workspaces",
            workspace,
            "workflows",
            workflow,
            "executions",
        ])?)
        .await
    }

    async fn read<T: DeserializeOwned>(&self, url: Url) -> Result<T, Failure> {
        decode(&self.exchange("GET", url, None, None).await?, false, &[200])
    }
    async fn write<T: DeserializeOwned>(
        &self,
        method: &str,
        url: Url,
        body: &impl Serialize,
        key: Option<&str>,
        statuses: &[u16],
    ) -> Result<T, Failure> {
        let body = Zeroizing::new(serde_json::to_vec(body).map_err(|_| Failure::Configuration)?);
        decode(
            &self.exchange(method, url, Some(&body), key).await?,
            true,
            statuses,
        )
    }

    #[tracing::instrument(name = "client.http", skip_all)]
    async fn exchange(
        &self,
        method: &str,
        url: Url,
        body: Option<&[u8]>,
        key: Option<&str>,
    ) -> Result<WireResponse, Failure> {
        let mutation = method != "GET";
        let failure = || {
            if mutation {
                Failure::OutcomeUnknown
            } else {
                Failure::ReadFailed
            }
        };
        let mut headers = BTreeMap::new();
        headers.insert("accept".to_owned(), "application/json".to_owned());
        if body.is_some() {
            headers.insert("content-type".into(), "application/json".into());
        }
        if let Some(token) = &self.authority.bearer {
            headers.insert("authorization".into(), format!("Bearer {}", token.as_str()));
        }
        if let Some(csrf) = &self.authority.csrf {
            headers.insert("x-csrf-token".into(), csrf.to_string());
        }
        if let Some(key) = key {
            headers.insert("idempotency-key".into(), key.to_owned());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(cookie) = &self.authority.cookie {
                headers.insert("cookie".into(), cookie.to_string());
            }
            let mut request = self.client.request(
                reqwest::Method::from_bytes(method.as_bytes())
                    .map_err(|_| Failure::Configuration)?,
                url,
            );
            for (name, value) in &headers {
                let mut header = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| Failure::Configuration)?;
                header.set_sensitive(true);
                request = request.header(name.as_str(), header);
            }
            if let Some(body) = body {
                request = request.body(body.to_vec());
            }
            let mut response = request.send().await.map_err(|_| failure())?;
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|header| header.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let cookies = response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|header| header.to_str().ok())
                .map(|header| Zeroizing::new(header.to_owned()))
                .collect();
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
                if chunk.len() > MAX_BODY.saturating_sub(bytes.len()) {
                    return Err(failure());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(WireResponse {
                status,
                content_type,
                body: bytes,
                cookies,
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let headers =
                serde_wasm_bindgen::to_value(&headers).map_err(|_| Failure::Configuration)?;
            let body = body.map(|body| String::from_utf8_lossy(body).into_owned());
            let value = browser_fetch(
                url.as_str(),
                method,
                headers,
                body,
                self.authority.bearer.is_none(),
                MAX_BODY as u32,
            )
            .await
            .map_err(|_| failure())?;
            let response: BrowserResponse =
                serde_wasm_bindgen::from_value(value).map_err(|_| failure())?;
            Ok(WireResponse {
                status: response.status,
                content_type: response.content_type,
                body: Zeroizing::new(response.body.into_bytes()),
            })
        }
    }
}

fn decode<T: DeserializeOwned>(
    response: &WireResponse,
    mutation: bool,
    statuses: &[u16],
) -> Result<T, Failure> {
    let invalid = || {
        if mutation {
            Failure::OutcomeUnknown
        } else {
            Failure::InvalidResponse
        }
    };
    let mime = response
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if statuses.contains(&response.status) && mime == "application/json" {
        return serde_json::from_slice(&response.body).map_err(|_| invalid());
    }
    if mime != "application/problem+json" {
        return Err(invalid());
    }
    let problem: ProblemDetails = serde_json::from_slice(&response.body).map_err(|_| invalid())?;
    if problem.status != response.status {
        return Err(invalid());
    }
    Err(match response.status {
        401 => Failure::Unauthorized,
        403 => Failure::Forbidden,
        409 => Failure::Conflict,
        501 => Failure::Unsupported,
        500..=599 if mutation => Failure::OutcomeUnknown,
        status => Failure::Rejected(status),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn session_cookie(cookies: &[Zeroizing<String>]) -> Result<Zeroizing<String>, Failure> {
    let mut values = BTreeMap::new();
    for cookie in cookies {
        let pair = cookie.split(';').next().ok_or(Failure::InvalidResponse)?;
        let (name, value) = pair.split_once('=').ok_or(Failure::InvalidResponse)?;
        if matches!(name, "__Host-nebula-session" | "__Host-nebula-csrf")
            && (value.is_empty()
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b';')
                || values.insert(name, value).is_some())
        {
            return Err(Failure::InvalidResponse);
        }
    }
    if values.len() != 2 {
        return Err(Failure::InvalidResponse);
    }
    Ok(Zeroizing::new(
        values
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct BrowserResponse {
    status: u16,
    content_type: String,
    body: String,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export async function browser_fetch(url, method, headers, body, session, maxBody) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 30000);
    try {
        const response = await fetch(url, {method, headers: Object.fromEntries(headers), body,
            redirect: 'error', cache: 'no-store', credentials: session ? 'same-origin' : 'omit', signal: controller.signal});
        const reader = response.body.getReader();
        const chunks = []; let size = 0;
        for (;;) {
            const {done, value} = await reader.read(); if (done) break;
            size += value.length;
            if (size > maxBody) { await reader.cancel(); throw new Error('Response too large'); }
            chunks.push(value);
        }
        const bytes = new Uint8Array(size); let offset = 0;
        for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
        return {status: response.status, content_type: response.headers.get('content-type') || '', body: new TextDecoder('utf-8', {fatal:true}).decode(bytes)};
    } finally { clearTimeout(timer); }
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn browser_fetch(
        url: &str,
        method: &str,
        headers: wasm_bindgen::JsValue,
        body: Option<String>,
        session: bool,
        max_body: u32,
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    /// Uses only an isolated, operator-enrolled test deployment; never a production account.
    #[tokio::test]
    #[ignore = "requires isolated SQLite server; see apps/client/README.md"]
    async fn live_existing_server_edit_conflict_publish_run_and_persisted_output() {
        use crate::document::Draft;
        use nebula_api_contract::v1::{
            auth::SecretString,
            execution::{ExecutionNodeOutput, ExecutionStatus},
            workflow::CreateWorkflowRequest,
        };
        let endpoint =
            std::env::var("NEBULA_CLIENT_TEST_ENDPOINT").expect("isolated test endpoint required");
        let signed_in = Connection::new(&endpoint)
            .unwrap()
            .sign_in(SignIn::Password(LoginRequest {
                email: "client-acceptance@example.test".into(),
                password: SecretString::new("isolated-client-acceptance-password".into()),
                totp: None,
            }))
            .await
            .unwrap();
        let connection = signed_in.connection;
        let name = format!("Client acceptance {}", uuid::Uuid::new_v4());
        let created: WorkflowResponse = connection.write("POST", connection.url(&["orgs", "personal", "workspaces", "default", "workflows"]).unwrap(),
            &CreateWorkflowRequest { name:name.clone(), description:None, definition:serde_json::json!({
                "nodes":[{"id":"transform","name":"Transform","plugin_key":"core","action_key":"json_transform","parameters":{
                    "data":{"type":"literal","value":{"value":1}},
                    "operations":{"type":"literal","value":[]}
                }}],"connections":[]
            }) }, None, &[201]).await.unwrap();
        let listed = connection.list("personal", "default", 1).await.unwrap();
        assert!(
            listed
                .workflows
                .iter()
                .any(|workflow| workflow.id == created.id)
        );
        let loaded = connection
            .load("personal", "default", &created.id)
            .await
            .unwrap();
        let mut draft = Draft::new(loaded).unwrap();
        draft.edit("transform", "data", r#"{"value":2}"#).unwrap();
        let mut competing = draft.save_request();
        competing.update.definition.as_mut().unwrap()["nodes"][0]["parameters"]["data"]["value"] =
            serde_json::json!({"value":3});
        connection
            .save("personal", "default", &created.id, &competing)
            .await
            .unwrap();
        assert_eq!(
            connection
                .save("personal", "default", &created.id, &draft.save_request())
                .await
                .unwrap_err(),
            Failure::Conflict
        );
        assert!(draft.dirty());
        draft.remote = Some(
            connection
                .load("personal", "default", &created.id)
                .await
                .unwrap(),
        );
        draft.reapply().unwrap();
        let saved = connection
            .save("personal", "default", &created.id, &draft.save_request())
            .await
            .unwrap();
        assert_eq!(saved.revision, 3);
        let published = connection
            .publish("personal", "default", &created.id, saved.revision)
            .await
            .unwrap();
        assert_eq!(
            published.definition["nodes"][0]["parameters"]["data"]["value"],
            serde_json::json!({"value":2})
        );
        let key = uuid::Uuid::new_v4().to_string();
        let receipt = connection
            .run("personal", "default", &created.id, &key)
            .await
            .unwrap();
        let replay = connection
            .run("personal", "default", &created.id, &key)
            .await
            .unwrap();
        assert_eq!(receipt.id, replay.id);
        let status = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let status = connection
                    .status("personal", "default", &receipt.id)
                    .await
                    .unwrap();
                if matches!(
                    status.execution.status,
                    ExecutionStatus::Completed | ExecutionStatus::Failed
                ) {
                    break status;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("worker must reach persisted terminal status");
        assert_eq!(status.execution.status, ExecutionStatus::Completed);
        assert!(
            matches!(&status.nodes["transform"].output, Some(ExecutionNodeOutput::Inline { value }) if value == &serde_json::json!({"value":2}))
        );
        let history = connection
            .history("personal", "default", &created.id)
            .await
            .unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].id, receipt.id);
    }
    #[test]
    fn endpoints_cannot_smuggle_authority_or_use_cleartext_remote_transport() {
        for url in [
            "http://remote.test",
            "https://user:secret@example.test",
            "https://example.test/?token=secret",
            "https://example.test/#secret",
        ] {
            assert!(Connection::new(url).is_err());
        }
        let connection = Connection::new("http://127.0.0.1:8000/prefix").unwrap();
        assert_eq!(
            connection
                .url(&["orgs", "a/b", "workspaces", "ws"])
                .unwrap()
                .as_str(),
            "http://127.0.0.1:8000/prefix/api/v1/orgs/a%2Fb/workspaces/ws"
        );
    }
    #[tokio::test]
    async fn token_sign_in_probes_root_version_then_authenticates_the_profile() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/version"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"name":"nebula","version":"0.33.0"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .and(header("authorization", "Bearer isolated-fixture-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "user_id":"user_fixture", "email":"fixture@example.test",
                "display_name":"Fixture", "email_verified":true,
                "mfa_enabled":false, "tokens_count":1
            })))
            .expect(1)
            .mount(&server)
            .await;
        let signed_in = Connection::new(&server.uri())
            .unwrap()
            .sign_in(SignIn::Token(Zeroizing::new(
                "isolated-fixture-token".into(),
            )))
            .await
            .unwrap();
        assert_eq!(signed_in.profile.user_id, "user_fixture");
    }

    #[tokio::test]
    async fn mutation_redirect_never_reaches_a_second_destination_and_is_uncertain() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/orgs/org/workspaces/ws/workflows/wf/executions",
            ))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/redirected", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/redirected"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let result = Connection::new(&server.uri())
            .unwrap()
            .run("org", "ws", "wf", "one-intent")
            .await;
        assert_eq!(result.unwrap_err(), Failure::OutcomeUnknown);
    }
    #[tokio::test]
    async fn uncertain_start_reuses_the_same_key_only_on_explicit_reconciliation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("idempotency-key", "same-intent"))
            .respond_with(ResponseTemplate::new(202).set_body_string("truncated receipt"))
            .expect(1)
            .mount(&server)
            .await;
        let connection = Connection::new(&server.uri()).unwrap();
        assert_eq!(
            connection
                .run("org", "ws", "wf", "same-intent")
                .await
                .unwrap_err(),
            Failure::OutcomeUnknown
        );
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST")).and(header("idempotency-key", "same-intent"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"id":"exe_one","workflow_id":"wf","status":"Created","started_at":0}))).expect(1).mount(&server).await;
        assert_eq!(
            connection
                .run("org", "ws", "wf", "same-intent")
                .await
                .unwrap()
                .id,
            "exe_one"
        );
    }
    #[tokio::test]
    async fn old_servers_without_document_revision_are_explicitly_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id":"wf","name":"Old server","created_at":0,"updated_at":0}),
            ))
            .mount(&server)
            .await;
        assert_eq!(
            Connection::new(&server.uri())
                .unwrap()
                .load("org", "ws", "wf")
                .await
                .unwrap_err(),
            Failure::Unsupported
        );
    }
}
