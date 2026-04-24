# Schemes catalog

**Статус:** draft — перечень built-in auth schemes с injection mechanics. Part of exploratory notes.

Purpose: document design intent для каждого scheme type before implementation. Ne all этих types существуют как built-in sегодня; это target после redesign.

## Распределение по данным n8n (428 credential types)

| Bucket | Count | % | Complexity | Nebula home |
|---|---|---|---|---|
| API Key / Bearer | 252 | 59% | Low | `credential-builtin/api_key/*` |
| OAuth 2.0 | 108 | 25% | High | `credential-builtin/oauth2/*` + per-service wrappers |
| Basic Auth | 25 | 6% | Low | `credential-builtin/basic/*` |
| Custom signing | 12 | 3% | High | `credential-builtin/custom/*` |
| DB connection | 10 | 2% | Medium | `credential-builtin/db/*` |
| Message queues | 4 | 1% | Medium | `credential-builtin/queue/*` |
| AWS static | 2 | 0.5% | Medium | `credential-builtin/aws/*` |
| Other | 14 | 3% | Mixed | Various |

**Observation:** 84% — только 2 families (API Key + OAuth2). Designed optimize hot path for these; tail schemes less frequent but each is enterprise-critical в своём domain.

## Scheme catalog

Each entry: **Name → Pattern → Capability markers → Injection mechanics → Typical Credential types**.

### 1. Static header injection (bearers, api keys)

**Scheme:** `BearerScheme`
**AuthPattern:** `SecretToken`
**Capabilities:** `AcceptsBearer`

```rust
#[derive(ZeroizeOnDrop)]
pub struct BearerScheme {
    secret: SecretString,
    // optional non-standard prefix (default: "Bearer")
    #[zeroize(skip)] pub prefix: Option<String>,
}

impl SchemeInjector for BearerScheme {
    fn inject(&self, req: &mut RequestParts) -> Result<(), InjectError> {
        self.secret.expose(|raw| {
            let prefix = self.prefix.as_deref().unwrap_or("Bearer");
            req.headers_mut().insert(
                AUTHORIZATION,
                format!("{prefix} {raw}").parse()?
            );
            Ok(())
        })
    }
}

impl AcceptsBearer for BearerScheme {}
```

**Credential types using this scheme:**
- `OAuth2Credential::Scheme = BearerScheme` (access_token → Bearer)
- `AnthropicApiKey::Scheme = BearerScheme` (x-api-key → custom header via different scheme — or generic with prefix)
- Most API Key credentials (252 из 428)

### 2. Custom header injection (non-Bearer)

**Scheme:** `HeaderScheme`
**AuthPattern:** `SecretToken` (same family, different injection)
**Capabilities:** `AcceptsBearer` (if convertible) or just `AcceptsHeader` (new marker)

```rust
#[derive(ZeroizeOnDrop)]
pub struct HeaderScheme {
    secret: SecretString,
    #[zeroize(skip)] pub header_name: HeaderName,
    #[zeroize(skip)] pub prefix: Option<String>,
    #[zeroize(skip)] pub extra_headers: Vec<(HeaderName, String)>,  // non-secret
}

impl SchemeInjector for HeaderScheme {
    fn inject(&self, req: &mut RequestParts) -> Result<(), InjectError> {
        for (name, value) in &self.extra_headers {
            req.headers_mut().insert(name, value.parse()?);
        }
        self.secret.expose(|raw| {
            let value = match &self.prefix {
                Some(p) => format!("{p}{raw}"),
                None => raw.to_string(),
            };
            req.headers_mut().insert(&self.header_name, value.parse()?);
            Ok(())
        })
    }
}
```

**Examples:**
- Stripe: `Authorization: Bearer sk_live_...` → use BearerScheme
- GitHub Personal Access Token: `Authorization: token ghp_...` → HeaderScheme с prefix="token "
- AWS X-API-Key header: HeaderScheme с header_name = "x-api-key"
- HubSpot: `X-HubSpot-Api-Key` header

### 3. Multiple headers / vendor-specific

**Scheme:** `MultiHeaderScheme`
**AuthPattern:** `SecretToken` with structured headers
**Capabilities:** depends

```rust
#[derive(ZeroizeOnDrop)]
pub struct MultiHeaderScheme {
    // Secret headers
    secrets: Vec<(HeaderName, SecretString)>,
    // Public headers (workspace, tenant, etc.)
    #[zeroize(skip)] pub public_headers: Vec<(HeaderName, String)>,
}
```

