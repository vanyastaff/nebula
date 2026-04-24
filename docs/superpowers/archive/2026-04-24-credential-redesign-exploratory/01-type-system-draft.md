# Type system draft

**Статус:** НЕ компилируется as-is. Содержит acknowledged holes. Сlessful для prototype spike.

## Что уже есть (не трогаем)

```rust
// nebula-core/src/auth.rs — уже в production
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    fn pattern() -> AuthPattern;
    fn expires_at(&self) -> Option<DateTime<Utc>> { None }
}

// nebula-core/src/accessor.rs — уже dyn-safe
pub trait CredentialAccessor: Send + Sync {
    fn has(&self, key: &CredentialKey) -> bool;
    fn resolve_any(&self, key: &CredentialKey) -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CoreError>>;
    fn try_resolve_any(&self, key: &CredentialKey) -> BoxFuture<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>>;
}

// nebula-core/src/accessor.rs — уже в core
pub trait RefreshCoordinator: Send + Sync {
    fn acquire_refresh(&self, credential_id: &str) -> BoxFuture<'_, Result<RefreshToken, CoreError>>;
    fn release_refresh(&self, token: RefreshToken) -> BoxFuture<'_, Result<(), CoreError>>;
}

// nebula-credential/src/contract/credential.rs — current, 4 assoc types
pub trait Credential: sealed::Sealed + Send + Sync + 'static {
    type Input: HasSchema;           // Config — что пользователь вводит
    type State: CredentialState;     // Encrypted at rest
    type Pending: PendingState;      // NoPendingState для static
    type Scheme: AuthScheme;         // Projection type

    const KEY: &'static str;
    // caps: 5 const bools (INTERACTIVE, REFRESHABLE, REVOCABLE, TESTABLE, SIGNING_CAPABLE)

    fn metadata() -> CredentialMetadata;
    fn project(state: &Self::State) -> Self::Scheme;
    async fn resolve(values: &FieldValues, ctx: &CredentialContext) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError>;
    async fn continue_resolve(...) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> { ... }
    async fn test(...) -> Result<Option<TestResult>, CredentialError> { Ok(None) }
    async fn refresh(...) -> Result<RefreshOutcome, CredentialError> { Ok(NotSupported) }
    async fn revoke(...) -> Result<(), CredentialError> { Ok(()) }
}
```

## Proposed delta (черновик — с holes)

### 1. `SchemeInjector` — отдельный trait

**Мотивация:** core's `AuthScheme` отвечает на "что это" (pattern + expires). Consumer'у нужно "как это применить" — inject в HTTP header, sign per-request, configure TLS, use in connection. Это другой concept.

```rust
// nebula-credential/src/scheme/injector.rs — NEW
pub trait SchemeInjector: nebula_core::AuthScheme + ZeroizeOnDrop {
    fn inject(&self, _req: &mut RequestParts) -> Result<(), InjectError> {
        Err(InjectError::NotApplicable)
    }

    fn sign(&self, req: &mut RequestParts, ctx: &SigningContext) -> Result<(), InjectError> {
        self.inject(req)  // fall through for static schemes
    }

    fn configure_tls(&self, _b: &mut TlsConfigBuilder) -> Result<(), InjectError> { Ok(()) }

    fn connection_descriptor(&self) -> Option<&ConnectionDescriptor> { None }
}
```

