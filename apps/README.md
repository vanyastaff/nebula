# Apps

Application layer for Nebula clients.

## Structure

- `apps/web` - browser UI (SaaS/self-hosted frontend)
- `apps/desktop` - Tauri desktop shell + same web UI runtime

## Notes

This repository could not use `npm create tauri-app` in the current environment
because npm registry access is offline-only (`only-if-cached`).
The scaffold below follows Tauri 2.x structure and can be completed by running
`npm install` when network access is available.
