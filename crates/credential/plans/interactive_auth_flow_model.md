# Interactive Auth Flow Model

A design document for modeling interactive and semi-interactive authentication flows in an auth-focused library while keeping the core client-agnostic, resource-agnostic, and UI-agnostic.

---

## Purpose

This document defines how to model authentication flows that require one or more user-driven or externally-driven steps.

Examples include:

- browser-based OAuth / OIDC login
- SAML SSO
- MFA / OTP verification
- WebAuthn / passkey authentication
- OAuth Device Flow
- magic link login
- push approval flows
- CIBA-style backchannel login

The goal is to support these flows in a clean auth core without hardcoding:

- HTTP servers
- browser opening
- frontend UI
- CLI prompts
- resource/client-specific logic

---

# 1. Core design principle

> Core owns flow semantics, but the integrator owns orchestration.

That means:

- the auth library knows the rules of the flow
- the auth library knows which step comes next
- the auth library knows what input is required to continue
- the auth library knows when the flow is complete
- the integrator decides how to display prompts, redirect users, store state, receive callbacks, and resume the flow

This keeps the library:

- resource-agnostic
- client-agnostic
- UI-agnostic
- transport-agnostic

---

# 2. Interactive vs non-interactive flows

Interactive behavior should be treated as a separate flow property.

It is not the same thing as the protocol family.

---

## Non-interactive flows

These can usually complete in a single operation without human participation.

### Examples

- API key authentication
- OAuth 2.0 Client Credentials
- mTLS client authentication
- service JWT assertion
- client certificate authentication
- static token authentication

### Typical shape

- input credential
- verify or exchange
- output token / authenticated state

---

## Interactive flows

These require user participation and often span multiple steps.

### Examples

- OIDC Authorization Code flow
- SAML browser SSO
- password + MFA login
- WebAuthn challenge / assertion
- password reset confirmation
- login with consent screen

### Typical shape

- create challenge or authorization request
- external action occurs
- callback, response, or user-entered data is received
- state is validated
- flow continues or completes

---

## Semi-interactive flows

These begin machine-side but later depend on a separate user action.

### Examples

- OAuth Device Flow
- email magic link
- push approval
- backchannel authentication
- some out-of-band verification schemes

### Typical shape

- flow starts on one device or server
- user completes an action elsewhere
- flow continues via polling or callback

---

# 3. Why a one-shot API is not enough

A single method like this is not enough for the whole auth problem:

```rust
trait AuthProvider {
    fn authenticate(&self) -> Token;
}
```

This fails to model:

- redirects
- callbacks
- state validation
- MFA prompts
- device codes
- WebAuthn challenge / response
- asynchronous or delayed completion

Interactive auth requires a multi-step model.

---

# 4. Recommended architecture

Use two families of abstractions:

1. one-shot authentication for non-interactive flows
2. multi-step flow state machines for interactive and semi-interactive flows

---

## 4.1 Non-interactive API

```rust
pub trait Authenticator<Cx> {
    type Output;
    type Error;

    fn authenticate(&self, cx: &Cx) -> Result<Self::Output, Self::Error>;
}
```

This is suitable for:

- API keys
- client credentials
- mTLS setup
- direct token exchange
- service-to-service auth

---

## 4.2 Interactive API

Interactive flows should be modeled as resumable state machines.

```rust
pub trait InteractiveAuthenticator {
    type Input;
    type Action;
    type State;
    type Output;
    type Error;

    fn begin(
        &self,
        input: Self::Input,
    ) -> Result<FlowStatus<Self::Action, Self::State, Self::Output>, Self::Error>;

    fn advance(
        &self,
        state: Self::State,
        event: FlowEvent,
    ) -> Result<FlowStatus<Self::Action, Self::State, Self::Output>, Self::Error>;
}
```

This structure supports:

- browser callbacks
- OTP entry
- WebAuthn assertions
- device polling
- magic link completion
- asynchronous continuation

---

# 5. Flow status model

The core should return either:

- a final completed result
- or an action that must be performed externally plus a pending flow state

```rust
pub enum FlowStatus<A, S, O> {
    ActionRequired {
        action: A,
        next: S,
    },
    Completed(O),
}
```

This is the key boundary between core and integrator.

---

# 6. External action model

