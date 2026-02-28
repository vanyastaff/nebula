# Security

## Threat Model

- **Assets:** System information (CPU, memory, process list); potential exposure of sensitive data (process names, paths)
- **Trust boundaries:** Crate runs in process context; no network exposure by default
- **Attacker capabilities:** Local process; may read `/proc`, sysctl, WinAPI; no privilege escalation

## Security Controls

- **Authn/authz:** None; relies on process privileges
- **Isolation/sandboxing:** None; caller responsibility
- **Secret handling:** Process list may expose env vars; do not log full `ProcessInfo`
- **Input validation:** PID, mount points validated; platform-specific paths sanitized

## Abuse Cases

- **Process list enumeration:** Attacker could infer running services
  - **Prevention:** Document that process list is sensitive; avoid logging
  - **Detection:** N/A
  - **Response:** N/A

- **Memory lock (mlock) abuse:** `memory::management::lock()` can exhaust `RLIMIT_MEMLOCK`
  - **Prevention:** Document limits; require explicit opt-in
  - **Detection:** N/A
  - **Response:** Caller handles `SystemError`

- **Path traversal in disk/network:** Paths from sysinfo are trusted; `filesystem_info(path)` takes user input
  - **Prevention:** Validate path format; avoid symlink following
  - **Detection:** N/A
  - **Response:** Return `None` on invalid path

## Security Requirements

- **Must-have:** No unsafe exposure of uninitialized memory; documented unsafe preconditions
- **Should-have:** Avoid logging sensitive process/network data

## Security Test Plan

- **Static analysis:** `cargo audit`; `cargo clippy` with safety lints
- **Dynamic tests:** Permission-denied scenarios; invalid PID/path
- **Fuzz/property tests:** Format functions (e.g., `format_bytes`) with proptest
