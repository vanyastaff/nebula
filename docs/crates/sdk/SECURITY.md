# Security

## Threat Model

- **Assets:** SDK is library for authors; no runtime secrets. TestContext/MockExecution may hold mock credentials (test only); not for production.
- **Trust boundaries:** Author code runs in their tests or in engine/runtime; sdk does not execute user code in production.
- **Attacker capabilities:** Malicious author code is out of scope (they own their crate); sdk must not expose unsafe or credential leakage in test mocks beyond intended test use.

## Security Controls

- **Authn/authz:** N/A in sdk. Test mocks (MockToken, etc.) are for test only; document that they must not be used in production.
- **Secret handling:** No real secrets in sdk; mock credentials in testing module are clearly test-only.
- **Input validation:** Builders validate input where applicable; invalid input returns error, no panic or unsafe.

## Abuse Cases

- **Test mock in production:** Prevention: document clearly that TestContext and mocks are for testing only. Detection: code review. Response: N/A (author responsibility).
- **Unsafe or panic in builder:** Prevention: validate inputs; no unsafe in sdk. Detection: tests and fuzz. Response: fix and release patch.

## Security Requirements

- **Must-have:** No unsafe in sdk (or gated and documented); test mocks documented as test-only.
- **Should-have:** Builders reject invalid input with error, not panic.

## Security Test Plan

- Unit tests: builders reject invalid input; no panic on malformed data.
- No fuzz requirement for sdk (low attack surface).
