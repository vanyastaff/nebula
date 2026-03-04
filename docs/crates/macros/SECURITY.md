# Security

## Threat Model

- **Assets:** Macro crate is trusted by all authors; no runtime secrets. Generated code runs in author's crate and in engine/runtime; macro does not execute user code at compile time beyond expansion.
- **Trust boundaries:** We trust syn/quote and proc_macro; authors trust macro crate. Malicious macro could generate malicious code — we maintain macro crate and do not allow arbitrary code execution at compile time beyond intended expansion.
- **Attacker capabilities:** Supply-chain: malicious version of macro crate could emit bad code. Mitigation: crate is maintained in repo; authors pin version. No network or file I/O in macro (expansion only).

## Security Controls

- **No unsafe:** forbid(unsafe_code) prevents unsafe in macro crate; no FFI or memory manipulation.
- **No I/O at compile time:** Expansion does not read files or network; only TokenStream in/out. (Build scripts could; macro crate does not use build script for expansion.)
- **Input validation:** Attribute parsing validates required fields and types; invalid input yields compile error, not arbitrary expansion.

## Abuse Cases

- **Malicious expansion:** If macro were to emit code that exfiltrates data or runs unexpected code — prevention: review macro code; no unsafe; expansion is deterministic and documented. Detection: code review, audit.
- **Attribute injection:** Author passes malicious attribute value — macro treats as data (e.g. string for key/name); no eval or execution. Generated code uses it as identifier or literal; no injection vector in current design.

## Security Requirements

- **Must-have:** No unsafe; no compile-time I/O in expansion; expansion deterministic and reviewable.
- **Should-have:** Document that authors should pin macro crate version in production.

## Security Test Plan

- Audit: no unsafe in crate (cargo build, grep unsafe).
- No dynamic load or eval in macro code.
