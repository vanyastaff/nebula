# Auth Taxonomy for a Library

A practical reference for designing an authentication / authorization library without mixing concepts from different abstraction layers.

---

## Why this taxonomy matters

A common design mistake is to put things like `OAuth2`, `JWT`, `API Key`, `Session Cookie`, and `Auth0` into one enum or one namespace as if they were the same kind of entity.

They are not.

These concepts answer different questions:

- What standard flow is used?
- What credential proves identity?
- What format carries claims?
- How is the credential or token presented?
- How is auth state stored?
- Which vendor or product implements the system?
- Does the flow require user interaction?

A clean library should model these separately.

---

# 1. Core taxonomy

The most useful top-level split is into these layers:

1. **Protocol**
2. **Credential / Factor**
3. **Token / Assertion Format**
4. **Transport / Presentation Mechanism**
5. **Session / State Model**
6. **Provider / Product / Implementation**
7. **Interaction Model**

---

## 1. Protocols

Protocols define a standardized interaction between parties.

They usually define:

- actors / participants
- message flow
- challenge-response rules
- token or assertion issuance
- validation and trust rules

### Examples

- **OAuth 2.0** — delegated authorization
- **OpenID Connect (OIDC)** — authentication layer on top of OAuth 2.0
- **SAML 2.0** — federation / SSO / assertion-based identity exchange
- **Kerberos** — network authentication
- **WebAuthn / FIDO2** — strong authentication / passkeys
- **GNAP** — newer authorization protocol model
- **OpenID4VC / OpenID4VP / OpenID4VCI** — verifiable credential flows

### Not protocols

- JWT
- API Key
- Session Cookie
- Auth0

---

## 2. Credentials / Authentication Factors

A credential is what a subject presents to prove identity.

### Examples

- **Password**
- **Passkey**
- **TOTP code**
- **SMS code**
- **API Key**
- **Client secret**
- **Client certificate**
- **Private key signature**
- **Magic link token**
- **Hardware security key**

### Notes

`API Key` belongs much closer to **credential** than to **protocol**.

---

## 3. Token / Assertion Formats

These define the representation of identity claims, access grants, or signed data.

They are data formats, not complete auth systems.

### Examples

- **JWT**
- **JWS**
- **JWE**
- **PASETO**
- **Opaque Token**
- **SAML Assertion**
- **Macaroon**
- **X.509 certificate** (sometimes modeled as credential and/or assertion container)

### Important distinction

- `Access Token` is a **semantic role**
- `JWT` is a **format**

An access token can be:

- JWT
- opaque
- PASETO

So `JWT` and `AccessToken` should not be treated as the same kind of thing.

---

## 4. Transport / Presentation Mechanisms

These describe how a credential or token is sent to the verifying party.

### Examples

- `Authorization: Bearer <token>`
- `Authorization: Basic <base64>`
- `Cookie: session=...`
- `X-API-Key: ...`
- mTLS certificate presentation
- signed request headers
- query parameter token
- form post callback

### Important distinction

`Bearer` is not the token format itself.

`Bearer` is the presentation method.

The token being carried may be:

- JWT
- opaque token
- PASETO

---

## 5. Session / State Model

This describes how authenticated state persists between requests.

### Examples

- **Server-side session**
- **Stateless token session**
- **Refresh-token based session**
- **Sliding session**
- **Persistent login session**
- **One-time challenge session**
- **Device-bound session**

### Important distinction

`Cookie` and `Session` are not the same thing.

- `Cookie` is presentation / transport
- `Session` is state model

---

## 6. Providers / Products / Implementations

These are concrete systems or vendors.

### Examples

- **Auth0**
- **Okta**
- **Keycloak**
- **FusionAuth**
- **AWS Cognito**
- **Microsoft Entra ID**
- **Clerk**
- **Supabase Auth**

### Important distinction

A provider may implement or expose:

- OAuth 2.0
- OIDC
- SAML
- passkeys
- sessions
- social login
- MFA

But the provider itself is not the protocol.

---

## 7. Interaction Model

This describes whether a flow requires user participation or multiple interactive steps.

This is extremely important for library design.

### Interaction categories

- **Interactive**
- **NonInteractive**
- **SemiInteractive**

### Interactive means

A user is involved and the flow may require:

- login page
- redirect
- consent screen
- MFA input
- device confirmation
- user presence
- challenge-response with a user-controlled authenticator

### NonInteractive means

The client can authenticate without human participation.

