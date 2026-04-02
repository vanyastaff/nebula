//! Example: Telegram bot webhook trigger
//!
//! This example demonstrates how to create a Telegram bot webhook trigger
//! that receives messages and commands from Telegram.

// Reason: webhook crate still uses v1 compat types (Context/Scope); migration tracked separately.
#![allow(deprecated)]

use async_trait::async_trait;
use nebula_core::{ExecutionId, WorkflowId};
use nebula_resource::{Context, Scope};
use nebula_webhook::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const BOT_TOKEN: &str = "8582438389:AAGfJHsC6OdCl3o_LmkZ3uRDjMdzCDJEWtw";
const TELEGRAM_API: &str = "https://api.telegram.org";

/// Telegram update structure (simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<TelegramUser>,
    chat: TelegramChat,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramUser {
    id: i64,
    first_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

/// Telegram webhook trigger
struct TelegramTrigger {
    bot_token: String,
    client: reqwest::Client,
}

impl TelegramTrigger {
    fn new(bot_token: String) -> Self {
        Self {
            bot_token,
            client: reqwest::Client::new(),
        }
    }

    /// Set webhook URL at Telegram
    async fn set_webhook(&self, url: &str) -> Result<()> {
        let api_url = format!("{}/bot{}/setWebhook", TELEGRAM_API, self.bot_token);

        let response = self
            .client
            .post(&api_url)
            .json(&serde_json::json!({
                "url": url,
                "drop_pending_updates": true,
            }))
            .send()
            .await
            .map_err(|e| Error::other(format!("Failed to set webhook: {}", e)))?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::other(format!("Telegram API error: {}", text)));
        }

        Ok(())
    }

    /// Delete webhook from Telegram
    async fn delete_webhook(&self) -> Result<()> {
        let api_url = format!("{}/bot{}/deleteWebhook", TELEGRAM_API, self.bot_token);

        let response = self
            .client
            .post(&api_url)
            .send()
            .await
            .map_err(|e| Error::other(format!("Failed to delete webhook: {}", e)))?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::other(format!("Telegram API error: {}", text)));
        }

        Ok(())
    }

    /// Get bot info
    async fn get_me(&self) -> Result<serde_json::Value> {
        let api_url = format!("{}/bot{}/getMe", TELEGRAM_API, self.bot_token);

        let response = self
            .client
            .get(&api_url)
            .send()
            .await
            .map_err(|e| Error::other(format!("Failed to get bot info: {}", e)))?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::other(format!("Telegram API error: {}", text)));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::other(format!("Failed to parse response: {}", e)))?;

        Ok(json)
    }

    /// Send a message to a chat
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let api_url = format!("{}/bot{}/sendMessage", TELEGRAM_API, self.bot_token);

        let response = self
            .client
            .post(&api_url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            }))
            .send()
            .await
            .map_err(|e| Error::other(format!("Failed to send message: {}", e)))?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::other(format!("Telegram API error: {}", text)));
        }

        Ok(())
    }
}

#[async_trait]
impl WebhookAction for TelegramTrigger {
    type Event = TelegramUpdate;

    async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()> {
        println!("📱 Registering Telegram webhook...");

        let webhook_url = ctx.webhook_url();
        self.set_webhook(&webhook_url).await?;

        println!("✅ Telegram webhook registered at: {}", webhook_url);
        Ok(())
    }

    async fn on_webhook(
        &self,
        _ctx: &TriggerCtx,
        payload: WebhookPayload,
    ) -> Result<Option<Self::Event>> {
        // Parse Telegram update
        let update: TelegramUpdate = payload
            .body_json()
            .map_err(|e| Error::payload_parse(format!("Invalid Telegram update: {}", e)))?;

        println!("📥 Telegram update #{}", update.update_id);

        if let Some(ref message) = update.message
            && let Some(ref text) = message.text
        {
            let username = message
                .from
                .as_ref()
                .and_then(|u| u.username.as_deref())
                .unwrap_or("unknown");
            println!("   💬 Message from @{}: {}", username, text);
        }

        Ok(Some(update))
    }

