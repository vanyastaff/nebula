# Decisions

## D-001: Schema-first, Value-later Model

Status: accepted

Decision:
- define parameter structure as typed Rust schema, while runtime values remain JSON.

Reason:
- flexible transport/storage compatibility and dynamic workflow execution.

## D-002: Declarative Validation Rules

Status: accepted

Decision:
- represent validation constraints as serializable rule data, then evaluate in validation layer.

Reason:
- enables persistence, tooling, and cross-layer portability.

## D-003: Capability-based Kind Semantics

Status: accepted

Decision:
- model behavior flags via `ParameterCapability` instead of hardcoded ad-hoc checks.

Reason:
- clearer downstream logic for UI/render/engine constraints.

## D-004: Recursive Containers

Status: accepted

Decision:
- support nested schema structures through object/list/mode/group/expirable.

Reason:
- real-world node configs require hierarchical inputs.

## D-005: Error Aggregation over Fail-fast

Status: accepted

Decision:
- collection validation accumulates errors instead of returning first failure.

Reason:
- better UX and faster debugging for complex parameter forms.
