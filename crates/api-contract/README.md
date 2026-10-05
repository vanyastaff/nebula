# nebula-api-contract

Versioned transport data for Nebula's HTTP server and curated SDK client. This technical crate is published lockstep when the SDK dependency graph requires it; it is not a separately supported Rust product. Remote Rust consumers use `nebula-sdk`.

`v1` contains request and response bodies, query shapes, public role wrappers, pagination envelopes, RFC 9457 problems and credential wire codes. Both serde directions are available. Enable `openapi` for the same utoipa schemas the server serves. The default feature set contains no HTTP framework or HTTP executor.

Transport data does not grant runtime authority. The server resolves authenticated scope, authorizes intent and converts wire values through its owning ports. Internal cursor encoding, storage conversions, provider identity state, handlers and middleware remain server-owned. `v1::internal` describes unsupported operator-only transport separately.

Secret-bearing request and response values retain redacted formatting and the existing zeroizing authority response lifecycle. Redirects, pending tokens and form data returned by acquisition are sensitive transit data; do not log or casually persist them.
