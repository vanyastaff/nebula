//! First-party authorization-code acquisition across file-SQLite reopenings.
//!
//! This exercises composition and controller commands, not the HTTP gateway or
//! a process crash. The TLS fixture retains the production transport policy.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nebula_core::UserId;
use nebula_credential::{
    Acquisition, AuthorizationDecision, CredentialActor, CredentialAuthenticationBinding,
    CredentialAuthorizationError, CredentialCommand, CredentialCommandResult, CredentialController,
    CredentialControllerError, CredentialOperation, CredentialServiceError,
    CredentialTenantAuthority, InteractionRequest, OAuth2Credential, TenantScope, UserInput,
};
use nebula_crypto::EncryptedData;
use nebula_metrics::MetricsRegistry;
use nebula_storage::credential::{EnvKeyProvider, KeyProvider, SqliteCredentialPersistence};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialSelector, Scope, StoredCredential,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use super::{
    CredentialRuntime, compose_runtime_with_transport, refresh_runtime_ports, sqlite_pending_store,
};
use crate::credential_adapters::transport_security_tests::TlsFixture;

const CLIENT_SECRET: &str = "restart-client-secret-canary";
const CALLBACK_CODE: &str = "restart-callback-code-canary";
const REDIRECT_URI: &str = "https://app.example.test/oauth/callback";

#[derive(Debug)]
struct FixtureAuthority {
    actor: CredentialActor,
    scope: Scope,
}

#[async_trait::async_trait]
impl CredentialTenantAuthority for FixtureAuthority {
    async fn decide(
        &self,
        actor: &CredentialActor,
        scope: &Scope,
        _operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
        Ok(
            if actor == &self.actor
                && scope.org_id == self.scope.org_id
                && scope.workspace_id == self.scope.workspace_id
            {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Deny
            },
        )
    }
}

fn controller(
    runtime: &CredentialRuntime,
    actor: &CredentialActor,
    scope: &Scope,
) -> CredentialController {
    CredentialController::new(
        runtime.service(),
        Arc::new(FixtureAuthority {
            actor: actor.clone(),
            scope: scope.clone(),
        }),
        Arc::clone(&runtime.adjudicator),
        Some(Arc::clone(&runtime.audit_sink)),
    )
}

fn authentication_binding(byte: char) -> CredentialAuthenticationBinding {
    CredentialAuthenticationBinding::parse(byte.to_string().repeat(43))
        .expect("valid fixture authentication binding")
}

fn continuation(token: &str, state: &str, binding: char) -> CredentialCommand {
    CredentialCommand::ContinueResolve {
        credential_key: nebula_core::credential_key!("oauth2"),
        pending_token: token.to_owned(),
        user_input: UserInput::Callback {
            params: HashMap::from([
                ("code".to_owned(), CALLBACK_CODE.to_owned()),
                ("state".to_owned(), state.to_owned()),
            ]),
        },
        authentication_binding: authentication_binding(binding),
    }
}

async fn reopen(
    database: &str,
    provider: &TlsFixture,
) -> (CredentialRuntime, SqliteCredentialPersistence) {
    let store = SqliteCredentialPersistence::connect(database)
        .await
        .expect("admitted file SQLite credential store");
    let key: Arc<dyn KeyProvider> = Arc::new(
        EnvKeyProvider::from_base64(super::DEVELOPMENT_KEY_BASE64).expect("fixed fixture key"),
    );
    let pending = sqlite_pending_store(&store, Arc::clone(&key), Vec::new());
    let refresh_ports = refresh_runtime_ports(store.refresh_schedule(), store.refresh_claim_repo());
    let transport = Arc::new(provider.transport(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]));
    let runtime = compose_runtime_with_transport(
        store.clone(),
        refresh_ports,
        pending,
        key,
        Vec::new(),
        Arc::new(MetricsRegistry::new()),
        transport,
    )
    .expect("first-party runtime composes");
    (runtime, store)
}

