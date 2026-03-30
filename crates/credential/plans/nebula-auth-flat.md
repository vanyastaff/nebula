# nebula-auth — Flat Single Crate

---

## Module tree

```
nebula-auth/
├── Cargo.toml
└── src/
    ├── lib.rs
    │
    ├── types.rs        # taxonomy enums + AuthFlowDescriptor
    ├── principal.rs    # AuthenticatedPrincipal
    ├── error.rs        # AuthError
    │
    ├── flow.rs         # FlowStatus, AuthAction, FlowEvent, FlowState,
    │                   # Authenticator, InteractiveAuthenticator, DynInteractiveAuthenticator
    │
    ├── credential.rs   # ApiKey, Password, ClientSecret, TotpCode, MagicLinkToken, ...
    │                   # + CredentialVerifier trait
    │
    ├── token.rs        # TokenSet, StandardClaims, JwksSource trait,
    │                   # JwtVerifier, OpaqueToken, TokenIntrospector trait
    │
    ├── session.rs      # Session, SessionId, SessionStore trait
    ├── pending.rs      # PendingStateStore trait, DynPendingStore trait
    ├── policy.rs       # RequireScopes, Role, Permission, RoleResolver,
    │                   # PermissionChecker, Policy, PolicyDecision,
    │                   # PolicyContext, AllOf, AnyOf, Not
    │
    ├── oauth2.rs       # feature = "oauth2"
    │                   # OAuthEndpoints, AuthCodeConfig, ClientCredentialsConfig,
    │                   # PkceChallenge, AuthCodePendingState, DeviceFlowPendingState,
    │                   # AuthorizationCodeFlow, ClientCredentialsFlow, DeviceFlow
    │
    ├── oidc.rs         # feature = "oidc"
    │                   # OidcConfig, OidcMetadata, IdTokenClaims, UserInfo,
    │                   # OidcAuthFlow, discover()
    │
    ├── webauthn.rs     # feature = "webauthn"
    │                   # WebAuthnConfig, CredentialRecord,
    │                   # RegistrationPendingState, AuthenticationPendingState,
    │                   # RegistrationFlow, AuthenticationFlow
    │
    ├── axum.rs         # feature = "axum"
    │                   # BearerToken, VerifiedPrincipal, RequireAuthLayer,
    │                   # oauth_callback_router()
    │
    └── reqwest.rs      # feature = "reqwest"
                        # CachedTokenProvider, BearerAuthMiddleware
```

---

## Cargo.toml

```toml
[package]
name    = "nebula-auth"
version = "0.1.0"
edition = "2024"

[features]
default  = ["oauth2", "oidc"]
oauth2   = ["dep:reqwest", "dep:sha2", "dep:base64"]
oidc     = ["oauth2", "dep:jsonwebtoken"]
webauthn = ["dep:cbor4ii", "dep:p256", "dep:base64"]
axum     = ["dep:axum", "dep:tower"]
reqwest  = ["dep:reqwest"]
tonic    = ["dep:tonic"]

[dependencies]
serde         = { version = "1",    features = ["derive"] }
serde_json    = "1"
thiserror     = "2"
uuid          = { version = "1",    features = ["v7"] }
zeroize       = { version = "1",    features = ["derive"] }
tokio         = { version = "1",    features = ["sync", "time"] }
tracing       = "0.1"
rand          = "0.8"

reqwest       = { version = "0.12", optional = true, default-features = false, features = ["json", "rustls-tls"] }
jsonwebtoken  = { version = "9",    optional = true }
axum          = { version = "0.8",  optional = true }
tower         = { version = "0.5",  optional = true }
tonic         = { version = "0.13", optional = true }
sha2          = { version = "0.10", optional = true }
base64        = { version = "0.22", optional = true }
cbor4ii       = { version = "0.3",  optional = true }
p256          = { version = "0.13", optional = true }
```

---

## src/lib.rs

