//! HTTP resource adapter (`resource-http` feature): requests submitted as
//! managed units on a resource whose instance holds an [`HttpTransport`].
//!
//! - **Transport.** [`HttpConfig`] is the operator configuration;
//!   [`HttpTransport::new`] builds an auth-neutral connection pool from it
//!   in `Provider::create`, reused across credential rotations. It follows
//!   no redirect (a `3xx` is an answer), retries nothing, uses no proxy,
//!   sends no `Referer`, keeps no cookies and holds no credential. TLS
//!   verifies against the platform's roots plus the configured extra
//!   roots, with no switch to skip verification.
//! - **Credentials.** A resource implements [`HttpApi::authorize`]: the
//!   unit's pinned slots become headers through [`Authorize`] (`bearer`,
//!   `basic`, `api_key_header`), applied after the attempt is granted and
//!   last. A slot pinned `None` refuses the attempt `CredentialUnavailable`
//!   before anything is sent. Values are zeroized and marked sensitive.
//! - **Requests.** A [`Request`] is the unit's intent; its method marker
//!   ([`Get`], [`Post`], [`Keyed`], [`AsWrite`], …) fixes the
//!   [`Effect`](nebula_resource::call::Effect), and so whether an unknown
//!   outcome may be retried. Submit it with
//!   [`Lease::submit`](nebula_resource::call::Lease::submit) — or call
//!   [`send`] on an attempt of a custom operation.
//! - **Answers.** Each exchange classifies its answer once with an
//!   [`OperationError`](nebula_resource::call::OperationError) constructor
//!   ([`send`] has the table) — a connect failure `unreachable`, a lost
//!   connection or a `5xx` `interrupted`, a `429` `throttled`, another
//!   `4xx` `rejected` — and the runtime derives the sent state, the rate
//!   limit's verdict and any re-attempt from it: a throttle pauses the
//!   quota, a `Write` that may have been sent is never re-attempted, and a
//!   failed unit's error never carries the provider's text.
//! - **Streams.** [`open_stream`] runs one exchange as one streaming unit
//!   and returns a [`ResponseStream`] once the head arrived; the body is
//!   read in chunks through a small buffer, so a slow reader pushes back on
//!   the provider. The lease closing, a dropped or cancelled stream, the
//!   deadline and the stream byte budget end it. The 5-minute unit cap
//!   applies; there is no interval profile for longer streams yet.
//!
//! Nothing here prints or logs a URL, a header value or a transport error;
//! each attempt runs in a `nebula.sdk.http.attempt` span (method, attempt,
//! status, sent state).
//!
//! Out of scope: following absolute or next-page URLs, query-parameter API
//! keys, client certificates (mTLS), a generic `Http<C>` resource, streaming
//! request bodies, turning `401` / `403` into a credential signal, and
//! per-key throttles (`KeyThrottled`).

mod auth;
mod config;
mod exchange;
mod request;
mod response;
mod stream;

pub use auth::{Authorize, BearerMaterial, HttpApi};
pub use config::{HttpConfig, HttpTransport};
pub use exchange::send;
pub use request::{AsWrite, Delete, Get, Head, Keyed, Method, Options, Patch, Post, Put, Request};
pub use response::Response;
pub use stream::{ResponseStream, open_stream, open_stream_until};