The flow should communicate what the outside world must do next.

These actions should remain generic and UI-neutral.

```rust
pub enum AuthAction {
    Redirect {
        url: String,
    },
    ShowUserCode {
        user_code: String,
        verification_uri: String,
        verification_uri_complete: Option<String>,
    },
    Prompt {
        kind: PromptKind,
        message: Option<String>,
    },
    AwaitExternalCallback,
    PollAfter {
        seconds: u64,
    },
}
```

### Notes

- `Redirect` can be used by web apps, desktop apps, or CLI tools
- `ShowUserCode` works well for device flow
- `Prompt` leaves rendering and UX to the integrator
- `AwaitExternalCallback` tells the integrator the flow is waiting for an inbound response
- `PollAfter` supports delayed continuation

The auth core should not open a browser, render a screen, or run a web server by itself.

---

## Prompt kinds

```rust
pub enum PromptKind {
    Password,
    Otp,
    Totp,
    EmailCode,
    Consent,
    WebAuthnChallenge,
}
```

This enum can be extended, but the point is to model what kind of external input is required.

---

# 7. Flow events

The integrator uses events to continue a pending flow.

```rust
pub enum FlowEvent {
    CallbackReceived {
        query: String,
    },
    UserCodeEntered(String),
    PasswordSubmitted(String),
    OtpSubmitted(String),
    WebAuthnResponse(Vec<u8>),
    Poll,
    Cancel,
}
```

### Notes

Events represent input from the outside world.

The core does not care whether they came from:

- a browser callback
- a terminal prompt
- a frontend form
- a webhook
- a mobile device
- a background poll loop

---

# 8. Pending flow state

Interactive flows almost always need state between steps.

Examples:

- OAuth `state`
- PKCE verifier
- nonce
- device code
- issued challenge
- expiration timestamp
- provider session identifiers
- anti-replay material

This state must be serializable or otherwise persistable by the integrator.

---

## State design goals

Pending state should ideally be:

- serializable
- resumable
- time-bounded
- verifiable
- minimal

### Example trait

```rust
pub trait FlowState {
    fn expires_at(&self) -> Option<std::time::SystemTime>;
}
```

In practice, concrete typed state structs are often better than a very generic trait.

---

# 9. Recommended metadata for flows

It is useful to describe flows with metadata, even if that metadata is not the flow implementation itself.

