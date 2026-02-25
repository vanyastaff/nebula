//! # Nebula Telegram Plugin
//!
//! Telegram bot integration for the Nebula workflow engine using [teloxide].
//!
//! Provides a [`Resource`](nebula_resource::Resource) that wraps a [`teloxide::Bot`] client,
//! so the runtime can pool and manage bot instances. Use the resource in workflow
//! actions to send messages, handle updates, or call any Telegram Bot API method.
//!
//! [teloxide]: https://docs.rs/teloxide

pub mod resources;

pub use resources::{TelegramBotConfig, TelegramBotResource};
