# Desktop Architecture Foundation — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Build a clean Tauri desktop architecture with typed IPC (tauri-specta), typed errors (AppError),
service layer (AuthService / ConnectionService), AppState, typed events, Zustand stores, and TanStack Query.

**Architecture:**
- Rust: thin commands → service layer → models. AppState holds Arc<Service>. Typed errors + typed events.
- TypeScript: generated bindings from specta. Zustand for Rust-backed state. TanStack Query for HTTP API data.

**Tech Stack:** Tauri 2, tauri-specta 0.20, tauri-plugin-store 2, tauri-plugin-window-state 2,
reqwest 0.12, url 2, thiserror 2 — React 18, Zustand 5, TanStack Query 5, TypeScript 5.

---

## Final src-tauri structure

```
src-tauri/src/
  commands/
    mod.rs
    auth.rs          ← 3-line thin handlers
    connection.rs    ← 3-line thin handlers
  services/
    mod.rs
    auth.rs          ← AuthService — all business logic
    connection.rs    ← ConnectionService — all business logic
  events/
    mod.rs
    auth.rs          ← AuthStateChanged (typed via tauri-specta)
  models/
    mod.rs
    auth.rs          ← AuthState, AuthStatus, UserProfile
    connection.rs    ← ConnectionConfig, ConnectionMode
  error.rs           ← AppError: Serialize + Type
  state.rs           ← AppState { auth, connection }
  deep_link.rs       ← uses &AppState, no AppHandle
  lib.rs             ← wires everything
  main.rs
```

---

## Task 1: Add Rust dependencies

**Files:**
- Modify: `apps/desktop/src-tauri/Cargo.toml`

**Step 1: Replace `[dependencies]` section**

```toml
[dependencies]
tauri                    = { version = "2.0", features = [] }
tauri-plugin-opener      = "2.0"
tauri-plugin-deep-link   = "2.0"
tauri-plugin-single-instance = { version = "2.4.0", features = ["deep-link"] }
tauri-plugin-store       = "2.2"
tauri-plugin-window-state = "2.0"
tauri-specta             = { version = "0.20", features = ["derive", "typescript"] }
specta                   = { version = "2", features = ["derive"] }
specta-typescript        = "0.0"
reqwest                  = { version = "0.12", features = ["json"] }
url                      = "2"
thiserror                = "2"
serde                    = { version = "1", features = ["derive"] }
serde_json               = "1"
```

**Step 2: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

Expected: no errors.

**Step 3: Commit**

```bash
git add apps/desktop/src-tauri/Cargo.toml
git commit -m "feat(desktop): add all Rust deps — specta, store, window-state, reqwest"
```

---

## Task 2: Add npm dependencies

**Files:**
- Modify: `apps/desktop/package.json`

**Step 1: Install**

```bash
cd apps/desktop
npm install zustand @tanstack/react-query @tauri-apps/plugin-store
```

**Step 2: Verify**

```bash
npm run build
```

**Step 3: Commit**

```bash
git add apps/desktop/package.json apps/desktop/package-lock.json
git commit -m "feat(desktop): add Zustand, TanStack Query, plugin-store"
```

---

## Task 3: error.rs — typed AppError

**Files:**
- Create: `apps/desktop/src-tauri/src/error.rs`

**Step 1: Write file**

```rust
use serde::Serialize;
use specta::Type;

/// Single error type for all Tauri commands.
/// Serializes as `{ kind: "...", message: "..." }` — TypeScript can discriminate.
#[derive(Debug, thiserror::Error, Serialize, Type)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum AppError {
    #[error("store: {0}")]
    Store(String),

    #[error("network: {0}")]
    Network(String),

    #[error("auth: {0}")]
    Auth(String),
}
```

**Step 2: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 3: Commit**

```bash
git add apps/desktop/src-tauri/src/error.rs
git commit -m "feat(desktop): typed AppError with Serialize + Type"
```

---

## Task 4: models/ — IPC types

**Files:**
- Create: `apps/desktop/src-tauri/src/models/mod.rs`
- Create: `apps/desktop/src-tauri/src/models/auth.rs`
- Create: `apps/desktop/src-tauri/src/models/connection.rs`

**Step 1: `models/mod.rs`**

```rust
pub mod auth;
pub mod connection;
```

**Step 2: `models/auth.rs`**

