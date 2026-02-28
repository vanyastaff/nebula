# Phase 0 Research: Config Contract Hardening

## Decision 1: Keep deterministic layered precedence as a hard compatibility contract

- Decision: Preserve and formalize explicit precedence behavior (defaults < file < env < high-priority inline overrides).
- Rationale: Runtime-facing crates require predictable resolution to avoid environment-specific drift.
- Alternatives considered:
  - Unordered merge behavior: rejected due to operational ambiguity.
  - Dynamic precedence policy by default: rejected for compatibility risk.

## Decision 2: Treat validation as activation gate for startup and reload

- Decision: Candidate merged config must pass validation before activation.
- Rationale: Prevents silent bad-config activation and runtime instability.
- Alternatives considered:
  - Warning-only validation: rejected as unsafe for production control paths.

## Decision 3: Preserve last-known-good snapshot on reload failure

- Decision: Reject invalid reload attempts atomically and keep previous active snapshot.
- Rationale: Supports service continuity under invalid change events.
- Alternatives considered:
  - Partial apply of valid sections: rejected due to non-deterministic state risk.

## Decision 4: Keep dynamic core storage with stable typed retrieval contracts

- Decision: Retain JSON-tree storage model and codify typed `get<T>` behavior contract.
- Rationale: Balances flexibility with consumer-level type guarantees.
- Alternatives considered:
  - Compile-time-only typed global config model: rejected as too rigid for diverse consumers.

## Decision 5: Document and test path-based access contract as versioned surface

- Decision: Path traversal behavior and error categories are compatibility fixtures.
- Rationale: Consumer crates depend on stable retrieval semantics.
- Alternatives considered:
  - Undocumented path behavior: rejected due to regression detection gaps.

## Decision 6: Defer remote source GA until trust and reliability model is hardened

- Decision: Keep remote/database/kv source support as deferred expansion with explicit security requirements.
- Rationale: Reduces rollout risk while core contracts are stabilized first.
- Alternatives considered:
  - Immediate remote source production rollout: rejected due to incomplete trust policy.
