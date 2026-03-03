# Protocol Layer

Unified view of credential protocols in `nebula-credential`.

## Protocol → Trait Mapping

| Protocol | Trait | Config | State | Notes |
|----------|-------|--------|-------|-------|
| `ApiKeyProtocol` | `StaticProtocol` | — | `ApiKeyState` | server + token |
| `BasicAuthProtocol` | `StaticProtocol` | — | `BasicAuthState` | username + password |
| `HeaderAuthProtocol` | `StaticProtocol` | — | `HeaderAuthState` | header_name + header_value |
| `DatabaseProtocol` | `StaticProtocol` | — | `DatabaseState` | host, port, database, username, password, ssl_mode |
| `OAuth2Protocol` | `FlowProtocol` | `OAuth2Config` | `OAuth2State` | AuthCode, ClientCredentials, Device |
| `LdapProtocol` | `FlowProtocol` | `LdapConfig` | `LdapState` | host, port, bind_dn, tls_mode |
| `SamlConfig` | *(stub)* | `SamlConfig` | — | Phase 5 |
| `KerberosConfig` | *(stub)* | `KerberosConfig` | — | Phase 5 |
| `MtlsConfig` | *(stub)* | `MtlsConfig` | — | Phase 5 |

## StaticProtocol vs FlowProtocol

- **StaticProtocol**: sync, no IO. `parameters()` + `build_state(values)` → State.
- **FlowProtocol**: async. `parameters()` + `initialize(config, values, ctx)` → `InitializeResult<State>`. Optional `refresh`/`revoke`.

Protocols do **not** know about storage or rotation; they only build/update State and return `InitializeResult`/`refresh`/`revoke`.

## Config Types (FlowProtocol)

- **OAuth2Config**: auth_url, token_url, scopes, grant_type, auth_style, pkce
- **LdapConfig**: tls (None/Tls/StartTls), timeout, ca_cert
- **SamlConfig**: binding (HttpPost/HttpRedirect), sign_requests
- **KerberosConfig**, **MtlsConfig**: stubs

## Core Types (core::result)

- **InitializeResult\<S\>**: `Complete(S)` | `Pending { partial_state, next_step }` | `RequiresInteraction(InteractionRequest)`
- **PartialState**: data, step, created_at, ttl_seconds, metadata
- **UserInput**: Callback, Code, CaptchaSolution, Poll, ChallengeResponse, ConfirmationToken, Custom
- **InteractionRequest**: Redirect, CodeInput, DisplayInfo, AwaitConfirmation, Challenge, Captcha, Custom
