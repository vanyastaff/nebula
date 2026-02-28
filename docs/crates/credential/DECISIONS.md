# Decisions

## D-001: Provider Abstraction for Storage

Status: accepted

Decision:
- all persistence goes through `StorageProvider` trait.

Reason:
- backend portability and testability without changing manager API.

## D-002: Context + Scope Isolation

Status: accepted

Decision:
- credential access is context-driven (tenant/user/scope aware).

Reason:
- strict multi-tenant security boundary.

## D-003: Protocol-agnostic Core

Status: accepted

Decision:
- protocol-specific logic is isolated in `protocols`, with shared contracts in `traits`.

Reason:
- extensibility for new auth mechanisms with bounded integration surface.

## D-004: Rotation as First-class Subsystem

Status: accepted

Decision:
- keep rotation policy/transaction/failure handling explicit, not ad-hoc manager logic.

Reason:
- operational safety and auditability in production environments.

## D-005: Security-first Utility Layer

Status: accepted

Decision:
- centralize crypto/secret/time/retry utilities under controlled APIs.

Reason:
- avoid duplicated, inconsistent secret handling across modules.