**Open question (#10):** Добавить `+ Send + Sync`? Для `Arc<dyn SchemeInjector>` через async boundary — нужны. Но `AuthScheme` уже `Send + Sync` → наследуется. Проверить когда compile-test.

**Open question (#11):** Как signing проходит через streaming body? `RequestParts` не содержит body. AWS SigV4 хеширует body (`UNSIGNED-PAYLOAD` опция существует для streaming, но не для всех кейсов). Неясно — мы buffer'им? Требуем buffered body? Split `sign_headers_only` / `sign_with_body`?

**Open question (#12):** `sign()` default fallback на `inject()` концептуально путает. Static scheme с sign_and_send — должен работать? Или требовать explicit `sign_only: bool`? Нужна дисциплина, возможно compile-time марку через separate capability trait.

**Open question (#13):** `InjectError::NotApplicable` — permanent programming error или transient config error? Вне `nebula_error::Classify` осей. Или через new axis `DomainErrorKind` с `Capability`/`Runtime`/`Config` категориями?

### 2. Capability marker traits

```rust
// Blanket impls — concrete scheme declares what it accepts
pub trait AcceptsBearer:        SchemeInjector {}
pub trait AcceptsSigning:       SchemeInjector {}
pub trait AcceptsTlsIdentity:   SchemeInjector {}
pub trait AcceptsDbConnection:  SchemeInjector {}
pub trait AcceptsKafkaAuth:     SchemeInjector {}

impl AcceptsBearer for OAuth2Token {}
impl AcceptsBearer for ApiKeyBearerScheme {}
impl AcceptsSigning for AwsSigV4Scheme {}
impl AcceptsTlsIdentity for MtlsScheme {}
impl AcceptsDbConnection for PostgresConnectionScheme {}
```

**Open question (#9):** Capability markers живут в `nebula-credential`. Но `nebula-resource` hace `type Auth: AcceptsBearer`. Значит resource-writer зависит от credential crate даже если он credential-agnostic по логике. Альтернатива — капабилити markers в `nebula-core` (но тогда всё знание про injection в core = scope creep). Текущая best guess — credential.

### 3. `ctx.credential::<C>()` — API shape

```rust
impl ActionContext {
    // Вариант A: type-driven, uniqueness-required
    pub async fn credential<C>(&self) -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where
        C: Credential + ?Sized,
    {
        // compile error at macro expansion if action has 0 or >1 slots of type C
        let key = self.bindings.unique_of::<C>()?;
        self.accessor.resolve::<C>(&key).await
    }

    // Вариант B: field-ref-driven, unambiguous
    pub async fn credential_at<C>(&self, binding: &CredentialRef<C>)
        -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where
        C: Credential + ?Sized,
    {
        self.accessor.resolve::<C>(&binding.key).await
    }
}
```

**Open question (#1):** Если у action'а два slot'а одного type (`slack_dev: CredentialRef<SlackOAuth2>`, `slack_prod: CredentialRef<SlackOAuth2>`), вариант A fails. Вариант B требует field-ref. Compromise: variant A как shorthand для unique case, variant B для ambiguous. Но это два API для одного concept — путает. Может быть лучше **только** variant B, а variant A зарезервировать для special случаев (single-slot actions).

**Proposal (нужна validation):** default API — `ctx.credential_at(&self.slack)` с field ref (variant B). Compile error если нет такого field. `ctx.credential::<C>()` — opt-in shorthand с `#[derive(Action)]` атрибутом `#[action(shorthand_access)]` который включает его только когда explicitly opt-in.

### 4. `CredentialRef<C>` — runtime shape

```rust
pub struct CredentialRef<C: ?Sized> {
    key: CredentialKey,           // runtime identifier
    _marker: PhantomData<fn() -> C>,  // contravariant, type-only at compile
}
```

**Open question (#4):** `CredentialRef<SlackOAuth2>` и `CredentialRef<dyn AcceptsBearer>` на wire **одинаковы** — оба содержат `CredentialKey`. Type parameter — только для compile-time enforcement. Runtime resolution полагается на registry от `TypeId` → supported capabilities.

**Open:** Registry от TypeId к capabilities — новая infra. Shape:
```rust
pub struct CredentialTypeRegistry {
    types: HashMap<TypeId, RegisteredType>,
}

pub struct RegisteredType {
    key: &'static str,
    metadata: CredentialMetadata,
    capabilities: Capabilities,  // bitflags
    can_project_to: &'static [TypeId],  // Scheme type + capability markers
    builder: Box<dyn Fn() -> Box<dyn AnyCredential>>,
}
```

Populated при `plugin_registry::register::<C>()` в engine startup. Runtime check в resolve: "does stored credential type satisfy C's trait bounds?" If C is `dyn AcceptsBearer`, lookup stored type's capabilities.

**Open:** Не ясно, как macro `#[derive(Credential)]` emit'ит registration call. Инжекция через inventory crate (collect!) или explicit call. Inventory может не работать cross-crate.

### 5. Credential trait — dyn-safety

**Проблема (#3):** `Credential` имеет 4 assoc types (Config/State/Pending/Scheme). Trait object `dyn Credential` требует либо все assoc types unbound (не compile), либо projection через specific types (`dyn Credential<Config=..., State=..., Pending=..., Scheme=...>`) — громоздко.

**Existing solution:** `AnyCredential` object-safe supertrait — exposes только данные (metadata, key, caps), не методы с associated types. `Box<dyn AnyCredential>` носит `TypeId` через `Any`.

```rust
pub trait AnyCredential: Any + Send + Sync + 'static {
    fn credential_key(&self) -> &CredentialKey;
    fn metadata(&self) -> &CredentialMetadata;
    fn capabilities(&self) -> Capabilities;
    fn type_id(&self) -> TypeId;
}

impl<C: Credential> AnyCredential for C { ... }
```

**Open question (#32):** `CredentialGuard<C::Scheme>` для `C = dyn SomeServiceCredential` — `C::Scheme` это associated type на dyn trait. В Rust это requires `type Scheme = dyn AcceptsBearer` bound в trait declaration, не `Scheme: AcceptsBearer`. Разница семантическая: bound позволяет каждому concrete impl's Scheme быть different AcceptsBearer type; type-alias forces one specific type.

Для service traits вроде `BitbucketCredential` — вероятно `type Scheme = BearerCredentialScheme` (сведённый type) корректен. Но теряем variance.

Это **нужно тестировать в prototype** — я не могу на бумаге решить, работает ли dyn service trait projection.

### 6. Service trait pattern (для multi-auth services)

```rust
// Sealed trait для service аккаунтов с multiple auth methods
pub trait BitbucketCredential: Credential + sealed::Sealed {
    // Each impl must project to BearerCredentialScheme (or similar)
    // Unsure if this form is correct — see #32
}

pub struct BitbucketOAuth2Credential;
pub struct BitbucketAppPasswordCredential;
pub struct BitbucketPatCredential;

impl BitbucketCredential for BitbucketOAuth2Credential {}
impl BitbucketCredential for BitbucketAppPasswordCredential {}
impl BitbucketCredential for BitbucketPatCredential {}
```

**Open question (#8):** `sealed::Sealed` vs plugin extensibility. Если плагин хочет добавить `BitbucketCustomOAuth3Credential` — он не может, потому что `Sealed` — crate-internal. Это полный contradiction если мы claim'им 400+ third-party plugins.

**Possible resolutions (нужны prototypes):**
1. Sealed на `Credential` (core), открытое на service traits (`BitbucketCredential`) — но тогда core sealing не защищает от кастом invariants.
2. Sealed с `UnsafeCell::seal_extension` escape hatch для builtins и plugins-with-signed-manifest.
3. Not sealed вообще — replace sealing с `#[non_exhaustive]` + trait method invariants + CI test that implementors satisfy contract.

### 7. Pattern 1 vs Pattern 2 — **Pattern 2 is default**

После user's review (finding #5): большинство популярных services — multi-auth (Bitbucket, Jira, Shopify, GitHub, Slack, Stripe, HubSpot, Notion, Salesforce, ServiceNow, Airtable, Twitter). Pattern 1 (per-service concrete type) — для minority cases: service с единственным auth method (Cloudflare, OpenAI, Anthropic, etc.).

**Revised default:**

```rust
// ── Pattern 2 (DEFAULT) — service trait, multiple auth methods ─────
pub trait SlackCredential: Credential + sealed::Sealed {
    // different impls: SlackOAuth2, SlackBotToken, SlackUserToken, SlackApiToken
}

#[action(credential)]
pub slack: CredentialRef<dyn SlackCredential>,

// usage:
let auth = ctx.credential_at(&self.slack).await?;
// auth: CredentialGuard<Bearer>  — dyn resolved at runtime

// ── Pattern 1 (MINORITY) — single concrete type ─────────────────────
#[action(credential)]
pub anthropic: CredentialRef<AnthropicApiKeyCredential>,

let auth = ctx.credential_at(&self.anthropic).await?;
// auth: CredentialGuard<Bearer>  — static

// ── Pattern 3 (GENERIC fallback) — service-agnostic ─────────────────
#[action(credential, accepts = "dyn AcceptsBearer")]
pub any_bearer: CredentialRef<dyn AcceptsBearer>,
```

**Open question (#6):** Service trait + ProviderRegistry dublicate provider info. `SlackCredential` impls know provider "slack"; ProviderRegistry also knows. Resolution: service trait carries ProviderId as constant, validates at activation:
```rust
pub trait SlackCredential: Credential + sealed::Sealed {
    const PROVIDER_ID: &'static str = "slack";  // must match registry
}
```
Activation-time check: `ProviderRegistry::get(Self::PROVIDER_ID)` must exist. If operator removes "slack" from registry — existing credentials fail loudly on next resolve.

## Capabilities — bitflags

```rust
bitflags::bitflags! {
    pub struct Capabilities: u16 {
        const STATIC        = 1 << 0;
        const INTERACTIVE   = 1 << 1;
        const REFRESHABLE   = 1 << 2;
        const REVOCABLE     = 1 << 3;
        const TESTABLE      = 1 << 4;
        const SIGNING       = 1 << 5;
        const CONNECTION    = 1 << 6;
        const TLS_IDENTITY  = 1 << 7;
        const COMPOSED      = 1 << 8;   // references another credential
        const TEMPORARY     = 1 << 9;   // derived short-lived (STS)
        const NO_SECRET     = 1 << 10;  // webhook URL
        const MULTI_STEP    = 1 << 11;  // N-step beyond OAuth2
    }
}

impl Credential for SlackOAuth2Credential {
    const CAPS: Capabilities = Capabilities::INTERACTIVE
        .union(Capabilities::REFRESHABLE)
        .union(Capabilities::REVOCABLE)
        .union(Capabilities::TESTABLE);
    ...
}
```

Replaces existing 5 const bool fields (INTERACTIVE, REFRESHABLE, REVOCABLE, TESTABLE, SIGNING_CAPABLE). Compile-time const ops.

**Benefit:** `caps.contains(Capabilities::REFRESHABLE)` — one check. Иnstead of checking 5 bools separately.

## ExternalProvider — typed resolve + tenant scoping

```rust
pub trait ExternalProvider: Send + Sync + 'static {
    async fn resolve<S: AuthScheme + DeserializeFromProvider>(
        &self,
        tenant_ctx: &TenantContext,
        reference: &ExternalReference,  // user-supplied SUFFIX
    ) -> Result<S, ProviderError>;

    fn pattern_support(&self) -> &[AuthPattern];
    fn endpoint_allowlist(&self) -> &EndpointAllowlist;  // literal URL match
}

pub trait DeserializeFromProvider: Sized {
    fn deserialize_from(bytes: &[u8], format: ProviderFormat) -> Result<Self, DecodeError>;
}
```

**Open question (#31):** `DeserializeFromProvider` требует знать `ProviderFormat` (Vault JSON, AWS SM plaintext, KMS raw bytes, Azure KV wrapped). Каждый provider returns different shape. Альтернатива — provider-specific decoder trait: `S: DeserializeFromVault + DeserializeFromAwsSm + ...`. Это N-to-M matrix.

**Possible path:** two-stage: provider returns `RawProviderOutput { bytes: Vec<u8>, metadata: HashMap<String, Value> }`, each `S` implements `TryFrom<&RawProviderOutput>`. Explicit, no format coupling.

**Open question (#14):** Tenant scoping — `resolve(tenant_ctx, reference)`. Provider impl prefixes tenant namespace:
```rust
impl ExternalProvider for VaultProvider {
    async fn resolve<S: ...>(&self, tenant: &TenantContext, r: &ExternalReference) -> ... {
        let path = format!("/secret/tenants/{}/{}", tenant.tenant_id, r.suffix());
        // tenant-scoped Vault token
        let raw = self.client.read_with_token(&path, &self.token_for_tenant(tenant).await?).await?;
        S::try_from(&raw).map_err(ProviderError::Decode)
    }
}
```

But **open:** operator compromise / misconfig — if operator writes `allowed_paths = ["/secret/**"]` in ProviderSpec, the tenant-scoping is moot. Need trait-level invariant enforcement, not just impl discipline.

## Что pattern_support не покрывает (#30)

```rust
fn pattern_support(&self) -> &[AuthPattern];
// AuthPattern::Custom variant — plugin-defined patterns не знаем at static time
```

Plugin с Custom scheme — provider не может static-ly declare support. Need runtime-extensible mechanism. Possible:

```rust
fn pattern_support(&self) -> PatternSupport;

pub enum PatternSupport {
    Static(&'static [AuthPattern]),
    Dynamic(Box<dyn Fn(&AuthPattern) -> bool>),  // runtime check per-pattern
}
```

Overhead acceptable — pattern support check is rare.

## Summary — what prototype must validate

1. **dyn service trait projection** — does `CredentialGuard<C::Scheme>` compile for `C = dyn SlackCredential`? (finding #32)
2. **ctx.credential::<C>() ambiguity** — can compile-time uniqueness check в proc macro be done reliably? (finding #1)
3. **Service trait + Sealed + plugin** — does trait stay sealed but allow plugin extension through `#[plugin_credential]` macro which has escape hatch? (finding #8)
4. **Capability marker dispatch** — `dyn AcceptsBearer` resolve at runtime with registry lookup — actually works? (finding #4)
5. **SchemeInjector bounds** — `Send + Sync + ZeroizeOnDrop` — все compile-сable? (finding #10)
6. **4 assoc types через `dyn Credential<Config=_, ...>`** — feasible? Or just AnyCredential as the dyn path? (finding #3)

Все это testable только в prototype. Writing spec до prototype — гарантированный churn.