    async fn on_unsubscribe(&self, _ctx: &TriggerCtx) -> Result<()> {
        println!("🗑️  Deleting Telegram webhook...");
        self.delete_webhook().await?;
        println!("✅ Telegram webhook deleted");
        Ok(())
    }

    async fn test(&self, _ctx: &TriggerCtx) -> Result<TestResult> {
        println!("🔍 Testing Telegram bot connection...");

        let start = std::time::Instant::now();
        let bot_info = self.get_me().await?;
        let latency = start.elapsed();

        let bot_name = bot_info["result"]["first_name"]
            .as_str()
            .unwrap_or("Unknown");
        let bot_username = bot_info["result"]["username"].as_str().unwrap_or("unknown");

        let message = format!("✅ Connected to bot: {} (@{})", bot_name, bot_username);

        Ok(TestResult::success(message)
            .with_sample(bot_info)
            .with_latency(latency))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("🤖 Telegram Webhook Example\n");

    // Configure webhook server
    let config = WebhookServerConfig {
        bind_addr: "127.0.0.1:8080".parse().unwrap(),
        base_url: "https://3b60-2601-441-4983-5b70-d0df-6a06-d1ec-8aca.ngrok-free.app".to_string(),
        path_prefix: "/webhooks".to_string(),
        ..Default::default()
    };

    println!("⚠️  IMPORTANT: You need a public HTTPS URL for Telegram webhooks!");
    println!("   Use ngrok: ngrok http 8080");
    println!("   Then replace 'your-domain.ngrok.io' in this code with your ngrok URL\n");

    // Start webhook server
    let server = WebhookServer::new(config).await?;
    println!("🚀 Webhook server started at http://localhost:8080\n");

    // Create trigger context
    let base = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
    let state = Arc::new(TriggerState::new("telegram-trigger"));
    let ctx = TriggerCtx::new(
        base,
        "telegram-trigger",
        Environment::Production,
        state,
        "https://3b60-2601-441-4983-5b70-d0df-6a06-d1ec-8aca.ngrok-free.app".to_string(),
        "/webhooks",
    );

    // Create Telegram trigger
    let trigger = TelegramTrigger::new(BOT_TOKEN.to_string());

    // Test connection first
    println!("📡 Testing Telegram connection...");
    match trigger.test(&ctx).await {
        Ok(result) => {
            println!("   {}", result.message);
            if let Some(latency) = result.latency {
                println!("   ⏱️  Latency: {:?}", latency);
            }
        }
        Err(e) => {
            println!("   ❌ Test failed: {}", e);
            return Err(e);
        }
    }

    println!("\n📍 Webhook URL: {}\n", ctx.webhook_url());

    // Subscribe to webhooks
    trigger.on_subscribe(&ctx).await?;
    let mut handle = server.subscribe(&ctx, None).await?;

    println!("✨ Telegram bot is ready!");
    println!("   Send messages to your bot to see them here\n");
    println!("Press Ctrl+C to stop...\n");

    // Process incoming webhooks
    tokio::select! {
        _ = async {
            while let Ok(payload) = handle.recv().await {
                // Process webhook using trigger
                match trigger.on_webhook(&ctx, payload).await {
                    Ok(Some(update)) => {
                        println!("✅ Update processed: {}", update.update_id);

                        // Автоматически отвечаем на сообщения
                        if let Some(ref message) = update.message
                            && let Some(ref text) = message.text
                        {
                            let reply = format!("✅ Получено: {}", text);
                            if let Err(e) = trigger.send_message(message.chat.id, &reply).await {
                                println!("❌ Failed to send reply: {}", e);
                            } else {
                                println!("📤 Sent reply to chat {}", message.chat.id);
                            }
                        }
                    }
                    Ok(None) => {
                        println!("⚠️  Update filtered");
                    }
                    Err(e) => {
                        println!("❌ Error processing update: {}", e);
                    }
                }
            }
        } => {},
        _ = tokio::signal::ctrl_c() => {
            println!("\n\n🛑 Shutting down...");
        }
    }

    // Cleanup
    trigger.on_unsubscribe(&ctx).await?;
    drop(handle);
    server.shutdown().await?;

    println!("👋 Goodbye!");
    Ok(())
}
