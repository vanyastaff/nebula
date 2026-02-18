extern crate self as nebula_action;
extern crate self as nebula_credential;
extern crate self as nebula_parameter;
extern crate self as nebula_plugin;
extern crate self as nebula_resource;

pub mod capability {
    #[derive(Clone, Copy)]
    pub enum IsolationLevel {
        None,
        Sandbox,
        Process,
        Vm,
    }
}

pub mod metadata {
    use crate::capability::IsolationLevel;

    #[derive(Clone, Copy)]
    pub enum ActionType {
        Process,
        Stateful,
        Trigger,
        Streaming,
        Transactional,
        Interactive,
    }

    #[derive(Clone)]
    pub struct ActionMetadata {
        pub action_type: ActionType,
        pub isolation_level: IsolationLevel,
        pub credential: Option<String>,
    }

    impl ActionMetadata {
        pub fn new(_key: &str, _name: &str, _description: &str) -> Self {
            Self {
                action_type: ActionType::Process,
                isolation_level: IsolationLevel::None,
                credential: None,
            }
        }

        pub fn with_version(self, _major: u32, _minor: u32) -> Self {
            self
        }

        pub fn with_action_type(mut self, action_type: ActionType) -> Self {
            self.action_type = action_type;
            self
        }

        pub fn with_isolation(mut self, isolation_level: IsolationLevel) -> Self {
            self.isolation_level = isolation_level;
            self
        }

        pub fn with_credential(mut self, credential: &str) -> Self {
            self.credential = Some(credential.to_string());
            self
        }
    }
}

pub trait Action: Send + Sync + 'static {
    fn metadata(&self) -> &crate::metadata::ActionMetadata;
}

pub struct PluginComponents;

#[derive(Clone)]
pub struct PluginMetadata {
    _name: String,
}

pub struct PluginMetadataBuilder {
    _name: String,
}

impl PluginMetadata {
    pub fn builder(_key: &str, name: &str) -> PluginMetadataBuilder {
        PluginMetadataBuilder {
            _name: name.to_string(),
        }
    }
}

impl PluginMetadataBuilder {
    pub fn description(self, _description: &str) -> Self {
        self
    }

    pub fn version(self, _version: u32) -> Self {
        self
    }

    pub fn group(self, _group: Vec<String>) -> Self {
        self
    }

    pub fn build(self) -> Result<PluginMetadata, &'static str> {
        Ok(PluginMetadata { _name: self._name })
    }
}

pub trait Plugin: Send + Sync + 'static {
    fn metadata(&self) -> &PluginMetadata;
    fn register(&self, components: &mut PluginComponents);
}

pub mod context {
    pub struct Context;
}

pub mod error {
    pub type Result<T> = core::result::Result<T, String>;
}

pub trait Resource: Send + Sync + 'static {
    type Config;
    type Instance: Send + Sync + 'static;

    fn id(&self) -> &str;

    fn create(
        &self,
        config: &Self::Config,
        ctx: &crate::context::Context,
    ) -> impl ::std::future::Future<Output = crate::error::Result<Self::Instance>> + Send;

    fn is_valid(
        &self,
        instance: &Self::Instance,
    ) -> impl ::std::future::Future<Output = crate::error::Result<bool>> + Send;

    fn recycle(
        &self,
        instance: &mut Self::Instance,
    ) -> impl ::std::future::Future<Output = crate::error::Result<()>> + Send;

    fn cleanup(
        &self,
        instance: Self::Instance,
    ) -> impl ::std::future::Future<Output = crate::error::Result<()>> + Send;
}

pub mod core {
    #[derive(Clone)]
    pub struct CredentialDescription {
        pub key: String,
        pub name: String,
        pub description: String,
        pub icon: Option<String>,
        pub icon_url: Option<String>,
        pub documentation_url: Option<String>,
        pub properties: crate::collection::ParameterCollection,
    }

    pub struct CredentialContext;
    pub struct CredentialError;

    pub mod result {
        pub enum InitializeResult<T> {
            Complete(T),
        }
    }
}