```rust
#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2024_compatibility)]

pub mod error;
pub mod types;
pub mod principal;
pub mod flow;
pub mod credential;
pub mod token;
pub mod session;
pub mod pending;
pub mod policy;

#[cfg(feature = "oauth2")]   pub mod oauth2;
#[cfg(feature = "oidc")]     pub mod oidc;
#[cfg(feature = "webauthn")] pub mod webauthn;
#[cfg(feature = "axum")]     pub mod axum;
#[cfg(feature = "reqwest")]  pub mod reqwest;

// Top-level re-exports — всё самое нужное без пути
pub use error::AuthError;
pub use principal::AuthenticatedPrincipal;
pub use token::TokenSet;
pub use flow::{
    Authenticator,
    InteractiveAuthenticator,
    DynInteractiveAuthenticator,
    FlowStatus,
    FlowState,
    AuthAction,
    FlowEvent,
    PromptKind,
};
```

---

## src/types.rs

```rust
//! Taxonomy enums. Каждый отвечает на свой вопрос — не смешиваются в один enum.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    OAuth2, OpenIdConnect, Saml2, WebAuthn, Kerberos, Gnap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Password, ApiKey, Passkey, Totp, SmsOtp, EmailCode,
    MagicLinkToken, ClientCertificate, ClientSecret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenFormat {
    Jwt, Paseto, Opaque, SamlAssertion, Macaroon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationKind {
    BearerHeader, BasicHeader, Cookie, MutualTls, QueryParameter, CustomHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    ServerSide, Stateless, RefreshToken, Sliding, DeviceBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    NonInteractive, Interactive, SemiInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitiatorKind {
    User, Client, Service, Device, ExternalProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowChannel {
    BrowserRedirect, Backchannel, DirectRequest, CrossDevice, LocalDevice,
}

/// Метаданные flow — для discovery, диагностики, UI.
/// Не влияет на исполнение, только описание.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthFlowDescriptor {
    pub protocol:               Option<ProtocolKind>,
    pub credential:             CredentialKind,
    pub token_format:           Option<TokenFormat>,
    pub presentation:           PresentationKind,
    pub session:                Option<SessionKind>,
    pub interaction:            InteractionKind,
    pub initiator:              InitiatorKind,
    pub channel:                FlowChannel,
    pub supports_user_presence: bool,
    pub supports_user_consent:  bool,
    pub supports_redirect:      bool,
    pub supports_backchannel:   bool,
    pub supports_machine_only:  bool,
}
```

---

## src/flow.rs

