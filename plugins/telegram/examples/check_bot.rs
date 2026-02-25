//! One-off check: register TelegramBotResource, acquire, call get_me.
//! Run: cargo run -p nebula-telegram --example check_bot

use nebula_resource::{Context, Manager, PoolConfig, Scope};
use nebula_telegram::{TelegramBotConfig, TelegramBotResource};
use teloxide::prelude::Requester;

const TOKEN: &str = "8582438389:AAGfJHsC6OdCl3o_LmkZ3uRDjMdzCDJEWtw";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let token = std::env::var("TELOXIDE_TOKEN").unwrap_or_else(|_| TOKEN.to_string());

    let manager = Manager::new();
    let config = TelegramBotConfig {
        token: token.clone(),
        api_url: None,
    };
    manager.register(TelegramBotResource, config, PoolConfig::default())?;

    let ctx = Context::new(Scope::Global, "check", "run-1");
    let guard = manager.acquire_typed(TelegramBotResource, &ctx).await?;
    let me = guard.get_me().await?;
    println!(
        "Bot OK: @{} (id: {})",
        me.user.username.as_deref().unwrap_or("?"),
        me.user.id
    );

    Ok(())
}
