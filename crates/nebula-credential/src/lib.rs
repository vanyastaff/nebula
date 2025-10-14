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
pub mod flows;
/// Core traits for credentials, storage, and locking
pub mod traits;
/// Utilities for crypto, time, etc.
pub mod utils;

/// Commonly used types and traits
pub mod prelude {
    // Core types
    pub use crate::core::{
        CredentialContext, CredentialError, CredentialId, CredentialMetadata, CredentialState,
        SecureString,
        adapter::FlowCredential,
        result::{
            CaptchaType, CodeFormat, CredentialFlow, DisplayData, InitializeResult,
            InteractionRequest, PartialState, UserInput,
        },
    };

    // Built-in flows
    pub use crate::flows::{
        api_key::{ApiKeyCredential, ApiKeyFlow, ApiKeyInput, ApiKeyState},
        basic_auth::{BasicAuthCredential, BasicAuthFlow, BasicAuthInput, BasicAuthState},
        bearer_token::{
            BearerTokenCredential, BearerTokenFlow, BearerTokenInput, BearerTokenState,
        },
        oauth2::{
            AuthorizationCodeFlow, AuthorizationCodeInput, ClientCredentialsFlow,
            ClientCredentialsInput, OAuth2AuthorizationCode, OAuth2ClientCredentials, OAuth2State,
        },
        password::{PasswordCredential, PasswordFlow, PasswordInput, PasswordState},
    };

    // Traits
    pub use crate::traits::{
        Credential, DistributedLock, InteractiveCredential, LockError, LockGuard, StateStore,
    };

    // Utils
    pub use crate::utils::{
        generate_code_challenge, generate_pkce_verifier, generate_random_state,
    };
}
