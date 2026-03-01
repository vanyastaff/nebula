# Security

## Threat Model

- **Assets:** Execution state and journal may contain references to workflow structure and node output metadata (e.g. blob keys). No raw secrets in execution crate types; credential and secret handling is in nebula-credential and runtime.
- **Trust boundaries:** Execution crate is a library used by engine and API. Trust boundary is at process boundary and at API/auth layer; execution crate does not enforce auth.
- **Attacker capabilities:** Assume attackers can submit or modify workflow definitions and trigger executions (mitigated by API auth and engine validation). Execution crate does not parse untrusted input directly; workflow and plan validation use nebula-workflow and return PlanValidation on invalid input.

## Security Controls

- **Authn/authz:** None in execution crate. Engine and API are responsible for ensuring only authorized callers create or read execution state.
- **Isolation/sandboxing:** Execution crate does not run actions; runtime/sandbox isolate action execution. Execution state is data only.
- **Secret handling:** No secrets in ExecutionState, NodeOutput, or JournalEntry. Blob refs are storage keys, not secret material. Error messages (e.g. NodeFailed error string) should not log credentials — responsibility of engine/runtime when constructing journal or state.
- **Input validation:** Plan build validates workflow (non-empty, valid graph); transition validators reject invalid state transitions. Engine must not bypass validators.

## Abuse Cases

- **Invalid transition applied:** Attacker or bug causes engine to set state to an invalid combination. Prevention: engine must call `validate_execution_transition` and `validate_node_transition` before mutating; no direct field set that bypasses validation. Detection: tests and invariants (e.g. terminal state never transitions). Response: fail the request and log.
- **Idempotency key collision:** Malicious or buggy key generation could cause wrong duplicate detection. Prevention: key format is deterministic from (execution_id, node_id, attempt); engine must use correct attempt number. Detection: monitor duplicate rate and key distribution. Response: audit and fix key generation.
- **Journal/state injection:** If engine or storage were compromised, fake journal entries or state could be inserted. Prevention: execution crate does not authenticate journal entries; persistence layer and engine must enforce integrity. Detection: out-of-band audit. Response: incident response.

## Security Requirements

- **Must-have:** No credential or secret in execution types; transition validation is mandatory for state updates; idempotency key format is deterministic and documented.
- **Should-have:** Journal and state serialization do not expose internal implementation details that could aid enumeration or injection.

## Security Test Plan

- **Static analysis:** `cargo audit`; `#![forbid(unsafe_code)]` in crate (already present).
- **Dynamic tests:** Transition tests ensure no invalid transition is accepted; idempotency tests ensure key uniqueness and check_and_mark semantics.
- **Fuzz/property tests:** Optional: property test that any sequence of valid transitions never produces an invalid state; serde roundtrip for all public types.
