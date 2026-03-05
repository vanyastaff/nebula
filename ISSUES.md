# Issues: Guidelines & Templates

This document outlines how to report bugs, request features, and contribute documentation to Nebula.

---

## Table of Contents

- [Before You Report](#before-you-report)
- [Bug Reports](#bug-reports)
- [Feature Requests](#feature-requests)
- [Documentation Issues](#documentation-issues)
- [Issue Triage & Labels](#issue-triage--labels)
- [Getting Help](#getting-help)

---

## Before You Report

### ✅ Do's

- **Search first** — Check [existing issues](https://github.com/vanyastaff/nebula/issues) to avoid duplicates
- **Be specific** — Provide clear, reproducible examples
- **Include context** — Rust version, OS, relevant crate versions
- **One issue per report** — Don't mix multiple problems
- **Follow the template** — Use the provided structure

### ❌ Don'ts

- **Don't report security vulnerabilities publicly** — Email [vanya.john.stafford@gmail.com](mailto:vanya.john.stafford@gmail.com)
- **Don't ask for ETA** — Maintainers are volunteers; work happens when it happens
- **Don't spam** — No bumping issues without new information
- **Don't use issues for support** — Use [Discussions](https://github.com/vanyastaff/nebula/discussions) instead

---

## Bug Reports

### Template

```markdown
## Description
A clear, concise description of the bug.

## Steps to Reproduce
1. Step one
2. Step two
3. Step three

## Expected Behavior
What should happen?

## Actual Behavior
What actually happens?

## Environment
- Rust version: `rustc --version`
- OS: (Linux / macOS / Windows)
- Crate(s) affected: (e.g., `nebula-engine`, `nebula-runtime`)

## Minimal Example
```rust
// Code that triggers the bug
```

## Logs or Error Messages
```
Include stack traces, error output, etc.
```

## Additional Context
Screenshots, related issues, or any other info?
```

### What Makes a Good Bug Report?

✅ **Good:**
- "Streaming response handling panics with large payloads > 1GB on macOS. Here's the stack trace..."
- "When executing workflows with circular dependency detection, the engine freezes for 30s on a 1000-node DAG"

❌ **Vague:**
- "It doesn't work"
- "There's a problem with the action execution"

---

## Feature Requests

### Template

```markdown
## Problem Statement
Describe the problem or limitation you're facing.

## Proposed Solution
Your idea for solving this problem.

## Alternative Solutions
Other approaches you've considered.

## Use Case
Why is this important? Who benefits?

## Example
```rust
// Sketch of how this would work
```

## Related Issues
Link to any existing issues or discussions.
```

### What Makes a Good Feature Request?

✅ **Good:**
- "Add support for conditional branching in workflows based on previous node output"
- "Implement action retry with exponential backoff (currently only fixed backoff)"

❌ **Vague:**
- "Make it faster"
- "Add more features"

---

## Documentation Issues

### Template

```markdown
## What's Missing or Unclear?
Describe the documentation gap.

## Where?
- File/section: (e.g., `vision/ARCHITECTURE.md#async-patterns`)
- Or: "No docs exist for X"

## Suggested Improvement
What should the documentation say?

## Audience
Who needs this info? (e.g., new contributors, users of `nebula-runtime`)
```

### Examples of Good Docs Issues

✅ **Good:**
- "ARCHITECTURE.md doesn't explain the credential injection pattern; new contributors are confused"
- "Add a 'Testing Actions' guide for the `nebula-action` crate"

---

## Issue Triage & Labels

### Label Categories

See [LABELS.md](LABELS.md) for the complete label system.

#### Severity (Pick One)
- `severity:critical` — System broken, data loss risk
- `severity:high` — Major feature broken
- `severity:medium` — Workaround exists
- `severity:low` — Polish, nice-to-have

#### Type (Pick One)
- `type:bug` — Something is broken
- `type:feature` — New capability
- `type:enhancement` — Improve existing feature
- `type:docs` — Documentation only
- `type:chore` — Maintenance, no user impact

#### Area (Pick Multiple as Needed)
- `area:action` — Action trait, execution
- `area:engine` — DAG scheduler, orchestration
- `area:runtime` — Task execution, isolation
- `area:storage` — KV storage, persistence
- `area:api` — REST/WebSocket server
- `area:credential` — Secrets, encryption
- `area:resource` — Pooling, lifecycle
- `area:plugin` — Plugin system
- `area:testing` — Test infrastructure
- `area:docs` — Documentation

#### Priority (Pick One)
- `priority:p0` — Critical, fix immediately
- `priority:p1` — Important, schedule next
- `priority:p2` — Nice-to-have, add to backlog
- `priority:p3` — Future consideration

#### Status (Applied by Maintainers)
- `status:blocked` — Waiting on something
- `status:needs-discussion` — Needs design consensus
- `status:in-progress` — Someone is working on it
- `status:ready` — Approved and ready to work on

#### Difficulty (Pick One)
- `difficulty:good-first-issue` — Perfect for newcomers
- `difficulty:medium` — Takes 2–5 hours
- `difficulty:hard` — Complex, 1+ weeks

---

## Getting Help

**Unsure where to start?**

1. **Read** [CONTRIBUTING.md](CONTRIBUTING.md) and [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md)
2. **Filter issues** by `good-first-issue` or `help-wanted`
3. **Comment** on an issue to express interest; maintainers will help

**Have a question?**

→ Use [GitHub Discussions](https://github.com/vanyastaff/nebula/discussions) instead of issues

**Found a security vulnerability?**

→ Email [vanya.john.stafford@gmail.com](mailto:vanya.john.stafford@gmail.com) (do NOT open a public issue)

---

## Examples

### 📝 A Great Bug Report

```markdown
## Description
Workflow execution panics when a credential is deleted mid-execution.

## Steps to Reproduce
1. Create workflow with action that uses credential X
2. Start execution
3. While execution is running, delete credential X from UI
4. Observe panic in engine logs

## Expected Behavior
Execution should fail gracefully with "credential not found" error.

## Actual Behavior
```
thread 'tokio-runtime-worker' panicked at 'called `Option::unwrap()` on a `None` value'
at nebula-runtime/src/executor.rs:142
```

## Environment
- Rust 1.93 (via rustup)
- macOS 14.2
- Affected crate: `nebula-runtime`, `nebula-credential`

## Minimal Example
See workflow attachment: `workflow-with-deleted-cred.json`

## Additional Context
This likely affects all credential types. Possibly related to #456.
```

### 💡 A Great Feature Request

```markdown
## Problem Statement
Users can't throttle concurrent workflow executions. If 100 workflows trigger simultaneously, we overwhelm downstream APIs and databases.

## Proposed Solution
Add a configurable `max_concurrent_executions` setting per tenant with queuing.

## Use Case
E-commerce platform running thousands of order workflows; backend APIs can't handle 100 concurrent requests.

## Example
```rust
let config = TenantConfig {
    max_concurrent_executions: 10,
    // ...
};
```

## Related Issues
Discussed in #789 (performance issues under load)
```

---

**Have questions about this process?** Open a [discussion](https://github.com/vanyastaff/nebula/discussions) or comment on a related issue!

