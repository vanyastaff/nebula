//! Nebula Credential - Universal credential management system
//!
//! A secure, extensible credential management system for workflow automation.
//!
//! # Features
//!
//! - **Protocol-agnostic flows** - `OAuth2`, API Keys, JWT, SAML, Kerberos, mTLS
//! - **Type-safe credentials** - Compile-time verification with generic flows
//! - **Interactive authentication** - Multi-step flows with user interaction
//! - **Secure storage** - Zero-copy secrets with automatic zeroization
//! - **Minimal boilerplate** - ~30-50 lines to add new integrations
#![deny(unsafe_code)]
#![forbid(unsafe_code)]

/// Core types, errors, and primitives
pub mod core;
/// Built-in credential flows (OAuth2, API Key, etc.)
// TODO: Phase 5 - Re-enable flows after updating to new error API
// pub mod flows;
/// Storage provider implementations
pub mod providers;
/// Core traits for credentials, storage, and locking
pub mod traits;
/// Utilities for crypto, time, etc.
pub mod utils;

/// Commonly used types and traits
pub mod prelude {
    // Core types
    pub use crate::core::{
        CredentialContext, CredentialError, CredentialFilter, CredentialId, CredentialMetadata,
        SecretString,
    };

    // TODO: Phase 5 - Re-enable after updating flows to new error API
    // pub use crate::core::{
    //     CredentialState,
    //     adapter::FlowCredential,
    //     result::{
    //         CaptchaType, CodeFormat, CredentialFlow, DisplayData, InitializeResult,
    //         InteractionRequest, PartialState, UserInput,
    //     },
    // };

    // TODO: Phase 5 - Re-enable built-in flows
    // pub use crate::flows::{
    //     api_key::{ApiKeyCredential, ApiKeyFlow, ApiKeyInput, ApiKeyState},
    //     basic_auth::{BasicAuthCredential, BasicAuthFlow, BasicAuthInput, BasicAuthState},
    //     bearer_token::{
    //         BearerTokenCredential, BearerTokenFlow, BearerTokenInput, BearerTokenState,
    //     },
    //     oauth2::{
    //         AuthorizationCodeFlow, AuthorizationCodeInput, ClientCredentialsFlow,
    //         ClientCredentialsInput, OAuth2AuthorizationCode, OAuth2ClientCredentials, OAuth2State,
    //     },
    //     password::{PasswordCredential, PasswordFlow, PasswordInput, PasswordState},
    // };

    // Traits
    pub use crate::traits::{
        // Credential, InteractiveCredential,
        DistributedLock,
        LockError,
        LockGuard,
        StateStore,
        StorageProvider,
    };

    // Utils - crypto functions
    pub use crate::utils::{EncryptedData, EncryptionKey, decrypt, encrypt};

    // TODO: Phase 5 - Re-enable OAuth2 utils
    // pub use crate::utils::{
    //     generate_code_challenge, generate_pkce_verifier, generate_random_state,
    // };

    // Storage providers (Phase 2)
    pub use crate::providers::{MockStorageProvider, ProviderConfig, StorageMetrics};

    #[cfg(feature = "storage-local")]
    pub use crate::providers::{LocalStorageConfig, LocalStorageProvider};

    #[cfg(feature = "storage-aws")]
    pub use crate::providers::{AwsSecretsManagerConfig, AwsSecretsManagerProvider};

    // Azure provider skipped - SDK issues
    // #[cfg(feature = "storage-azure")]
    // pub use crate::providers::{AzureCredentialType, AzureKeyVaultConfig, AzureKeyVaultProvider};

    #[cfg(feature = "storage-vault")]
    pub use crate::providers::{HashiCorpVaultProvider, VaultAuthMethod, VaultConfig};

    #[cfg(feature = "storage-k8s")]
    pub use crate::providers::{KubernetesSecretsConfig, KubernetesSecretsProvider};

    // Retry utilities
    pub use crate::utils::RetryPolicy;
}