#[tokio::test]
async fn oauth_acquisition_continues_once_after_restart_and_projects_after_reopen() {
    let provider = TlsFixture::success().await;
    let directory = tempfile::tempdir().expect("temporary database directory");
    let database = directory.path().join("credentials.db");
    let database = database.to_str().expect("UTF-8 fixture path");
    let actor = CredentialActor::user(UserId::new());
    let scope = Scope::new("restart-workspace", "restart-organization");
    let tenant = TenantScope::from_scope(&scope);
    let owner = CredentialOwner::from_scope(&scope);

    // A: use the real acquisition pipeline; never seed protocol pending state.
    let (mut first, first_store) = reopen(database, &provider).await;
    let first_controller = controller(&first, &actor, &scope);
    let result = first_controller
        .execute(
            &actor,
            &scope,
            CredentialCommand::Resolve {
                credential_key: nebula_core::credential_key!("oauth2"),
                properties: json!({"authorization_code": {
                    "client": {"client_id": "restart-client", "client_secret": CLIENT_SECRET},
                    "auth_url": "https://provider.example.test/authorize",
                    "token_url": provider.endpoint(),
                    "scopes": ["read"],
                    "redirect_uri": REDIRECT_URI,
                    "auth_style": "post_body"
                }}),
                authentication_binding: authentication_binding('A'),
            },
        )
        .await
        .expect("authorization-code acquisition starts");
    assert!(!format!("{result:?}").contains(CLIENT_SECRET));
    let CredentialCommandResult::Acquisition(Acquisition::Pending {
        token,
        interaction: InteractionRequest::Redirect { url },
    }) = result
    else {
        panic!("authorization-code resolve must yield a browser interaction");
    };
    let authorization_url = url::Url::parse(&url).expect("valid authorization URL");
    let query: HashMap<_, _> = authorization_url.query_pairs().into_owned().collect();
    let state = query.get("state").expect("OAuth state").clone();
    let challenge = query.get("code_challenge").expect("PKCE challenge").clone();
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(provider.request_count(), 0);
    assert!(first_store.list(&owner, None).await.unwrap().is_empty());
    drop(first_controller);
    first.shutdown().await;
    drop(first);
    drop(first_store);

    // B: persisted pending survives; the wrong session cannot consume it or POST.
    let (mut second, second_store) = reopen(database, &provider).await;
    let second_controller = controller(&second, &actor, &scope);
    let wrong_binding = second_controller
        .execute(&actor, &scope, continuation(&token, &state, 'B'))
        .await
        .expect_err("wrong authentication binding fails closed");
    assert!(matches!(
        wrong_binding,
        CredentialControllerError::Service(CredentialServiceError::ValidationFailed { .. })
    ));
    assert_eq!(provider.request_count(), 0);
    assert!(second_store.list(&owner, None).await.unwrap().is_empty());
    for secret in [CLIENT_SECRET, CALLBACK_CODE, token.as_str(), state.as_str()] {
        assert!(!format!("{wrong_binding:?}").contains(secret));
    }

    let completed = second_controller
        .execute(&actor, &scope, continuation(&token, &state, 'A'))
        .await
        .expect("original pending completes after the wrong binding is rejected");
    let completion_debug = format!("{completed:?}");
    let CredentialCommandResult::Acquisition(Acquisition::Complete { head }) = completed else {
        panic!("valid continuation must persist a completed credential");
    };
    let id = nebula_core::CredentialId::parse(&head.id).expect("persisted credential id");
    assert_eq!(provider.request_count(), 1);
    assert_eq!(second_store.list(&owner, None).await.unwrap(), vec![id]);

    // Assert the actual authorization-code request, including restart-preserved PKCE.
    let request = provider.last_request();
    let request = std::str::from_utf8(&request).expect("UTF-8 HTTP fixture request");
    assert!(request.starts_with("POST /token HTTP/1.1"));
    let (_, body) = request.split_once("\r\n\r\n").expect("HTTP body");
    let form: HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(form.get("code").map(String::as_str), Some(CALLBACK_CODE));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some(CLIENT_SECRET)
    );
    assert_eq!(
        form.get("redirect_uri").map(String::as_str),
        Some(REDIRECT_URI)
    );
    let verifier = form.get("code_verifier").expect("stored PKCE verifier");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        challenge
    );

    let selector = CredentialSelector::new(owner.clone(), id);
    let StoredCredential::Live(raw) = second_store.get(&selector).await.unwrap() else {
        panic!("completed credential must be live");
    };
    let _: EncryptedData = serde_json::from_slice(raw.data())
        .expect("raw persistence contains an authenticated encryption envelope");
    for secret in [CLIENT_SECRET, "new-access", CALLBACK_CODE] {
        assert!(
            !raw.data()
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
        assert!(!completion_debug.contains(secret));
    }
    drop(raw);
    let replay = second_controller
        .execute(&actor, &scope, continuation(&token, &state, 'A'))
        .await
        .expect_err("consumed pending token cannot replay");
    assert!(matches!(
        replay,
        CredentialControllerError::Service(CredentialServiceError::PendingExpired)
    ));
    assert_eq!(provider.request_count(), 1);
    assert_eq!(second_store.list(&owner, None).await.unwrap(), vec![id]);
    drop(second_controller);
    second.shutdown().await;
    drop(second);
    drop(second_store);

    // C: no old service or decrypted cache can satisfy this slot projection.
    let (mut third, third_store) = reopen(database, &provider).await;
    let service = third.service();
    let binding = service
        .validate_credential_binding(&tenant, &id.to_string())
        .await
        .expect("persisted OAuth credential binds after reopen");
    let guard = service
        .resolve_for_slot::<OAuth2Credential>(&tenant, &binding, CancellationToken::new())
        .await
        .expect("reopened encrypted material projects into OAuth scheme");
    assert_eq!(guard.access_token().expose_secret(), "new-access");
    assert_eq!(provider.request_count(), 1);
    assert_eq!(third_store.list(&owner, None).await.unwrap(), vec![id]);
    drop(guard);
    drop(service);
    third.shutdown().await;
}