```rust
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub status: AuthStatus,
    pub provider: Option<String>,
    pub access_token: String,
    pub user: Option<UserProfile>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    SignedOut,
    Authorizing,
    SignedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}
```

**Step 3: `models/connection.rs`**

```rust
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub mode: ConnectionMode,
    pub local_base_url: String,
    pub remote_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    Local,
    Remote,
}
```

**Step 4: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/models/
git commit -m "feat(desktop): models/auth and models/connection with specta types"
```

---

## Task 5: events/ — typed Tauri events

**Files:**
- Create: `apps/desktop/src-tauri/src/events/mod.rs`
- Create: `apps/desktop/src-tauri/src/events/auth.rs`

**Step 1: `events/mod.rs`**

```rust
pub mod auth;
```

**Step 2: `events/auth.rs`**

```rust
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

use crate::models::auth::AuthState;

/// Emitted by Rust whenever auth state changes.
/// React listens via the generated typed `events.authStateChanged.listen(...)`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct AuthStateChanged(pub AuthState);
```

**Step 3: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/events/
git commit -m "feat(desktop): typed AuthStateChanged event via tauri-specta"
```

---

## Task 6: services/connection.rs — ConnectionService

**Files:**
- Create: `apps/desktop/src-tauri/src/services/mod.rs`
- Create: `apps/desktop/src-tauri/src/services/connection.rs`

**Step 1: `services/mod.rs`**

```rust
pub mod auth;
pub mod connection;
```

**Step 2: `services/connection.rs`**

```rust
use serde_json::json;
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::{
    error::AppError,
    models::connection::{ConnectionConfig, ConnectionMode},
};

const STORE_PATH: &str = "nebula-connection.json";
const KEY: &str = "connection";

pub struct ConnectionService {
    app: AppHandle,
}

impl ConnectionService {
    pub fn new(app: AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }

    fn default_config() -> ConnectionConfig {
        ConnectionConfig {
            mode: ConnectionMode::Local,
            local_base_url: "http://localhost:5678".to_string(),
            remote_base_url: String::new(),
        }
    }

    pub fn get(&self) -> ConnectionConfig {
        let Ok(store) = self.app.store(STORE_PATH) else {
            return Self::default_config();
        };
        store
            .get(KEY)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_else(Self::default_config)
    }

    pub fn set(&self, config: ConnectionConfig) -> Result<(), AppError> {
        let store = self
            .app
            .store(STORE_PATH)
            .map_err(|e| AppError::Store(e.to_string()))?;
        store.set(KEY, json!(config));
        store.save().map_err(|e| AppError::Store(e.to_string()))
    }

    /// Returns the currently active base URL based on connection mode.
    pub fn active_url(&self) -> String {
        let config = self.get();
        match config.mode {
            ConnectionMode::Local => config.local_base_url,
            ConnectionMode::Remote => config.remote_base_url,
        }
    }
}
```

**Step 3: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/services/
git commit -m "feat(desktop): ConnectionService with plugin-store persistence"
```

---

## Task 7: services/auth.rs — AuthService

**Files:**
- Modify: `apps/desktop/src-tauri/src/services/auth.rs` (create)

**Step 1: Write file**

```rust
use serde_json::json;
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_store::StoreExt;
use tauri_specta::EventExt;

use crate::{
    error::AppError,
    events::auth::AuthStateChanged,
    models::auth::{AuthState, AuthStatus, UserProfile},
    services::connection::ConnectionService,
};

const STORE_PATH: &str = "nebula-auth.json";
const KEY: &str = "auth";

pub struct AuthService {
    app: AppHandle,
    connection: Arc<ConnectionService>,
}

impl AuthService {
    pub fn new(app: AppHandle, connection: Arc<ConnectionService>) -> Arc<Self> {
        Arc::new(Self { app, connection })
    }

    /// Loads persisted auth state. Never returns `Authorizing` — resets it on load.
    pub fn load(&self) -> AuthState {
        let fallback = AuthState {
            status: AuthStatus::SignedOut,
            provider: None,
            access_token: String::new(),
            user: None,
            error: None,
        };
        let Ok(store) = self.app.store(STORE_PATH) else {
            return fallback;
        };
        let mut state: AuthState = store
            .get(KEY)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(fallback);
        if state.status == AuthStatus::Authorizing {
            state.status = AuthStatus::SignedOut;
        }
        state
    }

