# Desktop App — Plugin Reference

All Tauri official and community plugins evaluated for Nebula desktop.

---

## Tier 1 — Essential (use from day one)

| Plugin | Source | Purpose |
|--------|--------|---------|
| `tauri-plugin-store` | Official | Secure persistent storage (auth token, connection config) |
| `tauri-plugin-deep-link` | Official | `nebula://auth/callback` OAuth redirect |
| `tauri-plugin-opener` | Official | Open OAuth URL in system browser |
| `tauri-plugin-single-instance` | Official | Prevent duplicate windows, focus existing on reopen |
| `tauri-plugin-notification` | Official | Run failure alerts, workflow status notifications |
| `tauri-plugin-updater` | Official | In-app auto-update |
| `tauri-plugin-window-state` | Official | Remember window size + position across restarts (1 line to enable) |
| `tauri-plugin-websocket` | Official | Live execution log streaming (polling is a fallback, not a solution) |
| `tauri-specta` | Community | Auto-generate TypeScript types from Rust command signatures |

### Why `window-state` is in Tier 1

It takes literally one line in `lib.rs` and makes the app feel native immediately.
Without it every restart opens at the default position.

### Why `websocket` is in Tier 1

Execution monitoring (Phase 3) needs real-time log streaming. TanStack Query `refetchInterval`
is a fallback for APIs that don't support WebSocket. If the Nebula backend supports WS/SSE
for run events, the desktop should use it from the start, not retrofit later.

---

## Tier 2 — High value (add in Phase 2–3)

| Plugin | Source | Purpose | Phase |
|--------|--------|---------|-------|
| `tauri-plugin-dialog` | Official | Native file picker for workflow JSON import/export | 2 |
| `tauri-plugin-global-shortcut` | Official | Keyboard shortcuts (new workflow, run, zoom canvas) | 2 |
| `tauri-plugin-clipboard-manager` | Official | Copy workflow ID, node output values, execution logs | 3 |
| `tauri-plugin-log` | Official | Structured logging from Rust — visible in DevTools + file | 1 |
| `sentry-tauri` | Community | Captures JS errors + Rust panics + native crash dumps to Sentry | 2 |

### Notes

**`tauri-plugin-log`** integrates with the `tracing` crate (which Nebula already uses across
all Rust crates). Logs appear in DevTools console during development and write to a file
in production. Add early — harder to retrofit.

**`sentry-tauri`** is the only plugin that captures Rust panics + JS errors + native crash
dumps in one place. Critical for production. Unique to Tauri ecosystem.

---

## Tier 3 — Evaluate later

| Plugin | Source | Purpose | When |
|--------|--------|---------|------|
| `tauri-plugin-stronghold` | Official | Hardware-backed encrypted storage (alternative to plugin-store for credentials) | Phase 4 |
| `tauri-plugin-sql` | Official | Embed SQLite for local workflow caching / offline support | Phase 4–5 |
| `tauri-plugin-aptabase` | Community | Privacy-first analytics (no PII, open-source backend) | Phase 5 |
| `tauri-plugin-tracing` | Community | Bridges JS tracing to Rust `tracing` crate, flamegraph support | Phase 3 |

### `stronghold` vs `store` for auth

`plugin-store` writes JSON to disk, protected by OS file permissions.
`stronghold` uses the IOTA Stronghold engine with hardware-backed encryption.

For **auth tokens** — `plugin-store` is fine. OS file permissions protect it adequately.
For **user credentials** (API keys, OAuth secrets stored in the app) — consider `stronghold`.
Decision: defer to Phase 4 when credential management is built.

---

## Rejected / Not Applicable

| Plugin | Reason |
|--------|--------|
| `barcode-scanner`, `biometric`, `geolocation`, `haptics`, `nfc` | Mobile only |
| `tauri-plugin-localhost` | App serves its own backend, not needed |
| `tauri-plugin-persisted-scope` | File system access scope — not relevant |
| `tauri-plugin-shell` | Only if embedding Nebula backend in-process (Phase 5+ idea) |
| `tauri-plugin-os` | Tauri already exposes OS info via `@tauri-apps/api/os` |
| `tauri-plugin-upload` | No file upload UX planned |
| `tauri-plugin-fs` | Covered by `dialog` + direct fetch for workflow import/export |

---

## Current `Cargo.toml` state

```toml
# Tier 1 — already added or in plan
tauri-plugin-store        = "2.2"
tauri-plugin-deep-link    = "2.0"
tauri-plugin-opener       = "2.0"
tauri-plugin-single-instance = { version = "2.4.0", features = ["deep-link"] }

# Tier 1 — add next
tauri-plugin-notification = "2.0"
tauri-plugin-updater      = "2.0"
tauri-plugin-window-state = "2.0"
tauri-plugin-websocket    = "2.0"
tauri-plugin-log          = "2.0"

# Tier 2 — add in Phase 2-3
tauri-plugin-dialog           = "2.0"
tauri-plugin-global-shortcut  = "2.0"
tauri-plugin-clipboard-manager = "2.0"
sentry                        = { version = "0.34", features = ["backtrace", "contexts", "panic"] }
sentry-tauri                  = "0.3"
```
