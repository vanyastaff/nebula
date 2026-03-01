# Nebula Desktop (Tauri)

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
- `local`: `http://127.0.0.1:5678`
- `selfhosted`: `http://127.0.0.1:5678` (override in real deployment)
- `saas`: `https://api.nebula.example.com` (placeholder)
