# Security

## Threat Model

- **Assets:** Sensitive parameter values (secrets, credentials); schema definitions that may expose structure
- **Trust boundaries:** Schema is trusted (defined by action/credential authors); values may come from untrusted input (user, API)
- **Attacker capabilities:** Inject malformed values; attempt to bypass validation; exfiltrate secrets via error messages or logs

## Security Controls

- **Authn/authz:** None in this crate; caller (engine/API) enforces access control
- **Isolation/sandboxing:** N/A; pure schema and validation logic
- **Secret handling:** `ParameterMetadata::sensitive` and `SecretParameter` flag values for masking; caller must avoid logging/displaying sensitive values
- **Input validation:** `ParameterCollection::validate` enforces type and rules; Custom rules require expression engine (caller's responsibility)

## Abuse Cases

- **Malformed JSON causing panic:** Validation uses safe extraction; no unwrap on untrusted input in hot path
  - Prevention: Defensive checks; `ParameterError` for invalid types
  - Detection: Fuzz tests
  - Response: Return `ParameterError`; never panic on input
- **Secret leakage in error messages:** `ParameterError` variants do not include value content; `InvalidValue`/`ValidationError` use `reason` string
  - Prevention: Never include raw values in error strings; sensitive params use generic messages
  - Detection: Code review; audit error construction
  - Response: Sanitize before logging if needed
- **Regex DoS (Pattern rule):** `ValidationRule::Pattern` passes to validator; validator should use bounded backtracking
  - Prevention: Document regex constraints; consider timeout for complex patterns (validator responsibility)
  - Detection: Fuzz with pathological patterns
  - Response: Return `ValidationError` with safe message

## Security Requirements

- **Must-have:** No panic on malformed input; no secret value in error messages; `sensitive` respected by consumers
- **Should-have:** Document regex safety for Pattern rule; schema lint to catch obviously dangerous patterns

## Security Test Plan

- **Static analysis:** `cargo audit`; clippy for unwrap/expect on user input
- **Dynamic tests:** Validation with malformed types; oversized strings; deeply nested structures
- **Fuzz/property tests:** `serde_json::from_str` on random input; validate with random `ParameterValues`
