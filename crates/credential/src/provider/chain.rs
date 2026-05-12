//! Composable provider chain with error-discriminated fallback.
//!
//! Mirrors `CredentialsProviderChain::or_else` from `aws-config`
//! (`aws-config/src/meta/credentials/chain.rs`): a typed struct that wraps a
//! list of providers and dispatches them in order, but with **discriminated
//! fallback** — only [`ProviderError::NotFound`] triggers the next provider,
//! every other error short-circuits the chain. This prevents a misconfigured
//! later provider from masking an `Unavailable` or `AccessDenied` from an
//! earlier one.
//!
//! The chain itself implements [`ExternalProvider`], so it composes with the
//! same dispatch surface as any single provider (Liskov).

use std::{borrow::Cow, sync::Arc};

use tracing::Instrument;

use super::{ExternalProvider, ExternalReference, ProviderError, ProviderFuture};

/// Ordered composition of [`ExternalProvider`] instances with discriminated
/// fallback.
///
/// # Dispatch rules
///
/// For each provider in order:
/// - `Ok(_)` — return immediately.
/// - `Err(ProviderError::NotFound { .. })` — log at `debug`, try the next provider.
/// - Any other `Err(_)` — return immediately (do **not** mask under a later provider).
///
/// If every provider returns `NotFound`, the chain returns
/// `ProviderError::NotFound` with the original reference path.
///
/// # Examples
///
/// ```ignore
/// use std::sync::Arc;
/// use nebula_credential::{ExternalProvider, ExternalProviderChain};
///
/// // `first_try` / `or_else` take `Arc<dyn ExternalProvider>` — wrap concrete
/// // provider values explicitly to share them across the chain (and across
/// // multiple chains, if needed).
/// let chain = ExternalProviderChain::first_try(
///         "env",
///         Arc::new(env_provider) as Arc<dyn ExternalProvider>,
///     )
///     .or_else("vault", Arc::new(vault_provider) as Arc<dyn ExternalProvider>)
///     .or_else("aws_sm", Arc::new(aws_sm_provider) as Arc<dyn ExternalProvider>);
///
/// // `chain` is itself an `ExternalProvider`; can be registered or further nested.
/// ```
#[derive(Clone)]
pub struct ExternalProviderChain {
    providers: Vec<(Cow<'static, str>, Arc<dyn ExternalProvider>)>,
}

impl std::fmt::Debug for ExternalProviderChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.providers.iter().map(|(n, _)| n.as_ref()).collect();
        f.debug_struct("ExternalProviderChain")
            .field("providers", &names)
            .finish()
    }
}

impl ExternalProviderChain {
    /// Start a new chain with a single first provider.
    #[must_use]
    pub fn first_try(
        name: impl Into<Cow<'static, str>>,
        provider: Arc<dyn ExternalProvider>,
    ) -> Self {
        Self {
            providers: vec![(name.into(), provider)],
        }
    }

    /// Append a fallback provider. Builder-style.
    #[must_use]
    pub fn or_else(
        mut self,
        name: impl Into<Cow<'static, str>>,
        provider: Arc<dyn ExternalProvider>,
    ) -> Self {
        self.providers.push((name.into(), provider));
        self
    }