**Examples:**
- Zendesk: `X-Zendesk-Subdomain` + `Authorization: Basic ...`
- Some SaaS: tenant/account header + secret header

### 4. Query parameter injection

**Scheme:** `QueryScheme`
**AuthPattern:** `SecretToken`
**Capabilities:** `AcceptsQuery` (new marker) — explicitly NOT AcceptsBearer

**Rationale:** query-param auth не безопасен (URLs log'ются в access logs, caches). Marker разделён чтобы action/resource declarations могли force-require Bearer.

```rust
#[derive(ZeroizeOnDrop)]
pub struct QueryScheme {
    secret: SecretString,
    #[zeroize(skip)] pub param_name: String,
}

impl SchemeInjector for QueryScheme {
    fn inject(&self, req: &mut RequestParts) -> Result<(), InjectError> {
        self.secret.expose(|raw| {
            req.uri_mut().append_query(&self.param_name, raw);
            Ok(())
        })
    }
}
```

**Examples:**
- OpenWeatherMap: `?appid=...`
- Legacy SOAP APIs
- Some webhook signatures

### 5. Basic auth

**Scheme:** `BasicAuthScheme`
**AuthPattern:** `IdentityPassword`
**Capabilities:** `AcceptsBearer` (after base64 encode, becomes Authorization Basic)

```rust
#[derive(ZeroizeOnDrop)]
pub struct BasicAuthScheme {
    #[zeroize(skip)] pub username: String,  // public
    password: SecretString,
}

impl SchemeInjector for BasicAuthScheme {
    fn inject(&self, req: &mut RequestParts) -> Result<(), InjectError> {
        self.password.expose(|pw| {
            let pair = format!("{}:{}", self.username, pw);
            let encoded = base64::encode(&pair);
            req.headers_mut().insert(
                AUTHORIZATION,
                format!("Basic {encoded}").parse()?
            );
            Ok(())
        })
    }
}

impl AcceptsBearer for BasicAuthScheme {}  // Authorization: Basic принимается как Bearer-like
```

**Examples:**
- Jira Server, many legacy APIs
- 25 в n8n

### 6. OAuth2 token (projected from OAuth2Credential::State)

**Scheme:** `OAuth2TokenScheme`
**AuthPattern:** `OAuth2`
**Capabilities:** `AcceptsBearer`

```rust
#[derive(ZeroizeOnDrop)]
pub struct OAuth2TokenScheme {
    access_token: SecretString,
    #[zeroize(skip)] pub token_type: TokenType,  // Bearer | DPoP | MAC — usually Bearer
    #[zeroize(skip)] pub scopes: Vec<String>,    // public — for capability matching
    #[zeroize(skip)] pub expires_at: DateTime<Utc>,  // public
}

impl nebula_core::AuthScheme for OAuth2TokenScheme {
    fn pattern() -> AuthPattern { AuthPattern::OAuth2 }
    fn expires_at(&self) -> Option<DateTime<Utc>> { Some(self.expires_at) }
}

impl SchemeInjector for OAuth2TokenScheme {
    fn inject(&self, req: &mut RequestParts) -> Result<(), InjectError> {
        self.access_token.expose(|tok| {
            req.headers_mut().insert(
                AUTHORIZATION,
                format!("{} {}", self.token_type.header_scheme(), tok).parse()?
            );
            Ok(())
        })
    }
}

impl AcceptsBearer for OAuth2TokenScheme {}
```

### 7. AWS SigV4 — per-request signing

**Scheme:** `AwsSigV4Scheme`
**AuthPattern:** `RequestSigning`
**Capabilities:** `AcceptsSigning` (NOT AcceptsBearer — requires per-request work)

```rust
#[derive(ZeroizeOnDrop)]
pub struct AwsSigV4Scheme {
    // Public — logged в non-secret contexts
    #[zeroize(skip)] pub access_key_id: String,
    #[zeroize(skip)] pub region: String,
    #[zeroize(skip)] pub service: String,
    // Secret
    secret_access_key: SecretString,
    // For STS-derived temporary creds
    session_token: Option<SecretString>,
    #[zeroize(skip)] pub expires_at: Option<DateTime<Utc>>,  // Some() for temporary
}

impl SchemeInjector for AwsSigV4Scheme {
    fn inject(&self, _req: &mut RequestParts) -> Result<(), InjectError> {
        Err(InjectError::RequiresSigning)  // cannot statically inject
    }
    
    fn sign(&self, req: &mut RequestParts, ctx: &SigningContext) -> Result<(), InjectError> {
        self.secret_access_key.expose(|sk| {
            aws_sigv4::sign(
                req,
                &self.access_key_id,
                sk,
                &self.region,
                &self.service,
                self.session_token.as_ref().map(|s| s.expose_secret()),
                ctx.now,
                ctx.body_hash.as_deref(),  // pre-computed by caller for streaming
            )
        })
    }
}

impl AcceptsSigning for AwsSigV4Scheme {}
```

**Finding #11 — streaming body:** `SigningContext::body_hash` — pre-computed SHA256 или `UNSIGNED-PAYLOAD` option. Caller responsibility to compute. For streaming workflow data (multi-GB S3 upload), resource uses `UNSIGNED-PAYLOAD` if TLS guarantees integrity.

### 8. OAuth1 — per-request signing (legacy but still needed)

**Scheme:** `OAuth1Scheme`
**AuthPattern:** `RequestSigning`
**Capabilities:** `AcceptsSigning`

```rust
#[derive(ZeroizeOnDrop)]
pub struct OAuth1Scheme {
    #[zeroize(skip)] pub consumer_key: String,        // public
    consumer_secret: SecretString,                     // secret
    access_token: SecretString,                        // secret
    token_secret: SecretString,                        // secret
    #[zeroize(skip)] pub signature_method: OAuth1SignatureMethod,  // HMAC-SHA1 | RSA-SHA1 | PLAINTEXT
}

impl SchemeInjector for OAuth1Scheme {
    fn sign(&self, req: &mut RequestParts, ctx: &SigningContext) -> Result<(), InjectError> {
        // Build signature base string from method + URL + sorted params + body hash
        // Sign с consumer_secret + token_secret
        // Inject Authorization: OAuth oauth_consumer_key="...", oauth_signature="...", ...
    }
}

impl AcceptsSigning for OAuth1Scheme {}
```

**Examples:** Twitter OAuth1, some legacy SaaS.

### 9. HMAC webhook signing (Shopify, GitHub, Stripe signatures)

**Scheme:** `HmacSigningScheme`
**AuthPattern:** `RequestSigning`
**Capabilities:** `AcceptsSigning`

Используется для signing outgoing requests (или verifying incoming webhooks — но это по другой стороне).

```rust
#[derive(ZeroizeOnDrop)]
pub struct HmacSigningScheme {
    signing_secret: SecretBytes,  // raw bytes for HMAC
    #[zeroize(skip)] pub algorithm: HmacAlgorithm,  // Sha256 | Sha512
    #[zeroize(skip)] pub header_name: HeaderName,  // "X-Signature" или vendor-specific
    #[zeroize(skip)] pub encoding: SigEncoding,  // Hex | Base64 | Base64Url
    #[zeroize(skip)] pub body_source: HmacBodySource,  // Body | TimestampBody | Custom
}

impl SchemeInjector for HmacSigningScheme {
    fn sign(&self, req: &mut RequestParts, ctx: &SigningContext) -> Result<(), InjectError> {
        // Compute HMAC over body (or body + timestamp)
        // Inject signature header
    }
}

impl AcceptsSigning for HmacSigningScheme {}
```

### 10. GCP Service Account JWT-bearer

**Scheme:** `GcpServiceAccountScheme` — композитный: private_key → sign JWT → exchange для access_token → use as Bearer
**AuthPattern:** `RequestSigning` (для sign step), projected result — Bearer
**Capabilities:** `AcceptsSigning` (for JWT sign step), `AcceptsBearer` (for final access_token)

**Actual shape:** the `GcpServiceAccountCredential::State` holds `{private_key, access_token, expires_at}`. `project()` returns `BearerScheme` (from access_token) in normal case; refresh re-signs JWT + exchanges.

```rust
// Not a separate scheme in "inject" sense — it's the credential logic that matters
pub struct GcpServiceAccountCredential;

impl Credential for GcpServiceAccountCredential {
    type Config = GcpServiceAccountConfig {
        project_id: String,     // public
        client_email: String,   // public
        private_key: SecretBytes, // secret
        audience: Url,          // public
    };
    type State = GcpServiceAccountState {
        access_token: SecretString,
        expires_at: DateTime<Utc>,
        // keep for refresh
        private_key: SecretBytes,
        client_email: String,
        audience: Url,
    };
    type Scheme = BearerScheme;  // projected result
    
    fn project(state: &Self::State) -> BearerScheme {
        BearerScheme {
            secret: state.access_token.clone(),
            prefix: None,
        }
    }
    
    async fn refresh(state: &mut Self::State, ctx: &CredentialContext) -> Result<RefreshOutcome, CredentialError> {
        // 1. sign JWT assertion
        // 2. POST {audience}/token с assertion
        // 3. update state.access_token, state.expires_at
    }
}
```

### 11. Mutual TLS (mTLS client certificates)

**Scheme:** `MtlsScheme`
**AuthPattern:** `Certificate`
**Capabilities:** `AcceptsTlsIdentity`

```rust
#[derive(ZeroizeOnDrop)]
pub struct MtlsScheme {
    #[zeroize(skip)] pub cert_chain_pem: Vec<u8>,     // public (certificates are non-secret по nature)
    private_key_pem: SecretBytes,                       // secret
    #[zeroize(skip)] pub ca_bundle_pem: Option<Vec<u8>>,  // public
    #[zeroize(skip)] pub expires_at: Option<DateTime<Utc>>,  // from cert
}

impl SchemeInjector for MtlsScheme {
    fn inject(&self, _req: &mut RequestParts) -> Result<(), InjectError> {
        Err(InjectError::RequiresTlsConfig)  // cannot inject at request level
    }
    
    fn configure_tls(&self, builder: &mut TlsConfigBuilder) -> Result<(), InjectError> {
        self.private_key_pem.expose(|key_bytes| {
            builder.with_client_identity(&self.cert_chain_pem, key_bytes)?;
            if let Some(ca) = &self.ca_bundle_pem {
                builder.add_root_cert(ca)?;
            }
            Ok(())
        })
    }
}

impl AcceptsTlsIdentity for MtlsScheme {}
```

**Finding #14 — multi-credential resource:** если resource нуждается в mTLS + Bearer (enterprise internal service), resource tied к one auth scheme не hack'ается. Proposed:

```rust
// Resource with composite auth — uses two separate credentials
impl Resource for InternalServiceHttpClient {
    type Config = ...;
    // Resource declares MULTIPLE auth requirements:
    type Auth = DualAuth<dyn AcceptsTlsIdentity, dyn AcceptsBearer>;
    
    async fn create_with_auth(
        cfg: &Self::Config,
        auth: CredentialGuardTuple<dyn AcceptsTlsIdentity, dyn AcceptsBearer>,
        ctx: &ResourceContext,
    ) -> Result<Self::Handle> {
        let (mtls, bearer) = auth.split();
        let tls_cfg = { mtls.configure_tls(&mut builder)?; builder.build()? };
        let client = reqwest::Client::builder().use_preconfigured_tls(tls_cfg).build()?;
        Ok(Self { client, bearer: bearer.into_ref() })  // bearer для per-request inject
    }
}
```

**Open:** `DualAuth<A, B>` compiles? Variadic arity (3 auth credentials)? Solution unclear — prototype validation needed.

### 12. DB Connection (structured, not header-injectable)

**Scheme:** `DbConnectionScheme`
**AuthPattern:** `ConnectionUri`
**Capabilities:** `AcceptsDbConnection`

```rust
#[derive(ZeroizeOnDrop)]
pub struct DbConnectionScheme {
    #[zeroize(skip)] pub host: String,
    #[zeroize(skip)] pub port: u16,
    #[zeroize(skip)] pub database: String,
    #[zeroize(skip)] pub username: String,
    password: SecretString,
    #[zeroize(skip)] pub tls_mode: TlsMode,  // Disable | Prefer | Require | VerifyFull
    #[zeroize(skip)] pub ssh_tunnel: Option<CredentialKey>,  // composition!
    #[zeroize(skip)] pub extra_options: HashMap<String, String>,  // connect_timeout, etc.
}

impl SchemeInjector for DbConnectionScheme {
    fn inject(&self, _req: &mut RequestParts) -> Result<(), InjectError> {
        Err(InjectError::NotApplicable)  // DB — не HTTP
    }
    
    fn connection_descriptor(&self) -> Option<&ConnectionDescriptor> {
        Some(&self.as_descriptor())
    }
}

impl AcceptsDbConnection for DbConnectionScheme {}
```

**SSH tunnel composition (finding: multi-credential case):** `ssh_tunnel: Option<CredentialKey>` references another credential. Resolver при use сначала resolves DB scheme, then SSH scheme, connects SSH tunnel, opens DB через tunnel. Composition stored as key-ref (not inline), consistent с existing Credential references.

### 13. Kafka SASL

**Scheme:** `KafkaSaslScheme`
**AuthPattern:** `ChallengeResponse` (для SCRAM) or `SecretToken` (для PLAIN/OAUTHBEARER)
**Capabilities:** `AcceptsKafkaAuth` (new marker)

```rust
#[derive(ZeroizeOnDrop)]
pub enum KafkaSaslScheme {
    Plain {
        #[zeroize(skip)] username: String,
        password: SecretString,
    },
    Scram {
        #[zeroize(skip)] username: String,
        password: SecretString,
        #[zeroize(skip)] mechanism: ScramMechanism,  // SHA256 | SHA512
    },
    OAuthBearer {
        token: SecretString,  // short-lived — refresh via OAuth2Credential composition
    },
    Mtls(MtlsScheme),  // composition with mTLS
}
```

### 14. SSH Key

**Scheme:** `SshKeyScheme`
**AuthPattern:** `KeyPair`
**Capabilities:** `AcceptsSshAuth` (new marker)

```rust
#[derive(ZeroizeOnDrop)]
pub struct SshKeyScheme {
    #[zeroize(skip)] pub username: String,
    #[zeroize(skip)] pub host: String,
    #[zeroize(skip)] pub port: u16,
    private_key_pem: SecretBytes,
    passphrase: Option<SecretString>,
    #[zeroize(skip)] pub known_hosts: Vec<String>,  // public fingerprints
}

// Can ALSO implement AcceptsDbConnection для use as SSH tunnel inside DB cred
```

### 15. Webhook URL (no-secret credential)

**Scheme:** `WebhookUrlScheme`
**AuthPattern:** `SharedSecret` (арчитектурно — URL сам является секретом)
**Capabilities:** (custom — no standard marker fits well)

```rust
#[derive(ZeroizeOnDrop)]
pub struct WebhookUrlScheme {
    // URL contains secret path segment
    url: SecretString,  // treat as secret because path segment carries auth
}

impl SchemeInjector for WebhookUrlScheme {
    fn inject(&self, _req: &mut RequestParts) -> Result<(), InjectError> {
        // No injection — URL запомнен в resource/action, used as destination
        Ok(())
    }
}
```

**Examples:** Discord webhooks, Slack incoming webhooks. URL is capability — если кто-то knows URL, can post messages.

## Finding #37 — FieldSensitivity granularity

Three levels proposed:
- **Public** — logged freely, shown in UI
- **Identifier** — logged but not display-prominent (client_id, tenant_id)
- **Secret / SecretBytes** — never logged, redacted in audit

**User question:** Public vs Identifier distinction — нужна?

**Answer after review:** nope. Two levels достаточно:
- **Public** — logged freely
- **Secret** — wrapped в SecretString/SecretBytes, redacted

Identifier semantic — это UI display concern, belongs в `FieldUi` metadata (form hint, display priority). Not в sensitivity. Collapsing saves разrouting complexity.

## Summary — catalogue status

| Scheme | Pattern | Complexity | Addressed? | Notes |
|---|---|---|---|---|
| BearerScheme | SecretToken | Low | Yes | Default for OAuth2 access_token |
| HeaderScheme | SecretToken | Low | Yes | Custom header injection |
| MultiHeaderScheme | SecretToken+ | Low-Med | Yes | Zendesk-style vendor headers |
| QueryScheme | SecretToken | Low | Yes | Discouraged but needed |
| BasicAuthScheme | IdentityPassword | Low | Yes | Legacy APIs |
| OAuth2TokenScheme | OAuth2 | Med | Yes | Via OAuth2Credential::project |
| AwsSigV4Scheme | RequestSigning | High | Yes | Per-request signing, streaming concern open |
| OAuth1Scheme | RequestSigning | High | Yes | Twitter, legacy |
| HmacSigningScheme | RequestSigning | Med | Yes | Shopify/GitHub outgoing sig |
| GcpServiceAccountCredential | (composite) | High | Yes | via projection to Bearer |
| MtlsScheme | Certificate | High | Yes | TLS-level injection |
| DbConnectionScheme | ConnectionUri | Med | Yes | Composition via ssh_tunnel |
| KafkaSaslScheme | Various | Med-High | Yes | Multi-variant |
| SshKeyScheme | KeyPair | Med | Yes | Reusable as SSH tunnel |
| WebhookUrlScheme | SharedSecret | Low | Yes | No-secret credential corner case |

Не покрыто в catalog'е: **OIDC ID token**, **SAML assertion**, **LDAP bind** — all Plane A (per ADR-0033), out of scope for integration credentials.
