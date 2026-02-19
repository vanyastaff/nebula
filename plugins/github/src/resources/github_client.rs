//! GitHub client resource backed by [`octocrab`].
//!
//! Implements [`Resource`] so the client can be pooled and managed by
//! `nebula-resource`. Both authentication styles are supported:
//!
//! - **Personal Access Token** (from [`GithubApi`](crate::GithubApi))
//! - **OAuth2 access token** (from [`GithubOauth2`](crate::GithubOauth2))
//!
//! The two credential types both produce a plain bearer token, so
//! [`GithubClientConfig`] accepts a single `token` field regardless of origin.

use nebula_resource::context::Context;
use nebula_resource::error::{Error, FieldViolation, Result};
use nebula_resource::resource::{Config, Resource};
use octocrab::Octocrab;

const RESOURCE_ID: &str = "github-client";

/// Configuration for [`GithubClientResource`].
#[derive(Clone)]
pub struct GithubClientConfig {
    /// Bearer token — either a PAT (`ghp_…`) or an OAuth2 access token (`gho_…`).
    pub token: String,
    /// Base URL of the GitHub API. Defaults to `https://api.github.com`.
    /// Override for GitHub Enterprise Server instances.
    pub base_url: Option<String>,
}

impl Config for GithubClientConfig {
    fn validate(&self) -> Result<()> {
        if self.token.is_empty() {
            return Err(Error::validation(vec![FieldViolation::new(
                "token",
                "must not be empty",
                "(empty)",
            )]));
        }
        Ok(())
    }
}

/// Resource that produces an authenticated [`Octocrab`] client.
pub struct GithubClientResource;

impl Resource for GithubClientResource {
    type Config = GithubClientConfig;
    type Instance = Octocrab;

    fn id(&self) -> &str {
        RESOURCE_ID
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        let mut builder = Octocrab::builder().personal_token(config.token.clone());

        if let Some(url) = &config.base_url {
            builder = builder.base_uri(url).map_err(|e| Error::Configuration {
                message: format!("invalid base_url: {e}"),
                source: Some(Box::new(e)),
            })?;
        }

        builder.build().map_err(|e| Error::Initialization {
            resource_id: RESOURCE_ID.to_string(),
            reason: e.to_string(),
            source: Some(Box::new(e)),
        })
    }

    async fn is_valid(&self, instance: &Self::Instance) -> Result<bool> {
        Ok(instance.current().user().await.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(token: &str) -> GithubClientConfig {
        GithubClientConfig {
            token: token.to_string(),
            base_url: None,
        }
    }

    #[test]
    fn empty_token_fails_validation() {
        let err = config("").validate().unwrap_err();
        assert!(matches!(err, Error::Validation { .. }));
    }

    #[test]
    fn valid_token_passes_validation() {
        config("ghp_test").validate().unwrap();
    }

    #[test]
    fn resource_id_is_stable() {
        assert_eq!(GithubClientResource.id(), "github-client");
    }
}
