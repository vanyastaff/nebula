//! Webhook registration domain — `POST /orgs/{org}/workspaces/{ws}/webhooks`.
//!
//! This is the first live `mode=Prod` webhook producer endpoint.
//!
//! ## Module layout
//!
//! - [`dto`] — request/response DTOs with security invariant docs.
//! - [`handler`] — the `register_webhook` Axum handler; owns the 3-store
//!   write sequence and compensation path.

pub mod dto;
pub mod handler;
