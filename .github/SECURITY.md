# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Nebula, please report it responsibly.

**Do not open a public issue for security vulnerabilities.**

Instead, please either:

- Open a private **security advisory** on GitHub for the `vanyastaff/nebula` repository, or
- Email the maintainer at: **vanya.john.stafford@gmail.com**

### What to include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix or mitigation (if you have one)

### Response expectations

- **Acknowledgment**: within 72 hours
- **Initial assessment**: within 7 days
- **Fix timeline**: depends on severity and complexity, typically within 30–90 days

## Scope

This policy applies to:

- The Nebula workflow engine source code in this repository
- Official builds and artifacts published from this repository (e.g. GitHub Releases, container images managed by the maintainer)

Out of scope:

- Third-party dependencies (they should be reported upstream)
- Forks and modified versions of Nebula that are not maintained by the repository owner

## Supported Versions

Nebula is under active development. The following policy applies:

| Version               | Supported |
|----------------------|-----------|
| `main` branch        | Yes       |
| Latest tagged release| Yes       |
| Older releases       | No        |

## Security Boundaries

Nebula's main trust boundaries:

- **Plugin model** — plugins run **in-process** (ADR-0091): a plugin is a Rust
  crate linked into the host, registered through `nebula-plugin`. There is no
  process/WASM isolation boundary today — untrusted third-party native code is
  out of scope (canon §12.6). Trust is established at build/dependency time, not
  at runtime.
- **Credential boundary** — `nebula-credential` owns credential material
  (contract + lifecycle runtime, consolidated per ADR-0092). Other crates receive
  `CredentialGuard<C>` (which is `!Clone` and zeroizes on drop via a manual
  `Drop` impl) and read through it, never around it.
- **API boundary** — `nebula-api` is the external HTTP surface.
  Authentication / authorisation lives there; lower layers assume their
  inputs were checked.
- **Tenancy boundary** — `nebula-tenancy` decorates storage adapters; every
  read/write substitutes the active tenant scope before it reaches a
  handler.

## Secret Handling Rules

- **Never `Debug`-print a `CredentialGuard`, `SlotCell<CredentialGuard<_>>`,
  or a `Runtime` that holds authenticated state.** Derive a redacted
  `Debug` impl that omits connection strings, tokens, and internal buffers.
- **Never log raw error messages from a third-party driver that may embed
  a connection string or token.** Wrap them in your `Resource::Error`
  enum's variants with descriptive (but secret-free) messages.
- **`ResourceEvent::AcquireFailed { error }` / `SlotRefreshFailed { error }`
  are already redacted** by contract — the engine derives the string from
  the typed `ErrorKind`. Do not bypass `ClassifyError` and substitute the
  raw driver message.

## Logging and Telemetry Rules

- Use `tracing` spans on the lifecycle hot path (`create`, `recycle`,
  `destroy`, `refresh_slot`, `revoke_slot`).
- Span fields MUST NOT contain credential material. Use stable keys like
  `resource.key`, `scope.level`, `slot.name` — never raw `auth_token` etc.
- The broadcast `ResourceEvent` channel drops oldest events on overflow.
  External subscribers MUST handle `RecvError::Lagged` rather than treat
  it as a fault.

## Unsafe Code Policy

- Library crates carry `#![forbid(unsafe_code)]` at crate root. Adding
  `unsafe` requires a separately-justified PR + reviewer sign-off.
- Cross-crate `unsafe` API surfaces require an ADR.
- `nebula-resource` is currently `#![forbid(unsafe_code)]`. There is no
  unsafe code in the crate.