### SemiInteractive means

The flow is initiated by a machine or device but later requires user action elsewhere.

Examples:

- OAuth Device Flow
- magic link
- push approval
- email verification
- CIBA

---

# 2. Classification of common terms

## Protocols

- OAuth 2.0
- OpenID Connect
- SAML 2.0
- Kerberos
- WebAuthn / FIDO2
- GNAP
- OpenID4VC / OpenID4VP / OpenID4VCI

## Credentials / Factors

- Password
- API Key
- Passkey
- TOTP
- SMS OTP
- Magic link token
- Client certificate
- Client secret
- Hardware security key

## Token / Assertion Formats

- JWT
- JWS
- JWE
- PASETO
- Opaque token
- SAML Assertion
- Macaroon

## Presentation / Transport

- Bearer token header
- Basic auth header
- Cookie
- Custom header (`X-API-Key`)
- Query parameter token
- mTLS presentation

## Session / State Models

- Server-side session
- Stateless token session
- Refresh-token session
- Sliding session
- Persistent session
- One-time login session
- Device session

## Providers / Products

- Auth0
- Okta
- Keycloak
- AWS Cognito
- Microsoft Entra ID
- FusionAuth
- Clerk
- Supabase Auth

---

# 3. Where specific examples belong

## OAuth 2.0

**Category:** Protocol

## OpenID Connect

**Category:** Protocol

## SAML

**Category:** Protocol

## WebAuthn

**Category:** Protocol

## API Key

**Category:** Credential

Potential subtype naming:

- `ClientCredential`
- `ApiCredential`
- `ApiKeyCredential`

## JWT

**Category:** Token / Assertion Format

## Bearer Token

**Category:** Presentation / Transport Mechanism

## Cookie Session

This actually spans two layers:

- `Cookie` → Presentation / Transport
- `Session` → State Model

## Basic Auth

Best modeled as:

- HTTP authentication scheme
- presentation mechanism

## Auth0

**Category:** Provider / Product

## mTLS

This is often a composite concept and is best split into:

- `ClientCertificate` → Credential
- `MutualTlsPresentation` → Transport / Channel mechanism

---

# 4. Interactive vs non-interactive auth flows

This is a separate dimension from protocol type.

A protocol is not automatically interactive or non-interactive. The specific flow matters.

---

## Interactive flows

These usually require explicit user participation.

### Examples

- OIDC Authorization Code flow
- SAML SSO
- Password login
- Password + MFA
- WebAuthn login
- TOTP verification
- SMS OTP login
- Magic link confirmation

### Common properties

- multi-step
- UI integration
- redirects or callbacks
- anti-CSRF / anti-replay requirements
- temporary flow state
- expiration between steps
- challenge-response

---

## Non-interactive flows

These are machine-driven and typically do not require a user.

### Examples

- API Key auth
- OAuth 2.0 Client Credentials
- mTLS
- service JWT assertion
- static bearer token
- basic auth between services
- client certificate auth

### Common properties

- direct verification
- no consent screen
- no user presence
- no redirect browser flow
- often used for service-to-service communication

---

## Semi-interactive flows

These start without a browser-style interactive user step but eventually require user action.

### Examples

- OAuth Device Flow
- CIBA
- push approval
- magic link in some architectures
- email verification flow

### Common properties

- decoupled user action
- polling or callback continuation
- cross-device support
- out-of-band confirmation

---

# 5. Important design insight: interactivity is not a protocol category

A single protocol may support multiple interaction styles.

## Example: OAuth 2.0

OAuth 2.0 can be:

- **Authorization Code** → Interactive
- **Device Code** → SemiInteractive
- **Client Credentials** → NonInteractive
- **Refresh Token** → typically NonInteractive

So OAuth 2.0 itself should not be labeled simply as interactive or non-interactive.

It depends on the grant / flow.

## Example: OIDC

OIDC is commonly interactive for browser login, but token refresh or session continuation may be non-interactive.

## Example: WebAuthn

Usually interactive because it requires challenge-response and user action.

---

# 6. Practical library design guidance

Do not make a single enum like this:

```rust
enum AuthMethod {
    OAuth2,
    Jwt,
    ApiKey,
    Auth0,
}
```

This is bad because each variant belongs to a different abstraction layer:

- `OAuth2` → protocol
- `Jwt` → token format
- `ApiKey` → credential
- `Auth0` → provider

A clean design models orthogonal dimensions separately.

---

