//! Authentication module: extractors, OAuth handlers, and auth middleware helpers.

mod cors;
mod extractor;
mod oauth;

pub(crate) use cors::cors_layer;
pub(crate) use extractor::Authenticated;
pub(crate) use oauth::{auth_me, github_callback, oauth_callback, oauth_start};
