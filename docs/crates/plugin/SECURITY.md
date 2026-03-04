# Security

## Threat Model

- **Assets:** Registry holds plugin metadata and component refs; no secrets. Dynamic-loaded code (if feature enabled) runs in process; loader is trust boundary.
- **Trust boundaries:** Plugin crate trusts Plugin impls (static or from loader). Dynamic loading: caller is responsible for loading only trusted libs; loader does not sandbox loaded code.
- **Attacker capabilities:** Malicious plugin binary (dynamic-loading): could run arbitrary code if loaded. Malicious metadata: could confuse UI or engine if not validated.

## Security Controls

- **Authn/authz:** N/A in plugin crate. API/caller decides who can register or load plugins.
- **Isolation:** No sandbox in plugin crate; dynamic-loaded code runs in same process. Sandbox (if any) is in runtime/sandbox crate for action execution.
- **Input validation:** Metadata builder validates key/name/version where applicable; invalid key or duplicate key fails register.
- **Secret handling:** None; plugin declares credential refs only, does not hold secrets.

## Abuse Cases

- **Load malicious .so:** Prevention: document that loader must only load trusted paths; operator responsibility. Detection: N/A in crate. Response: do not enable dynamic-loading in untrusted environments.
- **Duplicate key DoS:** Prevention: register returns AlreadyExists; caller handles. No unbounded growth from duplicate attempts.
- **Malformed metadata:** Prevention: builder validates; invalid metadata fails build or register.

## Security Requirements

- **Must-have:** No unsafe in default build; dynamic-loading feature documents ABI and trust model. Register rejects duplicate key.
- **Should-have:** Metadata validation (key format, non-empty name) in builder.

## Security Test Plan

- Unit tests: duplicate key fails; invalid metadata fails build. Default build has no unsafe (audit or cargo geiger).
- With dynamic-loading: load valid lib succeeds; load invalid path or broken lib fails with clear error.