    /// Number of providers in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// `true` if the chain has no providers.
    ///
    /// The public builder API ([`first_try`](Self::first_try) +
    /// [`or_else`](Self::or_else)) always yields a non-empty chain (`len() >=
    /// 1`); this predicate exists so internal callers (e.g. unit tests that
    /// reach into [`Self`] via mutable patterns) and future API surface that
    /// may permit an explicitly-empty chain can guard the dispatch loop. The
    /// `providers` field is private, so downstream crates cannot construct
    /// an empty chain through the public API today.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl ExternalProvider for ExternalProviderChain {
    fn resolve<'a>(&'a self, reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::new(async move {
            for (name, provider) in &self.providers {
                let span = tracing::debug_span!(
                    "provider_chain",
                    provider = %name,
                    path = %reference.path,
                );
                let result = provider.resolve(reference).instrument(span.clone()).await;
                match result {
                    Ok(resolution) => return Ok(resolution),
                    Err(ProviderError::NotFound { .. }) => {
                        tracing::debug!(
                            parent: &span,
                            "provider returned NotFound; falling through to next"
                        );
                        continue;
                    },
                    Err(err) => {
                        tracing::debug!(
                            parent: &span,
                            error = %err,
                            "provider returned hard error; short-circuiting chain"
                        );
                        return Err(err);
                    },
                }
            }
            Err(ProviderError::NotFound {
                path: reference.path.clone(),
            })
        })
    }

    fn provider_name(&self) -> &'static str {
        "chain"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{SecretString, provider::ProviderResolution};

    /// Test double: resolves with a preconfigured outcome.
    #[derive(Debug)]
    struct MockProvider {
        name: &'static str,
        outcome: Result<&'static str, ProviderError>,
    }

    impl ExternalProvider for MockProvider {
        fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
            match &self.outcome {
                Ok(s) => ProviderFuture::ready(Ok(ProviderResolution::from_secret(
                    SecretString::new(*s),
                ))),
                Err(e) => ProviderFuture::ready(Err(clone_provider_error(e))),
            }
        }

        fn provider_name(&self) -> &str {
            self.name
        }
    }

    fn clone_provider_error(e: &ProviderError) -> ProviderError {
        match e {
            ProviderError::NotFound { path } => ProviderError::NotFound { path: path.clone() },
            ProviderError::Unavailable { reason } => ProviderError::Unavailable {
                reason: reason.clone(),
            },
            ProviderError::AccessDenied { reason } => ProviderError::AccessDenied {
                reason: reason.clone(),
            },
            ProviderError::Backend(_) => ProviderError::Unavailable {
                reason: "backend (cloned for test)".to_owned(),
            },
        }
    }

    fn refer() -> ExternalReference {
        ExternalReference {
            provider: crate::provider::ProviderKind::Custom("test".to_owned()),
            path: "secret/path".to_owned(),
            version: None,
            field: None,
        }
    }

    #[tokio::test]
    async fn first_ok_returns_immediately() {
        let chain = ExternalProviderChain::first_try(
            "primary",
            Arc::new(MockProvider {
                name: "primary",
                outcome: Ok("from-primary"),
            }),
        )
        .or_else(
            "fallback",
            Arc::new(MockProvider {
                name: "fallback",
                outcome: Ok("from-fallback"),
            }),
        );

        let r = chain.resolve(&refer()).await.unwrap();
        assert_eq!(r.secret.expose_secret(), "from-primary");
    }

    #[tokio::test]
    async fn not_found_falls_through() {
        let chain = ExternalProviderChain::first_try(
            "primary",
            Arc::new(MockProvider {
                name: "primary",
                outcome: Err(ProviderError::NotFound {
                    path: "secret/path".to_owned(),
                }),
            }),
        )
        .or_else(
            "fallback",
            Arc::new(MockProvider {
                name: "fallback",
                outcome: Ok("from-fallback"),
            }),
        );

        let r = chain.resolve(&refer()).await.unwrap();
        assert_eq!(r.secret.expose_secret(), "from-fallback");
    }

    #[tokio::test]
    async fn unavailable_short_circuits() {
        let chain = ExternalProviderChain::first_try(
            "primary",
            Arc::new(MockProvider {
                name: "primary",
                outcome: Err(ProviderError::Unavailable {
                    reason: "network down".to_owned(),
                }),
            }),
        )
        .or_else(
            "fallback",
            Arc::new(MockProvider {
                name: "fallback",
                outcome: Ok("from-fallback"),
            }),
        );

        let err = chain.resolve(&refer()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn access_denied_short_circuits() {
        let chain = ExternalProviderChain::first_try(
            "primary",
            Arc::new(MockProvider {
                name: "primary",
                outcome: Err(ProviderError::AccessDenied {
                    reason: "forbidden".to_owned(),
                }),
            }),
        )
        .or_else(
            "fallback",
            Arc::new(MockProvider {
                name: "fallback",
                outcome: Ok("from-fallback"),
            }),
        );

        let err = chain.resolve(&refer()).await.unwrap_err();
        assert!(matches!(err, ProviderError::AccessDenied { .. }));
    }

    #[tokio::test]
    async fn all_not_found_returns_not_found() {
        let chain = ExternalProviderChain::first_try(
            "a",
            Arc::new(MockProvider {
                name: "a",
                outcome: Err(ProviderError::NotFound {
                    path: "secret/path".to_owned(),
                }),
            }),
        )
        .or_else(
            "b",
            Arc::new(MockProvider {
                name: "b",
                outcome: Err(ProviderError::NotFound {
                    path: "secret/path".to_owned(),
                }),
            }),
        );

        let err = chain.resolve(&refer()).await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound { .. }));
    }

    #[tokio::test]
    async fn chain_is_itself_a_provider() {
        // Liskov: nested chains compose.
        let inner = ExternalProviderChain::first_try(
            "inner",
            Arc::new(MockProvider {
                name: "inner",
                outcome: Ok("nested"),
            }),
        );
        let outer = ExternalProviderChain::first_try("outer", Arc::new(inner));

        let r = outer.resolve(&refer()).await.unwrap();
        assert_eq!(r.secret.expose_secret(), "nested");
    }
}
