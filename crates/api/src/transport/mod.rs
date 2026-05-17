//! Protocol transports for the Nebula API (spec D5).
//!
//! This module contains inbound protocol transports — not business services.
//! Business logic lives in `handlers`; the transport layer owns routing,
//! signature policy, replay-window enforcement, and the dispatch pipeline.

pub mod credential;
pub mod oauth;
pub mod webhook;
