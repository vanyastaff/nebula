//! Echo bot: получает сообщение и отвечает тем же текстом.
//! Ресурс берётся из Manager, бот клонируется в Dispatcher.
//!
//! Запуск: cargo run -p nebula-telegram --example echo_bot

use nebula_resource::{Context, Manager, PoolConfig, Scope};
use nebula_telegram::{TelegramBotConfig, TelegramBotResource};
use teloxide::dispatching::UpdateFilterExt;
use teloxide::prelude::*;
use teloxide::types::Message;

const TOKEN: &str = "8582438389:AAGfJHsC6OdCl3o_LmkZ3uRDjMdzCDJEWtw";

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn echo_handler(bot: Bot, msg: Message) -> HandlerResult {
    if let Some(text) = msg.text() {
        bot.send_message(msg.chat.id, text).await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let token = std::env::var("TELOXIDE_TOKEN").unwrap_or_else(|_| TOKEN.to_string());

    let manager = Manager::new();
    let config = TelegramBotConfig {
        token: token.clone(),
        api_url: None,
    };
    manager.register(TelegramBotResource, config, PoolConfig::default())?;

    let ctx = Context::new(Scope::Global, "echo-bot", "run-1");
    let guard = manager.acquire_typed(TelegramBotResource, &ctx).await?;
    // Клонируем Bot (дешёвый clone), отдаём guard в пул, диспетчер работает со своим клоном
    let bot: teloxide::Bot = (**guard).clone();
    drop(guard);

    let schema = Update::filter_message().endpoint(echo_handler);
    println!("Echo bot running. Send any message to the bot.");
    Dispatcher::builder(bot, schema).build().dispatch().await;
    Ok(())
}