    fn save(&self, state: &AuthState) -> Result<(), AppError> {
        let store = self
            .app
            .store(STORE_PATH)
            .map_err(|e| AppError::Store(e.to_string()))?;
        store.set(KEY, json!(state));
        store.save().map_err(|e| AppError::Store(e.to_string()))
    }

    fn emit(&self, state: &AuthState) -> Result<(), AppError> {
        AuthStateChanged(state.clone())
            .emit(&self.app)
            .map_err(|e| AppError::Auth(e.to_string()))
    }

    fn save_and_emit(&self, state: &AuthState) -> Result<(), AppError> {
        self.save(state)?;
        self.emit(state)
    }

    /// Starts OAuth flow: calls backend /auth/oauth/start, opens browser.
    pub async fn start_oauth(
        &self,
        provider: String,
        api_base_url: String,
    ) -> Result<(), AppError> {
        let mut state = self.load();
        state.status = AuthStatus::Authorizing;
        state.provider = Some(provider.clone());
        state.error = None;
        self.save_and_emit(&state)?;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{api_base_url}/auth/oauth/start"))
            .json(&json!({
                "provider": provider,
                "redirectUri": "nebula://auth/callback"
            }))
            .send()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let msg = format!("oauth start failed: {}", response.status());
            let mut s = self.load();
            s.status = AuthStatus::SignedOut;
            s.error = Some(msg.clone());
            self.save_and_emit(&s)?;
            return Err(AppError::Auth(msg));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;

        // Backend returned token directly (mock / test flow)
        if let Some(token) = payload.get("accessToken").and_then(|v| v.as_str()) {
            let user = payload
                .get("user")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            return self.complete_sign_in(token.to_string(), Some(provider), user);
        }

        // Backend returned OAuth URL — open browser, wait for deep-link
        if let Some(url) = payload.get("authUrl").and_then(|v| v.as_str()) {
            self.app
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| AppError::Auth(e.to_string()))?;
        }

