# Nebula Desktop (Tauri)

> **Status:** reference shell, not a release artefact.
>
> This crate is kept as a working example of how to embed Nebula behind a
> Tauri 2.x frontend; it is not part of any release pipeline and is not
> covered by the canonical knife scenario. When a production desktop
> client lands, it will be its own composition root with its own ADR.

Desktop shell for Nebula using Tauri 2.x.

## Structure

- `src/` - frontend (Vite + React)
- `src-tauri/` - Rust Tauri host

## Commands

```bash
npm install
npm run tauri:dev
npm run tauri:build
```

## Runtime profiles

By default profile is `local`.

Set profile via env:

PowerShell:

```powershell
$env:VITE_NEBULA_PROFILE="selfhosted"
npm run tauri:dev
```

Or set explicit API URL:

```powershell
$env:VITE_NEBULA_API_URL="https://api.example.com"
npm run tauri:dev
```

Profiles:
- `local`: `http://localhost:5678`
- `selfhosted`: `http://localhost:5678` (override in real deployment)
- `saas`: `https://api.nebula.example.com` (placeholder)

Rust-side profile command:
- `NEBULA_API_PROFILE` env var is exposed via Tauri invoke command `get_api_profile`.

Backend CORS override:
- `NEBULA_CORS_ALLOW_ORIGINS` (comma-separated), for example:
  - `NEBULA_CORS_ALLOW_ORIGINS=http://localhost:5173,tauri://localhost`
