//! # Nebula API
//!
//! REST API server для Nebula workflow engine.
//!
//! Следует принципу "API как точка входа": тонкий HTTP-слой без бизнес-логики.
//! Вся логика выполнения находится в engine/storage/credential через порты (traits).
//!
//! ## Архитектура
//!
//! - **Handlers** — тонкие обработчики HTTP запросов (извлечение данных + делегация)
//! - **Services** — бизнес-логика (вызов портов, оркестрация)
//! - **Routes** — маршрутизация по доменам (users, workflows, executions, health)
//! - **Middleware** — auth, rate limiting, tracing, error handling
//! - **Models** — DTOs для API (запросы/ответы)
//! - **Errors** — RFC 9457 Problem Details

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod app;
pub mod config;
pub mod errors;
pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;
pub mod state;

pub use app::build_app;
pub use config::ApiConfig;
pub use state::AppState;