```rust
pub enum InteractionKind {
    NonInteractive,
    Interactive,
    SemiInteractive,
}

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

These help classify behavior without hardcoding any particular runtime.

---

## Example descriptor

```rust
pub struct AuthFlowDescriptor {
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

This is useful for discovery, diagnostics, configuration, and planning, even if actual execution uses separate traits.

---

# 10. Example flows

## 10.1 OIDC Authorization Code flow

### Begin

The core creates:

- authorization URL
- state
- PKCE verifier
- nonce
- expiration

It returns:

```rust
FlowStatus::ActionRequired {
    action: AuthAction::Redirect { url: auth_url },
    next: OidcPendingState { /* state, pkce, nonce, ... */ },
}
```

### Integrator responsibility

- redirect the user
- store pending state
- receive callback on return

### Advance

The integrator calls:

```rust
advance(pending_state, FlowEvent::CallbackReceived { query })
```

The core validates the callback and exchanges the code for tokens.

### Completion

```rust
FlowStatus::Completed(tokens)
```

---

## 10.2 OAuth Device Flow

### Begin

The core returns:

```rust
FlowStatus::ActionRequired {
    action: AuthAction::ShowUserCode {
        user_code,
        verification_uri,
        verification_uri_complete,
    },
    next: DeviceFlowState { /* device_code, interval, expiry */ },
}
```

### Integrator responsibility

- show the code to the user
- either poll periodically or provide a poll mechanism

### Advance

The integrator calls:

```rust
advance(state, FlowEvent::Poll)
```

The core either:

- returns another `PollAfter`
- or completes with tokens

---

## 10.3 WebAuthn login

### Begin

The core generates a challenge and returns an action indicating external user-agent work is required.

For example:

```rust
FlowStatus::ActionRequired {
    action: AuthAction::Prompt {
        kind: PromptKind::WebAuthnChallenge,
        message: None,
    },
    next: WebAuthnPendingState { /* challenge, rp, timeout, ... */ },
}
```

### Integrator responsibility

- send challenge data to the frontend
- invoke WebAuthn browser APIs
- collect the resulting assertion

### Advance

```rust
advance(state, FlowEvent::WebAuthnResponse(assertion_bytes))
```

### Completion

The core verifies the assertion and completes the flow.

---

## 10.4 Password + OTP

### Begin

The core may first ask for a password:

```rust
FlowStatus::ActionRequired {
    action: AuthAction::Prompt {
        kind: PromptKind::Password,
        message: Some("Enter your password".into()),
    },
    next: PasswordPendingState { /* ... */ },
}
```

### Advance password step

```rust
advance(state, FlowEvent::PasswordSubmitted(password))
```

If the password is valid but OTP is required, the core returns:

```rust
FlowStatus::ActionRequired {
    action: AuthAction::Prompt {
        kind: PromptKind::Otp,
        message: Some("Enter the verification code".into()),
    },
    next: OtpPendingState { /* ... */ },
}
```

### Advance OTP step

```rust
advance(state, FlowEvent::OtpSubmitted(code))
```

Then the flow completes.

---

# 11. Core responsibilities vs integrator responsibilities

This boundary is critical.

---

## Core responsibilities

The auth core should:

- define the flow state machine
- generate challenges and authorization requests
- validate callbacks and responses
- verify anti-replay data
- exchange tokens or assertions
- decide the next step or completion
- define provider-neutral actions and events
- expose state required for continuation

---

## Integrator responsibilities

The integrator should:

- store pending flow state
- render UI or prompts
- redirect the user or present links
- receive callbacks or out-of-band responses
- collect user-entered data
- decide when to call `advance`
- choose the runtime model: web, CLI, desktop, server, mobile

---

# 12. What should stay out of core

The auth core should not directly own:

- browser launching
- local web server lifecycle
- HTML form rendering
- terminal input handling
- framework-specific request/response objects
- provider-specific UI
- resource/client-specific request application

These belong to adapters, apps, or higher-level integration layers.

---

# 13. Provider extensions and interactive flows

Provider-specific crates may define:

- provider metadata
- default endpoints
- provider-specific flow state
- provider-specific quirks
- helper builders

But even provider-specific crates should avoid owning runtime orchestration.

Example:

- `auth-provider-github` may know how to build a GitHub authorization URL
- it should not be responsible for opening the browser or running a callback server

The same boundary still applies:

> Provider extension owns provider semantics, integrator owns orchestration.

---

# 14. Common design mistakes to avoid

## Mistake 1: One trait for all flows

Bad:

```rust
trait AuthProvider {
    fn authenticate(&self) -> Token;
}
```

This is too small to express redirects, polling, prompts, callbacks, and resumable state.

---

## Mistake 2: Provider owns the UI

Bad examples:

- provider opens the browser automatically
- provider launches a local HTTP server automatically
- provider prompts in the terminal directly

This makes the core opinionated and hard to integrate in other environments.

---

## Mistake 3: Flow state is hidden and not resumable

Interactive auth almost always needs durable state.

If the state cannot be stored and resumed cleanly, the flow becomes fragile and framework-specific.

---

## Mistake 4: Treating interactivity as a property of the resource

Interactivity belongs to the auth flow, not to the target resource or client.

---

# 15. Recommended minimal abstraction set

For an auth-only core, a strong minimal design is:

- `Authenticator<Cx>` for non-interactive flows
- `InteractiveAuthenticator` for multi-step flows
- `FlowStatus<A, S, O>` for pending vs complete
- `AuthAction` for outside-world requirements
- `FlowEvent` for resuming a pending flow
- typed pending state structs
- `InteractionKind` metadata for classification

This is enough to model a wide range of modern auth scenarios cleanly.

---

# 16. Final recommendation

Interactive auth should be modeled as a resumable, multi-step state machine.

The core should:

- return actions
- expose pending state
- accept external events
- decide the next step

The integrator should:

- present UX
- store state
- receive external data
- resume the flow

This keeps the auth library small, powerful, reusable, and independent from specific clients, resources, transports, or UI frameworks.

---

# 17. One-sentence rule

> Interactivity is a property of the auth flow lifecycle, not of the resource, client, or provider UI.

