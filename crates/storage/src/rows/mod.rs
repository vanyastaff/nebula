//! Row types of the Plane-A account tables and the webhook activation spec.
//!
//! Raw storage shapes, not domain types: IDs are `Vec<u8>` (BYTEA/BLOB),
//! timestamps are `chrono::DateTime<chrono::Utc>`.

// Row structs are plain data containers where field names mirror SQL columns.
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod user;
mod webhook_activation;

pub use user::{
    OAuthStateRow, PersonalAccessTokenRow, SessionDraft, SessionRow, UserRow,
    VerificationTokenRow,
};
pub use webhook_activation::{
    WEBHOOK_ACTIVATION_KEY, WebhookActivationSpec, WebhookActivationSpecError,
    WebhookTimestampFormat,
};
