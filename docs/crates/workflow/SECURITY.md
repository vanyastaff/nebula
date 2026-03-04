# Security

## Threat Model

- **Assets:** Workflow definitions may reference credential IDs, resource IDs, or sensitive parameter placeholders; definition itself is not secret but may be sensitive in multi-tenant contexts.
- **Trust boundaries:** Workflow crate is data-only; no execution. Trust boundary is at engine/API: who can load or submit workflows.
- **Attacker capabilities:** Malformed or maliciously large definitions (DoS); path traversal or injection if node/connection data is echoed without sanitization (API/UI responsibility).

## Security Controls

- **Input validation:** `validate_workflow` rejects invalid structure (cycles, unknown refs, malformed refs). No execution; no credential access in crate.
- **Secret handling:** None; workflow holds IDs/refs only, not secrets.
- **Size/DoS:** No built-in size limit in crate; API or storage should enforce max definition size and node/edge count.

## Abuse Cases

- **Oversized workflow (DoS):** Prevention: API or engine enforces max nodes/edges/size. Detection: metrics on definition size. Response: reject or rate-limit.
- **Invalid refs (confused deputy):** Prevention: validate_workflow rejects UnknownNode, InvalidParameterReference. Detection: validation errors. Response: 400 to caller.

## Security Requirements

- **Must-have:** Validation rejects cycles and invalid refs; no execution or credential access in workflow crate.
- **Should-have:** Document that API/engine must enforce size limits and tenant isolation for workflow load/save.

## Security Test Plan

- Unit tests: validation rejects cycles, duplicate nodes, unknown refs, empty name, no nodes, no entry nodes.
- No fuzz requirement for this crate (data shape only); optional fuzz on parser if added later.