```rust
//! Трейты аутентификации + state machine примитивы.
//! Нет I/O. Нет runtime-зависимостей.

use std::{future::Future, pin::Pin, time::SystemTime};

// ─── FlowState ────────────────────────────────────────────────────────────────

pub trait FlowState: Send + 'static {
    fn expires_at(&self) -> Option<SystemTime>;

    fn is_expired(&self) -> bool {
        self.expires_at()
            .map(|t| t <= SystemTime::now())
            .unwrap_or(false)
    }
}

// ─── FlowStatus ───────────────────────────────────────────────────────────────

pub enum FlowStatus<A, S, O> {
    ActionRequired { action: A, pending: S },
    Completed(O),
}

// ─── AuthAction ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthAction {
    Redirect { url: String },
    ShowUserCode {
        user_code:                 String,
        verification_uri:          String,
        verification_uri_complete: Option<String>,
        expires_in_secs:           u64,
    },
    Prompt {
        kind:    PromptKind,
        message: Option<String>,
    },
    AwaitCallback  { timeout_secs: u64 },
    PollAfter      { seconds: u64 },
    AwaitPushApproval { timeout_secs: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Password, Otp, Totp, EmailCode, Consent,
    WebAuthnChallenge, NewPassword, SecurityQuestion,
}

// ─── FlowEvent ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowEvent {
    CallbackReceived        { query: String },
    UserCodeEntered         { code: String },
    PasswordSubmitted       { password: String },
    OtpSubmitted            { code: String },
    NewPasswordSubmitted    { password: String },
    WebAuthnResponse        { response: Vec<u8> },
    ConsentGiven            { scopes: Vec<String> },
    ConsentDenied,
    Poll,
    Cancel,
}

// ─── Authenticator (non-interactive) ─────────────────────────────────────────

pub trait Authenticator<Cx>: Send + Sync {
    type Output: Send;
    type Error: std::error::Error + Send + Sync + 'static;

    fn authenticate(
        &self,
        cx: &Cx,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

// ─── InteractiveAuthenticator ─────────────────────────────────────────────────

pub trait InteractiveAuthenticator: Send + Sync {
    type Input:  Send;
    type Action: Send;
    type State:  FlowState + serde::Serialize + for<'de> serde::Deserialize<'de> + Send;
    type Output: Send;
    type Error:  std::error::Error + Send + Sync + 'static;

    fn begin(
        &self,
        input: Self::Input,
    ) -> impl Future<
        Output = Result<FlowStatus<Self::Action, Self::State, Self::Output>, Self::Error>,
    > + Send;

    fn advance(
        &self,
        state: Self::State,
        event: FlowEvent,
    ) -> impl Future<
        Output = Result<FlowStatus<Self::Action, Self::State, Self::Output>, Self::Error>,
    > + Send;
}

// ─── DynInteractiveAuthenticator ──────────────────────────────────────────────

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type DynErr        = Box<dyn std::error::Error + Send + Sync>;

/// Type-erased версия для registry, где хранятся разные flow одновременно.
/// Input / State / Output сериализованы в `serde_json::Value`.
///
/// Blanket impl ниже покрывает любой `InteractiveAuthenticator` автоматически.
pub trait DynInteractiveAuthenticator: Send + Sync {
    fn descriptor(&self) -> &crate::types::AuthFlowDescriptor;

    fn begin_dyn(
        &self,
        input: serde_json::Value,
    ) -> BoxFut<'_, Result<serde_json::Value, DynErr>>;

    fn advance_dyn(
        &self,
        state: serde_json::Value,
        event: FlowEvent,
    ) -> BoxFut<'_, Result<serde_json::Value, DynErr>>;
}

// Blanket impl — коммьюнити не пишет бойлерплейт для registry
impl<T> DynInteractiveAuthenticator for T
where
    T: InteractiveAuthenticator + Send + Sync,
    T::Input:  serde::de::DeserializeOwned + Send,
    T::State:  serde::Serialize + serde::de::DeserializeOwned,
    T::Output: serde::Serialize,
    T: HasDescriptor,
{
    fn descriptor(&self) -> &crate::types::AuthFlowDescriptor {
        HasDescriptor::descriptor(self)
    }

    fn begin_dyn(
        &self,
        input: serde_json::Value,
    ) -> BoxFut<'_, Result<serde_json::Value, DynErr>> {
        Box::pin(async move {
            let input: T::Input = serde_json::from_value(input)?;
            let result = self.begin(input).await.map_err(|e| Box::new(e) as DynErr)?;
            encode_status(result)
        })
    }

    fn advance_dyn(
        &self,
        state: serde_json::Value,
        event: FlowEvent,
    ) -> BoxFut<'_, Result<serde_json::Value, DynErr>> {
        Box::pin(async move {
            let state: T::State = serde_json::from_value(state)?;
            let result = self.advance(state, event).await.map_err(|e| Box::new(e) as DynErr)?;
            encode_status(result)
        })
    }
}

pub trait HasDescriptor {
    fn descriptor(&self) -> &crate::types::AuthFlowDescriptor;
}

fn encode_status<A, S: serde::Serialize, O: serde::Serialize>(
    status: FlowStatus<A, S, O>,
) -> Result<serde_json::Value, DynErr> {
    match status {
        FlowStatus::ActionRequired { pending, .. } => Ok(serde_json::to_value(&pending)?),
        FlowStatus::Completed(out)                 => Ok(serde_json::to_value(&out)?),
    }
}
```

