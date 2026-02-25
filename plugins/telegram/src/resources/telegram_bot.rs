//! Telegram Bot resource backed by [`teloxide`].
//!
//! Implements [`Resource`] so the bot client can be pooled and managed by
//! `nebula-resource`. The instance is a [`teloxide::Bot`] that can be used
//! to send messages, handle updates, and call any Telegram Bot API method.

use std::sync::Arc;

use nebula_resource::metadata::ResourceMetadata;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::{Context, Error, FieldViolation, Result};
use teloxide::requests::Requester;

const RESOURCE_ID: &str = "telegram-bot";

/// Configuration for [`TelegramBotResource`].
#[derive(Clone, Debug)]
pub struct TelegramBotConfig {
    /// Bot token from [@BotFather](https://t.me/BotFather).
    pub token: String,
    /// Optional base URL for the Telegram Bot API (e.g. for local Bot API server).
    pub api_url: Option<String>,
}

impl Config for TelegramBotConfig {
    fn validate(&self) -> Result<()> {
        if self.token.trim().is_empty() {
            return Err(Error::validation(vec![FieldViolation::new(
                "token",
                "must not be empty",
                "(empty)",
            )]));
        }
        if let Some(ref url) = self.api_url {
            if url.trim().is_empty() {
                return Err(Error::validation(vec![FieldViolation::new(
                    "api_url",
                    "must not be empty when set",
                    "(empty)",
                )]));
            }
        }
        Ok(())
    }
}

/// Resource that produces a [`teloxide::Bot`] client.
///
/// The bot can be acquired from the resource manager and used to send messages,
/// get updates, or perform any action supported by the Telegram Bot API.
pub struct TelegramBotResource;

impl Resource for TelegramBotResource {
    type Config = TelegramBotConfig;
    type Instance = Arc<teloxide::Bot>;

    fn id(&self) -> &str {
        RESOURCE_ID
    }

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            RESOURCE_ID,
            "Telegram Bot",
            "Teloxide-based Telegram Bot API client for sending messages and receiving updates.",
        )
        .with_icon("telegram")
        .with_tags(["category:bot", "category:messaging", "service:telegram"])
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        let mut bot = teloxide::Bot::new(config.token.clone());

        if let Some(ref url) = config.api_url {
            let parsed = url.parse::<url::Url>().map_err(|e| Error::Configuration {
                message: format!("invalid api_url: {e}"),
                source: Some(Box::new(e)),
            })?;
            bot = bot.set_api_url(parsed);
        }

        Ok(Arc::new(bot))
    }

    async fn is_valid(&self, instance: &Self::Instance) -> Result<bool> {
        instance
            .get_me()
            .await
            .map(|_| true)
            .map_err(|e| Error::HealthCheck {
                resource_id: RESOURCE_ID.to_string(),
                reason: format!("Telegram getMe failed: {e}"),
                attempt: 1,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(token: &str) -> TelegramBotConfig {
        TelegramBotConfig {
            token: token.to_string(),
            api_url: None,
        }
    }

    #[test]
    fn empty_token_fails_validation() {
        let err = config("").validate().unwrap_err();
        assert!(matches!(err, Error::Validation { .. }));
    }

    #[test]
    fn whitespace_only_token_fails_validation() {
        let err = config("   ").validate().unwrap_err();
        assert!(matches!(err, Error::Validation { .. }));
    }

    #[test]
    fn valid_token_passes_validation() {
        config("123:ABC").validate().unwrap();
    }

    #[test]
    fn resource_id_is_stable() {
        assert_eq!(TelegramBotResource.id(), RESOURCE_ID);
    }

    #[test]
    fn metadata_has_expected_name_and_icon() {
        let meta = TelegramBotResource.metadata();
        assert_eq!(meta.key, RESOURCE_ID);
        assert_eq!(meta.name, "Telegram Bot");
        assert_eq!(meta.icon.as_deref(), Some("telegram"));
    }
}
