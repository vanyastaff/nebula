# Project: Nebula

## Overview
Nebula is a modular, type-safe workflow automation engine written in Rust. It models automations as DAG workflows, executes them reliably via an async runtime, and exposes integration points through plugins, REST/WebSocket APIs, and a Tauri desktop client.

## Core Features
- DAG-based workflow definitions and orchestration
- Action/plugin runtime with credential and resource injection
- Reliable execution with retries, timeouts, and observability
- Storage abstraction with in-memory and PostgreSQL-backed implementations
- Multi-surface interfaces: REST API, WebSocket events, and desktop UI

## Tech Stack
- **Language:** Rust (workspace, edition 2024, MSRV 1.93)
- **Framework:** Axum (API), Tokio (async runtime)
- **Database:** PostgreSQL (SQL migrations in `migrations/`)
- **ORM/Query:** SQLx-style migration-driven approach (no monolithic ORM detected)
- **Frontend/Desktop:** Tauri v2 + React + TypeScript (Vite)
- **Integrations:** MCP servers, plugin-based extension system

## Architecture Notes
- Cargo workspace with layered crate architecture (core, cross-cutting, business, execution, infrastructure, API)
- One-way dependency direction is enforced and documented
- Shared cross-cutting concerns (eventbus, config, telemetry, metrics) are reused across layers
- Desktop app lives outside the workspace root members (`apps/desktop/src-tauri`)

## Non-Functional Requirements
- Logging: structured tracing-based logs with configurable level
- Error handling: typed errors (`thiserror`) in library crates
- Security: encrypted credentials at rest (AES-256-GCM), capability-based desktop permissions
- Reliability: retry/circuit-breaker patterns and explicit lifecycle state modeling
- Quality gates: fmt, clippy `-D warnings`, workspace tests, docs build