---

## src/credential.rs

```rust
//! Типы credentials + трейт верификации.
//! Sealed — конструируются только через named constructors.

use zeroize::Zeroizing;

mod sealed { pub trait Sealed {} }
pub trait Credential: sealed::Sealed + Send + Sync {
    fn kind(&self) -> crate::types::CredentialKind;
}

// ─── Типы ─────────────────────────────────────────────────────────────────────

pub struct ApiKey(pub(crate) Zeroizing<String>);
pub struct Password(pub(crate) Zeroizing<String>);
pub struct TotpCode(pub String);
pub struct EmailCode(pub String);
pub struct MagicLinkToken(pub String);

pub struct ClientSecret {
    pub client_id:     String,
    pub client_secret: Zeroizing<String>,
}

pub struct ClientCertificate {
    pub der: Vec<u8>,
}

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self { Self(Zeroizing::new(key.into())) }
    pub fn expose(&self) -> &str { &self.0 }
}

impl Password {
    pub fn new(pwd: impl Into<String>) -> Self { Self(Zeroizing::new(pwd.into())) }
    pub fn expose(&self) -> &str { &self.0 }
}

impl ClientSecret {
    pub fn new(client_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self { client_id: client_id.into(), client_secret: Zeroizing::new(secret.into()) }
    }
}

// impl Credential + sealed::Sealed для каждого...
impl sealed::Sealed for ApiKey {}
impl Credential for ApiKey {
    fn kind(&self) -> crate::types::CredentialKind { crate::types::CredentialKind::ApiKey }
}
// ... аналогично для остальных

// ─── Верификация ──────────────────────────────────────────────────────────────

pub trait CredentialVerifier<C: Credential>: Send + Sync {
    type Identity;
    type Error: std::error::Error + Send + Sync + 'static;

    fn verify(
        &self,
        credential: &C,
    ) -> impl std::future::Future<Output = Result<Self::Identity, Self::Error>> + Send;
}
```

---

## src/token.rs

```rust
//! TokenSet, JWT verification, opaque token introspection.

use std::future::Future;

// ─── TokenSet ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSet {
    pub access_token:  String,
    pub token_type:    String,
    pub expires_in:    Option<u64>,
    pub refresh_token: Option<zeroize::Zeroizing<String>>,
    pub scope:         Option<String>,
    pub id_token:      Option<String>,
}

// ─── JWT ──────────────────────────────────────────────────────────────────────

#[cfg(feature = "oidc")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StandardClaims {
    pub sub: String,
    pub iss: Option<String>,
    pub aud: Option<serde_json::Value>,
    pub exp: Option<i64>,
    pub nbf: Option<i64>,
    pub iat: Option<i64>,
    pub jti: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Порт для загрузки ключей верификации JWT.
/// Реализация (HTTP + кеш) — у интегратора.
pub trait JwksSource: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn get_key(
        &self,
        kid: Option<&str>,
        alg: Option<&str>,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
}

// ─── Opaque token ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueToken(pub String);

/// Порт для проверки непрозрачных токенов (lookup в БД / introspect endpoint).
pub trait TokenIntrospector: Send + Sync {
    type Claims;
    type Error: std::error::Error + Send + Sync + 'static;

    fn introspect(
        &self,
        token: &OpaqueToken,
    ) -> impl Future<Output = Result<Option<Self::Claims>, Self::Error>> + Send;

    fn revoke(
        &self,
        token: &OpaqueToken,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

---

## src/session.rs

```rust
use std::{future::Future, time::{Duration, SystemTime}};

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self { Self(uuid::Uuid::now_v7().to_string()) }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Session<D = serde_json::Value> {
    pub id:         SessionId,
    pub principal:  crate::principal::AuthenticatedPrincipal,
    pub data:       D,
    pub created_at: SystemTime,
    pub expires_at: Option<SystemTime>,
}

