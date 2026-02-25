# Nebula Telegram Plugin

Telegram bot integration for the Nebula workflow engine using [teloxide].

Provides a **Resource** (`TelegramBotResource`) that wraps a [`teloxide::Bot`] client so the runtime can pool and manage bot instances. Use the resource in workflow actions to send messages, handle updates, or call any Telegram Bot API method.

## Usage

1. Add the crate to your workspace or binary:

   ```toml
   [dependencies]
   nebula-telegram = { path = "../plugins/telegram" }
   nebula-resource = { path = "../crates/resource", features = ["tokio"] }
   ```

2. Register the resource with the resource manager:

   ```rust
   use nebula_resource::{Manager, PoolConfig, Context, Scope};
   use nebula_telegram::{TelegramBotConfig, TelegramBotResource};

   let manager = Manager::new();
   let config = TelegramBotConfig {
       token: std::env::var("TELOXIDE_TOKEN").expect("TELOXIDE_TOKEN"),
       api_url: None, // or Some("https://api.telegram.org".into()) for custom Bot API
   };
   manager.register(
       TelegramBotResource,
       config,
       PoolConfig::default(),
   )?;
   ```

3. Acquire the bot in a workflow/action and use it:

   ```rust
   let guard = manager.acquire("telegram-bot", &ctx).await?;
   let bot = guard.as_any().downcast_ref::<Arc<teloxide::Bot>>().unwrap();
   bot.send_message(chat_id, "Hello from Nebula!").await?;
   ```

## Configuration

- **`token`** — Bot token from [@BotFather](https://t.me/BotFather). Required.
- **`api_url`** — Optional base URL for the Telegram Bot API (e.g. for a [local Bot API server](https://core.telegram.org/bots/api#using-a-local-bot-api-server)).

## Resource ID

The resource is registered under the ID `telegram-bot`. Use this string with `Manager::acquire("telegram-bot", &ctx)`.

[teloxide]: https://docs.rs/teloxide