        Ok(())
    }

    /// Exchanges OAuth authorization code for access token via backend.
    /// Called by deep_link handler — uses connection.active_url(), no apiBaseUrl arg.
    pub async fn exchange_code(&self, code: String, provider: String) -> Result<(), AppError> {
        let api_base_url = self.connection.active_url();

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{api_base_url}/auth/oauth/callback"))
            .json(&json!({
                "provider": provider,
                "code": code,
                "redirectUri": "nebula://auth/callback"
            }))
            .send()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or(json!({}));
            let detail = body
                .get("message")
                .or_else(|| body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let msg = if detail.is_empty() {
                format!("oauth callback failed: {status}")
            } else {
                format!("oauth callback failed: {status} ({detail})")
            };
            let mut s = self.load();
            s.status = AuthStatus::SignedOut;
            s.error = Some(msg.clone());
            self.save_and_emit(&s)?;
            return Err(AppError::Auth(msg));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Network(e.to_string()))?;

        let token = payload
            .get("accessToken")
            .or_else(|| payload.get("access_token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let user: Option<UserProfile> = payload
            .get("user")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        self.complete_sign_in(token, Some(provider), user)
    }

    /// Finalizes sign-in. Used by both start_oauth (mock flow) and exchange_code.
    pub fn complete_sign_in(
        &self,
        token: String,
        provider: Option<String>,
        user: Option<UserProfile>,
    ) -> Result<(), AppError> {
        let token = token.trim().to_string();
        let status = if token.is_empty() {
            AuthStatus::SignedOut
        } else {
            AuthStatus::SignedIn
        };
        self.save_and_emit(&AuthState {
            status,
            provider,
            access_token: token,
            user,
            error: None,
        })
    }

    pub fn sign_out(&self) -> Result<(), AppError> {
        self.save_and_emit(&AuthState {
            status: AuthStatus::SignedOut,
            provider: None,
            access_token: String::new(),
            user: None,
            error: None,
        })
    }

    pub fn set_error(&self, message: String) -> Result<(), AppError> {
        let mut state = self.load();
        state.status = AuthStatus::SignedOut;
        state.access_token = String::new();
        state.user = None;
        state.error = Some(message);
        self.save_and_emit(&state)
    }
}
```

**Step 2: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 3: Commit**

```bash
git add apps/desktop/src-tauri/src/services/auth.rs
git commit -m "feat(desktop): AuthService with typed errors and events"
```

---

## Task 8: state.rs — AppState

**Files:**
- Create: `apps/desktop/src-tauri/src/state.rs`

**Step 1: Write file**

```rust
use std::sync::Arc;

use crate::services::{auth::AuthService, connection::ConnectionService};

/// Shared application state — injected into Tauri commands via State<'_, AppState>.
/// Both services are Arc — cheap to clone when passing to async tasks.
pub struct AppState {
    pub auth: Arc<AuthService>,
    pub connection: Arc<ConnectionService>,
}
```

**Step 2: Commit**

```bash
git add apps/desktop/src-tauri/src/state.rs
git commit -m "feat(desktop): AppState holding service Arcs"
```

---

## Task 9: commands/ — thin handlers

**Files:**
- Create: `apps/desktop/src-tauri/src/commands/mod.rs`
- Create: `apps/desktop/src-tauri/src/commands/auth.rs`
- Create: `apps/desktop/src-tauri/src/commands/connection.rs`

**Step 1: `commands/mod.rs`**

```rust
pub mod auth;
pub mod connection;
```

**Step 2: `commands/auth.rs`**

```rust
use tauri::State;

use crate::{error::AppError, models::auth::AuthState, state::AppState};

#[tauri::command]
#[specta::specta]
pub async fn get_auth_state(state: State<'_, AppState>) -> AuthState {
    state.auth.load()
}

#[tauri::command]
#[specta::specta]
pub async fn start_oauth(
    provider: String,
    api_base_url: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state.auth.start_oauth(provider, api_base_url).await
}

#[tauri::command]
#[specta::specta]
pub async fn sign_out(state: State<'_, AppState>) -> Result<(), AppError> {
    state.auth.sign_out()
}
```

**Step 3: `commands/connection.rs`**

```rust
use tauri::State;

use crate::{error::AppError, models::connection::ConnectionConfig, state::AppState};

#[tauri::command]
#[specta::specta]
pub async fn get_connection(state: State<'_, AppState>) -> ConnectionConfig {
    state.connection.get()
}

#[tauri::command]
#[specta::specta]
pub async fn set_connection(
    config: ConnectionConfig,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    state.connection.set(config)
}
```

**Step 4: Verify**

```bash
cd apps/desktop/src-tauri && cargo check
```

**Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/commands/
git commit -m "feat(desktop): thin command handlers delegating to services"
```

---

## Task 10: deep_link.rs — uses AppState

**Files:**
- Create: `apps/desktop/src-tauri/src/deep_link.rs`

**Step 1: Write file**

```rust
use crate::state::AppState;

/// Parses nebula://auth/callback URLs and drives the auth service.
/// Called from lib.rs setup — receives &AppState, not raw AppHandle.
pub async fn handle(raw_url: &str, state: &AppState) {
    let Ok(parsed) = raw_url.parse::<url::Url>() else {
        return;
    };

    if parsed.scheme() != "nebula" || parsed.host_str() != Some("auth") {
        return;
    }

    if parsed.path().trim_end_matches('/') != "/callback" {
        return;
    }

    let params: std::collections::HashMap<String, String> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Flow 1: direct token (legacy / test mock)
    if let Some(token) = params
        .get("access_token")
        .or_else(|| params.get("token"))
        .filter(|t| !t.is_empty())
    {
        let provider = params.get("provider").cloned();
        let _ = state.auth.complete_sign_in(token.clone(), provider, None);
        return;
    }

    // Flow 2: OAuth code exchange
    let Some(code) = params.get("code").cloned() else {
        return;
    };

    let Some(provider) = params.get("provider").cloned() else {
        let _ = state
            .auth
            .set_error("OAuth callback missing provider parameter.".to_string());
        return;
    };

    let _ = state.auth.exchange_code(code, provider).await;
}
```

**Step 2: Commit**

```bash
git add apps/desktop/src-tauri/src/deep_link.rs
git commit -m "feat(desktop): deep_link handler uses AppState, not raw AppHandle"
```

---

## Task 11: lib.rs — wire everything

**Files:**
- Replace: `apps/desktop/src-tauri/src/lib.rs`

**Step 1: Write file**

```rust
mod commands;
mod deep_link;
mod error;
mod events;
mod models;
mod services;
mod state;

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_specta::{collect_commands, collect_events, Builder};

use commands::auth::{get_auth_state, sign_out, start_oauth};
use commands::connection::{get_connection, set_connection};
use events::auth::AuthStateChanged;
use services::{auth::AuthService, connection::ConnectionService};
use state::AppState;

#[tauri::command]
#[specta::specta]
fn get_api_profile() -> String {
    std::env::var("NEBULA_API_PROFILE").unwrap_or_else(|_| "local".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            get_api_profile,
            get_auth_state,
            start_oauth,
            sign_out,
            get_connection,
            set_connection,
        ])
        .events(collect_events![AuthStateChanged]);

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default()
                .header("// Auto-generated by tauri-specta — do not edit manually.\n"),
            "../src/bindings.ts",
        )
        .expect("Failed to export TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            // Build services — connection first, auth depends on connection
            let connection = ConnectionService::new(app.handle().clone());
            let auth = AuthService::new(app.handle().clone(), Arc::clone(&connection));
            app.manage(AppState { auth, connection });

            // Register deep-link scheme on platforms that need it
            #[cfg(any(windows, target_os = "linux"))]
            if let Err(err) = app.deep_link().register_all() {
                eprintln!("failed to register deep-link schemes: {err}");
            }

            // Handle deep-links — route to AppState, not raw handle
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let handle = handle.clone();
                let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    for url in urls {
                        deep_link::handle(&url, &state).await;
                    }
                });
            });

            Ok(())
        })
        .invoke_handler(builder.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**Step 2: Full Rust build + generate bindings**

```bash
cd apps/desktop/src-tauri && cargo build
```

Expected: compiles. Check `apps/desktop/src/bindings.ts` was created.

**Step 3: Commit**

```bash
git add apps/desktop/src-tauri/src/lib.rs apps/desktop/src/bindings.ts
git commit -m "feat(desktop): wire lib.rs — specta, services, AppState, window-state"
```

---

## Task 12: Zustand auth store

**Files:**
- Create: `apps/desktop/src/features/auth/store.ts`

**Step 1: Write file**

```typescript
import { create } from "zustand";
import { events, commands } from "../../bindings";

export type AuthStatus = "signed_out" | "authorizing" | "signed_in";

export interface AuthUser {
  id: string;
  login: string;
  name?: string;
  email?: string;
  avatarUrl?: string;
}

interface AuthState {
  status: AuthStatus;
  provider?: string;
  accessToken: string;
  user?: AuthUser;
  error?: string;
  initialized: boolean;
}

interface AuthActions {
  initialize: () => Promise<void>;
  startOAuth: (provider: string, apiBaseUrl: string) => Promise<void>;
  signOut: () => Promise<void>;
}

export const useAuthStore = create<AuthState & AuthActions>((set) => ({
  status: "signed_out",
  accessToken: "",
  initialized: false,

  initialize: async () => {
    const raw = await commands.getAuthState();
    set({ ...toState(raw), initialized: true });

    // Typed event — no string literals, no any
    await events.authStateChanged.listen((event) => {
      set(toState(event.payload));
    });
  },

  startOAuth: async (provider, apiBaseUrl) => {
    await commands.startOAuth(provider, apiBaseUrl);
  },

  signOut: async () => {
    await commands.signOut();
  },
}));

function toState(raw: {
  status: string;
  provider?: string | null;
  accessToken: string;
  user?: AuthUser | null;
  error?: string | null;
}): Omit<AuthState, "initialized"> {
  return {
    status: raw.status as AuthStatus,
    provider: raw.provider ?? undefined,
    accessToken: raw.accessToken,
    user: raw.user ?? undefined,
    error: raw.error ?? undefined,
  };
}
```

**Step 2: Commit**

```bash
git add apps/desktop/src/features/auth/
git commit -m "feat(desktop): Zustand auth store with typed events"
```

---

## Task 13: Zustand connection store

**Files:**
- Create: `apps/desktop/src/features/connection/store.ts`

**Step 1: Write file**

```typescript
import { create } from "zustand";
import { commands } from "../../bindings";

export type ConnectionMode = "local" | "remote";

export interface ConnectionConfig {
  mode: ConnectionMode;
  localBaseUrl: string;
  remoteBaseUrl: string;
}

interface ConnectionState {
  config: ConnectionConfig;
  activeBaseUrl: string;
  initialized: boolean;
}

interface ConnectionActions {
  initialize: () => Promise<void>;
  setMode: (mode: ConnectionMode) => Promise<void>;
  setLocalBaseUrl: (url: string) => Promise<void>;
  setRemoteBaseUrl: (url: string) => Promise<void>;
}

const DEFAULT: ConnectionConfig = {
  mode: "local",
  localBaseUrl: "http://localhost:5678",
  remoteBaseUrl: "",
};

function active(cfg: ConnectionConfig): string {
  return cfg.mode === "local" ? cfg.localBaseUrl : cfg.remoteBaseUrl;
}

function trim(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export const useConnectionStore = create<ConnectionState & ConnectionActions>(
  (set, get) => ({
    config: DEFAULT,
    activeBaseUrl: DEFAULT.localBaseUrl,
    initialized: false,

    initialize: async () => {
      const raw = await commands.getConnection();
      const config: ConnectionConfig = {
        mode: raw.mode as ConnectionMode,
        localBaseUrl: raw.localBaseUrl,
        remoteBaseUrl: raw.remoteBaseUrl,
      };
      set({ config, activeBaseUrl: active(config), initialized: true });
    },

    setMode: async (mode) => {
      const config = { ...get().config, mode };
      await commands.setConnection({ mode, localBaseUrl: config.localBaseUrl, remoteBaseUrl: config.remoteBaseUrl });
      set({ config, activeBaseUrl: active(config) });
    },

    setLocalBaseUrl: async (url) => {
      const config = { ...get().config, localBaseUrl: trim(url) };
      await commands.setConnection({ mode: config.mode, localBaseUrl: config.localBaseUrl, remoteBaseUrl: config.remoteBaseUrl });
      set({ config, activeBaseUrl: active(config) });
    },

    setRemoteBaseUrl: async (url) => {
      const config = { ...get().config, remoteBaseUrl: trim(url) };
      await commands.setConnection({ mode: config.mode, localBaseUrl: config.localBaseUrl, remoteBaseUrl: config.remoteBaseUrl });
      set({ config, activeBaseUrl: active(config) });
    },
  })
);
```

**Step 2: Commit**

```bash
git add apps/desktop/src/features/connection/
git commit -m "feat(desktop): Zustand connection store backed by Rust"
```

---

## Task 14: app/providers.tsx + main.tsx

**Files:**
- Create: `apps/desktop/src/app/providers.tsx`

Note: `src/main.tsx` was already updated by linter to include `<Providers>`. Verify it matches:

```tsx
// main.tsx — should already look like this
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./ui/App";
import { Providers } from "./app/providers";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers>
      <App />
    </Providers>
  </React.StrictMode>
);
```

**Step 1: Create `app/providers.tsx`**

```tsx
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { useAuthStore } from "../features/auth/store";
import { useConnectionStore } from "../features/connection/store";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 30_000 },
  },
});