#[::async_trait::async_trait]
pub trait CredentialType: Send + Sync + 'static {
    type Input: Send + Sync + 'static;
    type State: Send + Sync + Clone + 'static;

    fn description() -> crate::core::CredentialDescription
    where
        Self: Sized;

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut crate::core::CredentialContext,
    ) -> Result<crate::core::result::InitializeResult<Self::State>, crate::core::CredentialError>;
}

pub trait StaticProtocol: Send + Sync + 'static {
    type State: Send + Sync + Clone + 'static;

    fn parameters() -> crate::collection::ParameterCollection
    where
        Self: Sized;

    fn build_state(
        values: &crate::values::ParameterValues,
    ) -> Result<Self::State, crate::core::CredentialError>
    where
        Self: Sized;
}

pub mod traits {
    pub use crate::CredentialType;
    pub use crate::FlowProtocol;
    pub use crate::StaticProtocol;
}

// ── FlowProtocol stub ──────────────────────────────────────────────────────

#[allow(async_fn_in_trait)]
pub trait FlowProtocol: Send + Sync + 'static {
    type Config: Send + Sync + 'static;
    type State: Send + Sync + Clone + 'static;

    fn parameters() -> crate::collection::ParameterCollection
    where
        Self: Sized;

    async fn initialize(
        config: &Self::Config,
        values: &crate::values::ParameterValues,
        ctx: &mut crate::core::CredentialContext,
    ) -> Result<crate::core::result::InitializeResult<Self::State>, crate::core::CredentialError>
    where
        Self: Sized;

    async fn refresh(
        _config: &Self::Config,
        _state: &mut Self::State,
        _ctx: &mut crate::core::CredentialContext,
    ) -> Result<(), crate::core::CredentialError>
    where
        Self: Sized,
    {
        Ok(())
    }
}

// ── OAuth2 stub types ──────────────────────────────────────────────────────

