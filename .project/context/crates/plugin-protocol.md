# nebula-plugin-protocol
Typed stdin/stdout JSON protocol for process-isolated community plugins.

## Invariants
- Plugin authors depend ONLY on this crate — not on nebula-action or nebula-core.
- Protocol is request/response over stdin/stdout: one `PluginRequest` in, one `PluginResponse` out.
- `__metadata__` is a reserved action key — returns `PluginMetadata` with plugin info and action list.
- Tagged enum serialization (`#[serde(tag = "status")]`) — no ambiguity between Ok and Error.

## Key Decisions
- Separate from nebula-action: plugins don't need the full action trait system. They implement `PluginHandler` (metadata + execute) instead.
- `PluginResponse::Error` carries `retryable: bool` — maps to `ActionError::Retryable` / `Fatal` on host side.
- `run()` entry point handles the protocol loop — plugin author just implements the trait.
- String-based error codes (e.g., "TIMEOUT", "RATE_LIMIT") — no dependency on `ErrorCode` enum.
- Minimal dependencies: only `serde` + `serde_json`.

## Traps
- `run()` panics on broken stdin/stdout — intentional, a plugin with broken I/O cannot function.
- Protocol version must match between host and plugin — no version negotiation yet.
- Action keys in plugin-protocol use full `plugin.action` format (e.g., "telegram.send_message"), unlike nebula-action where ActionKey is just the action name.

## Relations
- Used by community plugin binaries (external crate consumers).
- Host side (nebula-sandbox) deserializes the same types.
- Does NOT depend on nebula-action, nebula-core, or any other workspace crate.