export function Providers({ children }: { children: React.ReactNode }) {
  const initAuth = useAuthStore((s) => s.initialize);
  const initConnection = useConnectionStore((s) => s.initialize);

  useEffect(() => {
    // Connection first — auth.startOAuth needs activeBaseUrl
    void initConnection().then(() => initAuth());
  }, [initAuth, initConnection]);

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}
```

**Step 2: Commit**

```bash
git add apps/desktop/src/app/providers.tsx
git commit -m "feat(desktop): Providers — QueryClient + ordered store init"
```

---

## Task 15: Update App.tsx

**Files:**
- Replace: `apps/desktop/src/ui/App.tsx`

**Step 1: Replace with Zustand version**

```tsx
import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { useAuthStore } from "../features/auth/store";
import { useConnectionStore } from "../features/connection/store";

export function App() {
  const auth = useAuthStore();
  const { activeBaseUrl } = useConnectionStore();
  const [version, setVersion] = useState("...");

  useEffect(() => {
    getVersion()
      .then((v) => setVersion(v))
      .catch(() => setVersion("0.1.0"));
  }, []);

  return (
    <main
      style={{
        width: "100%",
        minHeight: "100dvh",
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        alignItems: "center",
        position: "relative",
        fontFamily: "'Segoe UI', 'Inter', sans-serif",
        background:
          "radial-gradient(1200px 520px at 20% -10%, #1c2b4f 0%, transparent 55%), radial-gradient(1000px 540px at 95% 110%, #0f3b2f 0%, transparent 60%), #0a0f1d",
        color: "#edf2ff",
      }}
    >
      <section
        style={{
          width: "min(460px, calc(100% - 24px))",
          background: "rgba(14, 20, 38, 0.82)",
          border: "1px solid rgba(151, 165, 198, 0.2)",
          borderRadius: 14,
          padding: "clamp(16px, 3.2vw, 28px)",
          boxShadow: "0 12px 40px rgba(0,0,0,0.35)",
        }}
      >
        <h1 style={{ margin: 0, fontSize: 26 }}>Nebula</h1>
        <p style={{ marginTop: 8, marginBottom: 20, color: "#b8c5e6", fontSize: 14 }}>
          Sign in to continue.
        </p>

        {auth.status === "signed_in" ? (
          <>
            <div style={{ display: "flex", gap: 12, alignItems: "center", marginBottom: 12 }}>
              {auth.user?.avatarUrl && (
                <img
                  src={auth.user.avatarUrl}
                  alt="avatar"
                  width={42}
                  height={42}
                  style={{ borderRadius: "50%", border: "1px solid rgba(184,197,230,0.3)" }}
                />
              )}
              <div>
                <p style={{ margin: 0, fontSize: 14, fontWeight: 600 }}>
                  {auth.user?.name ?? auth.user?.login ?? "Signed in"}
                </p>
                <p style={{ margin: 0, color: "#b8c5e6", fontSize: 12 }}>
                  {auth.user?.email ?? `via ${auth.provider ?? "OAuth"}`}
                </p>
              </div>
            </div>
            <button
              onClick={() => void auth.signOut()}
              style={btnStyle}
            >
              Sign out
            </button>
          </>
        ) : (
          <button
            onClick={() => void auth.startOAuth("github", activeBaseUrl)}
            disabled={auth.status === "authorizing"}
            style={btnStyle}
          >
            Continue with GitHub
          </button>
        )}

        {auth.status === "authorizing" && (
          <p style={{ marginTop: 14, marginBottom: 0, color: "#b8c5e6", fontSize: 13 }}>
            Waiting for OAuth callback…
          </p>
        )}
        {auth.error && (
          <p style={{ marginTop: 14, marginBottom: 0, color: "#ffb7b7", fontSize: 13 }}>
            {auth.error}
          </p>
        )}
      </section>

      <footer style={{ position: "absolute", bottom: 10, textAlign: "center", fontSize: 12, color: "#8ea0cf" }}>
        v{version}
      </footer>
    </main>
  );
}