pub mod protocols {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub enum AuthStyle {
        #[default]
        Header,
        PostBody,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub enum GrantType {
        #[default]
        AuthorizationCode,
        ClientCredentials,
        DeviceCode,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OAuth2Config {
        pub auth_url: String,
        pub token_url: String,
        pub scopes: Vec<String>,
        pub grant_type: GrantType,
        pub auth_style: AuthStyle,
        pub pkce: bool,
    }

    impl OAuth2Config {
        pub fn authorization_code() -> OAuth2ConfigBuilder {
            OAuth2ConfigBuilder {
                grant_type: GrantType::AuthorizationCode,
                auth_url: String::new(),
                token_url: String::new(),
                scopes: Vec::new(),
                auth_style: AuthStyle::Header,
                pkce: false,
            }
        }
    }

    pub struct OAuth2ConfigBuilder {
        pub grant_type: GrantType,
        pub auth_url: String,
        pub token_url: String,
        pub scopes: Vec<String>,
        pub auth_style: AuthStyle,
        pub pkce: bool,
    }

    impl OAuth2ConfigBuilder {
        pub fn auth_url(mut self, url: impl Into<String>) -> Self {
            self.auth_url = url.into();
            self
        }
        pub fn token_url(mut self, url: impl Into<String>) -> Self {
            self.token_url = url.into();
            self
        }
        pub fn scopes(mut self, s: Vec<String>) -> Self {
            self.scopes = s;
            self
        }
        pub fn auth_style(mut self, style: AuthStyle) -> Self {
            self.auth_style = style;
            self
        }
        pub fn pkce(mut self, p: bool) -> Self {
            self.pkce = p;
            self
        }
        pub fn build(self) -> OAuth2Config {
            OAuth2Config {
                auth_url: self.auth_url,
                token_url: self.token_url,
                scopes: self.scopes,
                grant_type: self.grant_type,
                auth_style: self.auth_style,
                pkce: self.pkce,
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OAuth2State {
        pub access_token: String,
    }

    pub struct OAuth2Protocol;

    impl crate::FlowProtocol for OAuth2Protocol {
        type Config = OAuth2Config;
        type State = OAuth2State;

        fn parameters() -> crate::collection::ParameterCollection {
            crate::collection::ParameterCollection::new()
        }

        async fn initialize(
            _config: &OAuth2Config,
            _values: &crate::values::ParameterValues,
            _ctx: &mut crate::core::CredentialContext,
        ) -> Result<crate::core::result::InitializeResult<OAuth2State>, crate::core::CredentialError>
        {
            Ok(crate::core::result::InitializeResult::Complete(
                OAuth2State {
                    access_token: String::new(),
                },
            ))
        }
    }

    // ── LDAP stub types ────────────────────────────────────────────────────

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    pub enum TlsMode {
        #[default]
        None,
        Tls,
        StartTls,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LdapConfig {
        pub tls: TlsMode,
        pub timeout: std::time::Duration,
        pub ca_cert: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LdapState {
        pub host: String,
        pub port: u16,
        pub bind_dn: String,
        pub bind_password: String,
        pub tls: TlsMode,
    }

    pub struct LdapProtocol;

    impl crate::FlowProtocol for LdapProtocol {
        type Config = LdapConfig;
        type State = LdapState;

        fn parameters() -> crate::collection::ParameterCollection {
            crate::collection::ParameterCollection::new()
        }

        async fn initialize(
            _config: &LdapConfig,
            _values: &crate::values::ParameterValues,
            _ctx: &mut crate::core::CredentialContext,
        ) -> Result<crate::core::result::InitializeResult<LdapState>, crate::core::CredentialError>
        {
            Ok(crate::core::result::InitializeResult::Complete(LdapState {
                host: String::new(),
                port: 389,
                bind_dn: String::new(),
                bind_password: String::new(),
                tls: TlsMode::None,
            }))
        }
    }
}

pub mod values {
    #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct ParameterValues {
        #[serde(flatten)]
        values: std::collections::HashMap<String, serde_json::Value>,
    }

    impl ParameterValues {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn get_string(&self, key: &str) -> Option<&str> {
            self.values.get(key)?.as_str()
        }
        pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
            self.values.insert(key.into(), value);
        }
    }
}

pub mod collection {
    #[derive(Clone, Default)]
    pub struct ParameterCollection {
        pub(crate) items: Vec<crate::def::ParameterDef>,
    }

    impl ParameterCollection {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with(mut self, item: crate::def::ParameterDef) -> Self {
            self.items.push(item);
            self
        }
    }
}

pub mod def {
    #[derive(Clone)]
    pub enum ParameterDef {
        Text(crate::types::TextParameter),
        Number(crate::types::NumberParameter),
        Checkbox(crate::types::CheckboxParameter),
        Secret(crate::types::SecretParameter),
    }
}

pub mod types {
    #[derive(Clone, Default)]
    pub struct Metadata {
        pub description: Option<String>,
        pub required: bool,
    }

    #[derive(Clone)]
    pub struct TextParameter {
        pub metadata: Metadata,
        pub default: Option<String>,
    }

    #[derive(Clone)]
    pub struct NumberParameter {
        pub metadata: Metadata,
        pub default: Option<f64>,
    }

    #[derive(Clone)]
    pub struct CheckboxParameter {
        pub metadata: Metadata,
        pub default: Option<bool>,
    }

    #[derive(Clone)]
    pub struct SecretParameter {
        pub metadata: Metadata,
        pub default: Option<String>,
    }

    impl TextParameter {
        pub fn new(_key: &str, _name: &str) -> Self {
            Self {
                metadata: Metadata::default(),
                default: None,
            }
        }
    }

    impl NumberParameter {
        pub fn new(_key: &str, _name: &str) -> Self {
            Self {
                metadata: Metadata::default(),
                default: None,
            }
        }
    }

    impl CheckboxParameter {
        pub fn new(_key: &str, _name: &str) -> Self {
            Self {
                metadata: Metadata::default(),
                default: None,
            }
        }
    }

    impl SecretParameter {
        pub fn new(_key: &str, _name: &str) -> Self {
            Self {
                metadata: Metadata::default(),
                default: None,
            }
        }
    }
}