# 7. Recommended Rust-style taxonomy

## High-level module layout

```text
auth/
  protocol/
  credential/
  token/
  presentation/
  session/
  provider/
  authorization/
  principal/
  policy/
```

If you want a larger system split:

```text
auth/
  authentication/
    protocol/
    credential/
    challenge/
    verifier/
  authorization/
    policy/
    permission/
    scope/
    role/
  token/
  presentation/
  session/
  provider/
```

---

## Recommended enums

```rust
pub enum ProtocolKind {
    OAuth2,
    OpenIdConnect,
    Saml2,
    WebAuthn,
    Kerberos,
    Gnap,
    OpenId4Vc,
}

pub enum CredentialKind {
    Password,
    ApiKey,
    Passkey,
    Totp,
    SmsOtp,
    ClientCertificate,
    ClientSecret,
    MagicLinkToken,
}

pub enum TokenFormatKind {
    Jwt,
    Paseto,
    Opaque,
    SamlAssertion,
    Macaroon,
}

pub enum PresentationKind {
    BearerHeader,
    BasicHeader,
    Cookie,
    MutualTls,
    QueryParameter,
    CustomHeader,
}

pub enum SessionKind {
    ServerSide,
    Stateless,
    RefreshToken,
    Sliding,
    Persistent,
    DeviceBound,
}

pub enum ProviderKind {
    Auth0,
    Okta,
    Keycloak,
    Cognito,
    EntraId,
    FusionAuth,
    Clerk,
    Supabase,
}

pub enum InteractionKind {
    Interactive,
    NonInteractive,
    SemiInteractive,
}
```

---

## Useful additional dimensions

```rust
pub enum InitiatorKind {
    User,
    Client,
    Service,
    Device,
    ExternalProvider,
}

pub enum FlowChannel {
    BrowserRedirect,
    Backchannel,
    DirectRequest,
    CrossDevice,
    LocalDevice,
}
```

---

## Example flow descriptor

```rust
pub struct AuthFlowDescriptor {
    pub protocol: Option<ProtocolKind>,
    pub credential: CredentialKind,
    pub token_format: Option<TokenFormatKind>,
    pub presentation: PresentationKind,
    pub session: Option<SessionKind>,
    pub provider: Option<ProviderKind>,
    pub interaction: InteractionKind,
    pub initiator: InitiatorKind,
    pub channel: FlowChannel,
    pub supports_user_presence: bool,
    pub supports_user_consent: bool,
    pub supports_redirect: bool,
    pub supports_backchannel: bool,
    pub supports_machine_only: bool,
}
```

---

# 8. Example descriptors

## OIDC browser login

```rust
AuthFlowDescriptor {
    protocol: Some(ProtocolKind::OpenIdConnect),
    credential: CredentialKind::Password,
    token_format: Some(TokenFormatKind::Jwt),
    presentation: PresentationKind::Cookie,
    session: Some(SessionKind::ServerSide),
    provider: Some(ProviderKind::Auth0),
    interaction: InteractionKind::Interactive,
    initiator: InitiatorKind::User,
    channel: FlowChannel::BrowserRedirect,
    supports_user_presence: true,
    supports_user_consent: true,
    supports_redirect: true,
    supports_backchannel: false,
    supports_machine_only: false,
}
```

## OAuth 2.0 Client Credentials

```rust
AuthFlowDescriptor {
    protocol: Some(ProtocolKind::OAuth2),
    credential: CredentialKind::ClientSecret,
    token_format: Some(TokenFormatKind::Opaque),
    presentation: PresentationKind::BearerHeader,
    session: Some(SessionKind::RefreshToken),
    provider: Some(ProviderKind::Keycloak),
    interaction: InteractionKind::NonInteractive,
    initiator: InitiatorKind::Client,
    channel: FlowChannel::DirectRequest,
    supports_user_presence: false,
    supports_user_consent: false,
    supports_redirect: false,
    supports_backchannel: false,
    supports_machine_only: true,
}
```

## OAuth Device Flow

```rust
AuthFlowDescriptor {
    protocol: Some(ProtocolKind::OAuth2),
    credential: CredentialKind::Password,
    token_format: Some(TokenFormatKind::Jwt),
    presentation: PresentationKind::BearerHeader,
    session: Some(SessionKind::RefreshToken),
    provider: Some(ProviderKind::Okta),
    interaction: InteractionKind::SemiInteractive,
    initiator: InitiatorKind::Device,
    channel: FlowChannel::CrossDevice,
    supports_user_presence: true,
    supports_user_consent: true,
    supports_redirect: false,
    supports_backchannel: true,
    supports_machine_only: false,
}
```

