# Security

## Threat Model

- **Assets:** Plugin metadata (keys, names, docs URLs); credential descriptions (no secrets); handler references. Loaded plugin code (dynamic-loading) — potential malicious libraries.
- **Trust boundaries:** Plugin author → registry; dynamic loader → host process.
- **Attacker capabilities:** Malicious plugin in dynamic load; metadata injection (XSS if rendered unsanitized); key collision to replace legitimate plugin.

## Security Controls

- **Authn/authz:** Plugin crate has none; runtime/registry own access control. Plugin metadata is not sensitive.
- **Isolation/sandboxing:** Dynamic-loaded plugins run in host process; no sandbox in plugin crate. Runtime/sandbox crate enforces execution boundaries.
- **Secret handling:** Plugin does not store secrets; `CredentialDescription` declares requirements only. Credential crate owns secret lifecycle.
- **Input validation:** `PluginKey` normalized via core; invalid keys rejected at `PluginMetadata::build()`.

## Abuse Cases

- **Malicious dynamic plugin:**
  - Prevention: Load only from trusted paths; verify signatures (future)
  - Detection: Load failures; runtime monitoring
  - Response: Reject load; alert operator
- **Key collision (replace plugin):**
  - Prevention: `register()` fails if key exists; `register_or_replace` is explicit
  - Detection: Registry audit
  - Response: Use `register()` to block overwrites
- **Metadata XSS (if rendered in UI):**
  - Prevention: UI must sanitize/escape plugin name, description, docs URL
  - Detection: N/A
  - Response: N/A (plugin crate does not render)

## Security Requirements

- **Must-have:** PluginKey validation; no secret storage in plugin
- **Should-have:** Dynamic loader path restriction; optional signature verification

## Security Test Plan

- **Static analysis:** `cargo audit`; no unsafe in plugin crate (except loader, scoped)
- **Dynamic tests:** Reject invalid keys; reject duplicate register when using `register()`
- **Fuzz/property tests:** Key normalization idempotence