const btnStyle: React.CSSProperties = {
  width: "100%",
  padding: "11px 14px",
  borderRadius: 10,
  border: "1px solid rgba(184,197,230,0.35)",
  background: "transparent",
  color: "#edf2ff",
  fontWeight: 600,
  cursor: "pointer",
};
```

**Step 2: Commit**

```bash
git add apps/desktop/src/ui/App.tsx
git commit -m "refactor(desktop): App.tsx uses Zustand stores"
```

---

## Task 16: shared/api/client.ts + delete old code

**Files:**
- Create: `apps/desktop/src/shared/api/client.ts`
- Delete: `apps/desktop/src/application/`
- Delete: `apps/desktop/src/infrastructure/`
- Delete: `apps/desktop/src/domain/`

**Step 1: Create API client**

```typescript
import { useAuthStore } from "../../features/auth/store";
import { useConnectionStore } from "../../features/connection/store";

export async function apiFetch(
  path: string,
  init?: RequestInit
): Promise<Response> {
  const { accessToken } = useAuthStore.getState();
  const { activeBaseUrl } = useConnectionStore.getState();

  return fetch(`${activeBaseUrl}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(accessToken ? { Authorization: `Bearer ${accessToken}` } : {}),
      ...(init?.headers ?? {}),
    },
  });
}
```

**Step 2: Delete old TS infrastructure**

```bash
rm -rf apps/desktop/src/application \
        apps/desktop/src/infrastructure \
        apps/desktop/src/domain
```

**Step 3: Full verification**

```bash
# Rust
cd apps/desktop/src-tauri && cargo check

# TypeScript
cd apps/desktop && npx tsc --noEmit

# Full build
cd apps/desktop && npm run build
```

All three must pass with zero errors.

**Step 4: Commit**

```bash
git add -A apps/desktop/src/
git commit -m "feat(desktop): shared API client, delete old Manager infrastructure"
```