## WebAuthn login

```rust
AuthFlowDescriptor {
    protocol: Some(ProtocolKind::WebAuthn),
    credential: CredentialKind::Passkey,
    token_format: None,
    presentation: PresentationKind::CustomHeader,
    session: Some(SessionKind::ServerSide),
    provider: None,
    interaction: InteractionKind::Interactive,
    initiator: InitiatorKind::User,
    channel: FlowChannel::LocalDevice,
    supports_user_presence: true,
    supports_user_consent: false,
    supports_redirect: false,
    supports_backchannel: false,
    supports_machine_only: false,
}
```

---

# 9. Common design mistakes to avoid

## Mistake 1: Mixing abstraction layers in one enum

Bad:

```rust
enum AuthMethod {
    OAuth2,
    Jwt,
    ApiKey,
    Auth0,
}
```

## Mistake 2: Treating `Bearer` as a token format

`Bearer` is a presentation method.

The token may still be JWT, opaque, or something else.

## Mistake 3: Treating `SessionCookie` as a protocol

- cookie = transport
- session = state model

## Mistake 4: Treating `Auth0` as an auth protocol

It is a provider / product.

## Mistake 5: Treating interactivity as a property of the protocol only

Interactivity is usually a property of the **flow**, not just the protocol family.

---

# 10. Practical classification table

| Term | Category |
|---|---|
| OAuth 2.0 | Protocol |
| OpenID Connect | Protocol |
| SAML | Protocol |
| Kerberos | Protocol |
| WebAuthn | Protocol |
| GNAP | Protocol |
| Password | Credential |
| API Key | Credential |
| Passkey | Credential |
| TOTP | Credential |
| Client Certificate | Credential |
| Client Secret | Credential |
| JWT | Token Format |
| PASETO | Token Format |
| Opaque Token | Token Format |
| SAML Assertion | Assertion Format |
| Bearer | Presentation Mechanism |
| Basic | Presentation Mechanism |
| Cookie | Presentation Mechanism |
| mTLS | Presentation / Channel Mechanism |
| Server Session | Session Model |
| Refresh Session | Session Model |
| Auth0 | Provider |
| Okta | Provider |
| Keycloak | Provider |

---

# 11. Practical mental model

When designing any auth feature, ask these questions in order:

1. **What flow standard is used?**
   - OAuth2, OIDC, SAML, WebAuthn, etc.

2. **What proves identity or client legitimacy?**
   - password, passkey, certificate, API key, client secret

3. **What format carries the result?**
   - JWT, opaque token, PASETO, assertion

4. **How is it presented?**
   - bearer header, cookie, mTLS, custom header

5. **How is state persisted?**
   - server session, stateless token, refresh rotation

6. **Is the flow interactive?**
   - interactive, non-interactive, semi-interactive

7. **Which provider implements it?**
   - Auth0, Okta, Keycloak, internal IdP, custom implementation

If each of these questions maps to a separate type or module, the architecture stays clean.

---

# 12. Recommended naming guidance

Prefer precise names over vague buckets like `mechanism`.

Good names:

- `protocol`
- `credential`
- `token`
- `presentation`
- `session`
- `provider`
- `interaction`

Avoid using one broad bucket like `mechanism` for everything, because it becomes ambiguous:

- Is API key a mechanism?
- Is bearer a mechanism?
- Is MFA a mechanism?
- Is session cookie a mechanism?

It is usually better to split them by role.

---

# 13. Final recommendation

For a clean auth library, separate the following dimensions explicitly:

- **Protocols** — OAuth2, OIDC, SAML, WebAuthn, Kerberos
- **Credentials** — Password, API Key, Passkey, Certificate, TOTP
- **Token Formats / Assertions** — JWT, PASETO, Opaque, SAML Assertion
- **Presentation Mechanisms** — Bearer, Basic, Cookie, mTLS
- **Session Models** — Server session, Stateless session, Refresh session
- **Providers** — Auth0, Okta, Keycloak, Cognito
- **Interaction Models** — Interactive, NonInteractive, SemiInteractive

And never mix them into one flat enum as if they were the same abstraction level.

---

# 14. One-sentence rule for the whole library

> If two auth concepts answer different architectural questions, they should not live in the same enum or namespace as peers.