/// Порт. Реализация — у интегратора (sqlx, Redis, in-memory).
pub trait SessionStore: Send + Sync {
    type Data: Send + serde::Serialize + for<'de> serde::Deserialize<'de>;
    type Error: std::error::Error + Send + Sync + 'static;

    fn create(
        &self,
        principal: crate::principal::AuthenticatedPrincipal,
        data:      Self::Data,
        ttl:       Option<Duration>,
    ) -> impl Future<Output = Result<Session<Self::Data>, Self::Error>> + Send;

    fn get(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<Session<Self::Data>>, Self::Error>> + Send;

    fn refresh(
        &self,
        id:  &SessionId,
        ttl: Duration,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn delete(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

---

## src/pending.rs

```rust
use std::{future::Future, time::Duration};
use crate::flow::FlowState;

/// Порт для хранения pending state между шагами interactive flow.
/// Реализация — у интегратора (sqlx, Redis, DashMap).
pub trait PendingStateStore<S>: Send + Sync
where
    S: FlowState + serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type Error: std::error::Error + Send + Sync + 'static;

    fn save(
        &self,
        key: &str,
        state: &S,
        ttl: Duration,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn load(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<S>, Self::Error>> + Send;

    fn remove(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Type-erased версия — нужна когда registry хранит разные типы state.
pub trait DynPendingStore: Send + Sync {
    fn save_json(
        &self,
        key:   &str,
        value: &serde_json::Value,
        ttl:   Duration,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + '_>>;

    fn load_json(
        &self,
        key: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>>> + Send + '_>>;

    fn remove_json(
        &self,
        key: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + '_>>;
}
```

---

## src/oauth2.rs

```rust
//! OAuth 2.0: Authorization Code, Client Credentials, Device Flow.
//! feature = "oauth2"

use std::time::{Duration, SystemTime};
use zeroize::Zeroizing;
use crate::flow::{
    Authenticator, InteractiveAuthenticator,
    FlowStatus, FlowEvent, FlowState, AuthAction,
};
use crate::token::TokenSet;

// ─── Config ───────────────────────────────────────────────────────────────────

pub struct OAuthEndpoints {
    pub authorization: String,
    pub token:         String,
    pub revocation:    Option<String>,
    pub introspection: Option<String>,
}

pub struct AuthCodeConfig {
    pub client_id:     String,
    pub client_secret: Option<Zeroizing<String>>,
    pub redirect_uri:  String,
    pub scopes:        Vec<String>,
    pub endpoints:     OAuthEndpoints,
    pub extra_params:  Vec<(String, String)>,
}

pub struct ClientCredentialsConfig {
    pub client_id:     String,
    pub client_secret: Zeroizing<String>,
    pub scopes:        Vec<String>,
    pub endpoint:      String,
}

// ─── PKCE ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceMethod { S256, Plain }

pub struct PkceChallenge {
    pub verifier:  Zeroizing<String>,
    pub challenge: String,
    pub method:    PkceMethod,
}

impl PkceChallenge {
    pub fn new_s256() -> Self {
        use rand::Rng;
        use sha2::{Sha256, Digest};
        let verifier: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        let digest  = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(digest);
        Self { verifier: Zeroizing::new(verifier), challenge, method: PkceMethod::S256 }
    }
}

// ─── Pending states ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthCodePendingState {
    pub csrf_state:    String,
    pub pkce_verifier: Option<String>,
    pub nonce:         Option<String>,
    pub redirect_uri:  String,
    pub expires_at:    SystemTime,
}

impl FlowState for AuthCodePendingState {
    fn expires_at(&self) -> Option<SystemTime> { Some(self.expires_at) }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceFlowPendingState {
    pub device_code: String,
    pub interval:    u64,
    pub expires_at:  SystemTime,
}

impl FlowState for DeviceFlowPendingState {
    fn expires_at(&self) -> Option<SystemTime> { Some(self.expires_at) }
}

// ─── AuthorizationCodeFlow ────────────────────────────────────────────────────

pub struct AuthCodeInput {
    pub extra_params: Vec<(String, String)>,
}

pub struct AuthorizationCodeFlow {
    pub config: AuthCodeConfig,
    http:       reqwest::Client,
}

impl AuthorizationCodeFlow {
    pub fn new(config: AuthCodeConfig) -> Self {
        Self { config, http: reqwest::Client::new() }
    }
}

impl InteractiveAuthenticator for AuthorizationCodeFlow {
    type Input  = AuthCodeInput;
    type Action = AuthAction;
    type State  = AuthCodePendingState;
    type Output = TokenSet;
    type Error  = OAuth2Error;

    async fn begin(
        &self,
        input: AuthCodeInput,
    ) -> Result<FlowStatus<AuthAction, AuthCodePendingState, TokenSet>, OAuth2Error> {
        let pkce       = PkceChallenge::new_s256();
        let csrf_state = random_string(32);
        let nonce      = random_string(32);

        let mut url = url::Url::parse(&self.config.endpoints.authorization)
            .map_err(|_| OAuth2Error::InvalidEndpoint)?;

        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id",     &self.config.client_id)
            .append_pair("redirect_uri",  &self.config.redirect_uri)
            .append_pair("scope",         &self.config.scopes.join(" "))
            .append_pair("state",         &csrf_state)
            .append_pair("nonce",         &nonce)
            .append_pair("code_challenge",        &pkce.challenge)
            .append_pair("code_challenge_method", "S256");

        for (k, v) in &self.config.extra_params {
            url.query_pairs_mut().append_pair(k, v);
        }
        for (k, v) in &input.extra_params {
            url.query_pairs_mut().append_pair(k, v);
        }

        Ok(FlowStatus::ActionRequired {
            action: AuthAction::Redirect { url: url.to_string() },
            pending: AuthCodePendingState {
                csrf_state,
                pkce_verifier: Some((*pkce.verifier).clone()),
                nonce:         Some(nonce),
                redirect_uri:  self.config.redirect_uri.clone(),
                expires_at:    SystemTime::now() + Duration::from_secs(600),
            },
        })
    }

    async fn advance(
        &self,
        state: AuthCodePendingState,
        event: FlowEvent,
    ) -> Result<FlowStatus<AuthAction, AuthCodePendingState, TokenSet>, OAuth2Error> {
        if state.is_expired() { return Err(OAuth2Error::FlowExpired); }

        match event {
            FlowEvent::CallbackReceived { query } => {
                let params: std::collections::HashMap<String, String> =
                    serde_urlencoded::from_str(&query)
                        .map_err(|_| OAuth2Error::InvalidCallback)?;

                let returned_state = params.get("state").map(String::as_str).unwrap_or("");
                if returned_state != state.csrf_state {
                    return Err(OAuth2Error::StateMismatch);
                }

                let code = params.get("code")
                    .ok_or(OAuth2Error::MissingCode)?
                    .clone();

                let tokens = self.exchange_code(code, &state).await?;
                Ok(FlowStatus::Completed(tokens))
            }
            FlowEvent::Cancel => Err(OAuth2Error::Cancelled),
            _                 => Err(OAuth2Error::UnexpectedEvent),
        }
    }
}

impl AuthorizationCodeFlow {
    async fn exchange_code(
        &self,
        code:  String,
        state: &AuthCodePendingState,
    ) -> Result<TokenSet, OAuth2Error> {
        let mut body = vec![
            ("grant_type",   "authorization_code".to_owned()),
            ("code",         code),
            ("redirect_uri", state.redirect_uri.clone()),
            ("client_id",    self.config.client_id.clone()),
        ];
        if let Some(verifier) = &state.pkce_verifier {
            body.push(("code_verifier", verifier.clone()));
        }
        if let Some(secret) = &self.config.client_secret {
            body.push(("client_secret", secret.as_str().to_owned()));
        }

        let resp: TokenSet = self.http
            .post(&self.config.endpoints.token)
            .form(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp)
    }
}

// ─── ClientCredentialsFlow ────────────────────────────────────────────────────

pub struct ClientCredentialsFlow {
    pub config: ClientCredentialsConfig,
    http:       reqwest::Client,
}

impl ClientCredentialsFlow {
    pub fn new(config: ClientCredentialsConfig) -> Self {
        Self { config, http: reqwest::Client::new() }
    }
}

impl Authenticator<()> for ClientCredentialsFlow {
    type Output = TokenSet;
    type Error  = OAuth2Error;

    async fn authenticate(&self, _: &()) -> Result<TokenSet, OAuth2Error> {
        let body = [
            ("grant_type",    "client_credentials"),
            ("client_id",     &self.config.client_id),
            ("client_secret", self.config.client_secret.as_str()),
            ("scope",         &self.config.scopes.join(" ")),
        ];
        let resp: TokenSet = self.http
            .post(&self.config.endpoint)
            .form(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }
}

// ─── DeviceFlow ───────────────────────────────────────────────────────────────

pub struct DeviceFlowConfig {
    pub client_id:           String,
    pub scopes:              Vec<String>,
    pub device_auth_endpoint: String,
    pub token_endpoint:      String,
}

pub struct DeviceFlow {
    pub config: DeviceFlowConfig,
    http:       reqwest::Client,
}

impl DeviceFlow {
    pub fn new(config: DeviceFlowConfig) -> Self {
        Self { config, http: reqwest::Client::new() }
    }
}

impl InteractiveAuthenticator for DeviceFlow {
    type Input  = ();
    type Action = AuthAction;
    type State  = DeviceFlowPendingState;
    type Output = TokenSet;
    type Error  = OAuth2Error;

    async fn begin(
        &self,
        _: (),
    ) -> Result<FlowStatus<AuthAction, DeviceFlowPendingState, TokenSet>, OAuth2Error> {
        #[derive(serde::Deserialize)]
        struct DeviceAuthResponse {
            device_code:               String,
            user_code:                 String,
            verification_uri:          String,
            verification_uri_complete: Option<String>,
            expires_in:                u64,
            interval:                  Option<u64>,
        }

        let resp: DeviceAuthResponse = self.http
            .post(&self.config.device_auth_endpoint)
            .form(&[
                ("client_id", &*self.config.client_id),
                ("scope",     &*self.config.scopes.join(" ")),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let interval = resp.interval.unwrap_or(5);

        Ok(FlowStatus::ActionRequired {
            action: AuthAction::ShowUserCode {
                user_code:                 resp.user_code,
                verification_uri:          resp.verification_uri,
                verification_uri_complete: resp.verification_uri_complete,
                expires_in_secs:           resp.expires_in,
            },
            pending: DeviceFlowPendingState {
                device_code: resp.device_code,
                interval,
                expires_at:  SystemTime::now() + Duration::from_secs(resp.expires_in),
            },
        })
    }

    async fn advance(
        &self,
        state: DeviceFlowPendingState,
        event: FlowEvent,
    ) -> Result<FlowStatus<AuthAction, DeviceFlowPendingState, TokenSet>, OAuth2Error> {
        if state.is_expired() { return Err(OAuth2Error::FlowExpired); }

        match event {
            FlowEvent::Poll => {
                #[derive(serde::Deserialize)]
                #[serde(untagged)]
                enum PollResponse {
                    Token(TokenSet),
                    Error { error: String },
                }

                let resp: PollResponse = self.http
                    .post(&self.config.token_endpoint)
                    .form(&[
                        ("grant_type",  "urn:ietf:params:oauth:grant-type:device_code"),
                        ("device_code", &*state.device_code),
                        ("client_id",   &*self.config.client_id),
                    ])
                    .send()
                    .await?
                    .json()
                    .await?;

                match resp {
                    PollResponse::Token(tokens) => Ok(FlowStatus::Completed(tokens)),
                    PollResponse::Error { error } => match error.as_str() {
                        "authorization_pending" => Ok(FlowStatus::ActionRequired {
                            action:  AuthAction::PollAfter { seconds: state.interval },
                            pending: state,
                        }),
                        "slow_down" => Ok(FlowStatus::ActionRequired {
                            action:  AuthAction::PollAfter { seconds: state.interval + 5 },
                            pending: DeviceFlowPendingState {
                                interval: state.interval + 5,
                                ..state
                            },
                        }),
                        "access_denied" => Err(OAuth2Error::AccessDenied),
                        "expired_token" => Err(OAuth2Error::FlowExpired),
                        other           => Err(OAuth2Error::ProviderError(other.to_owned())),
                    },
                }
            }
            FlowEvent::Cancel => Err(OAuth2Error::Cancelled),
            _                 => Err(OAuth2Error::UnexpectedEvent),
        }
    }
}

// ─── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
    #[error("http error: {0}")]           Http(#[from] reqwest::Error),
    #[error("invalid endpoint url")]      InvalidEndpoint,
    #[error("invalid callback query")]    InvalidCallback,
    #[error("csrf state mismatch")]       StateMismatch,
    #[error("missing authorization code")] MissingCode,
    #[error("flow expired")]              FlowExpired,
    #[error("access denied by user")]     AccessDenied,
    #[error("cancelled")]                 Cancelled,
    #[error("unexpected event")]          UnexpectedEvent,
    #[error("provider error: {0}")]       ProviderError(String),
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn random_string(len: usize) -> String {
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(rand::distributions::Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}
```

---

## Итого: что в каком файле

| Файл | Содержимое |
|---|---|
| `types.rs` | все taxonomy enums + `AuthFlowDescriptor` |
| `principal.rs` | `AuthenticatedPrincipal` |
| `error.rs` | `AuthError` (top-level) |
| `flow.rs` | `FlowState`, `FlowStatus`, `AuthAction`, `FlowEvent`, `PromptKind`, `Authenticator`, `InteractiveAuthenticator`, `DynInteractiveAuthenticator`, blanket impl |
| `credential.rs` | все типы credential + `CredentialVerifier` |
| `token.rs` | `TokenSet`, `StandardClaims`, `JwksSource`, `OpaqueToken`, `TokenIntrospector` |
| `session.rs` | `Session`, `SessionId`, `SessionStore` |
| `pending.rs` | `PendingStateStore`, `DynPendingStore` |
| `policy.rs` | `RequireScopes`, `Role`, `Permission`, `RoleResolver`, `PermissionChecker`, `Policy`, `PolicyDecision`, `PolicyContext`, `AllOf`, `AnyOf`, `Not` |
| `oauth2.rs` | `OAuthEndpoints`, `AuthCodeConfig`, `ClientCredentialsConfig`, `DeviceFlowConfig`, `PkceChallenge`, `AuthCodePendingState`, `DeviceFlowPendingState`, `AuthorizationCodeFlow`, `ClientCredentialsFlow`, `DeviceFlow`, `OAuth2Error` |
| `oidc.rs` | `OidcConfig`, `OidcMetadata`, `IdTokenClaims`, `UserInfo`, `OidcAuthFlow`, `discover()`, `OidcError` |
| `webauthn.rs` | `WebAuthnConfig`, `CredentialRecord`, `RegistrationPendingState`, `AuthenticationPendingState`, `RegistrationFlow`, `AuthenticationFlow`, `WebAuthnError` |
| `axum.rs` | `BearerToken`, `VerifiedPrincipal`, `RequireAuthLayer`, `oauth_callback_router()` |
| `reqwest.rs` | `CachedTokenProvider`, `BearerAuthMiddleware` |
