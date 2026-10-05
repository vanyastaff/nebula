# nebula-api-contract — Agent orientation

**Purpose:** Versioned HTTP wire vocabulary shared by the technical server and the curated SDK client.
**Layer:** Core / shared-infra. Published only as a lockstep internal dependency; `nebula-sdk` remains the supported Rust product surface.

## Key files

- `src/v1/` — requests, responses, query shapes, RFC 9457 failures and public wire codes.
- `src/v1/shared.rs` — pagination envelopes and role wrappers; cursors are opaque to clients.
- `src/v1/auth.rs` — Plane-A identity transport, with redacted formatting and zeroizing authority responses.
- `src/v1/credential.rs` — Plane-B lifecycle/acquisition transport, independent of credential runtime authority.

## Conventions & never-do

- No server frameworks, HTTP clients, persistence adapters, runtime capabilities, tenant proofs, claim tokens or domain-to-wire conversions.
- Keep serde spelling, optionality, frozen tagged unions, OpenAPI schema names and secret direction annotations stable. Derive `Serialize` and `Deserialize` so both endpoints consume the same shapes.
- OpenAPI is optional. Gate its derives, attributes, imports and schema helpers under `openapi`; the default wire contract must compile without utoipa.
- Keep secret-aware `Debug` and zeroization intact. A transit authority string is not a runtime authority constructor.
- Tenant-role and credential-port conversions belong to `nebula-api`. Internal cursor payloads remain server-owned.
- `v1::internal` describes unsupported operator transport and is excluded from the supported client persona.

## Change checks

- Check default and `--features openapi` configurations.
- API regression evidence: unchanged `openapi_spec`, `openapi_canon_compliance`, `openapi_secret_redaction`, plus applicable lifecycle/authority suites.
- Dependency tree must exclude axum, tower, http, sqlx and reqwest.
- SDK external consumer proofs and public API snapshots apply when curated paths consume these types.

## See also

- [README.md](README.md)
- [API guide](../api/AGENTS.md)
- [Product canon](../../docs/PRODUCT_CANON.md)
