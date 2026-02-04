<!--
Sync Impact Report - Constitution v1.1.0

VERSION CHANGE: Minor update (v1.0.0 → v1.1.0)
RATIONALE: Add Rust API Guidelines compliance and per-phase quality gates to ensure idiomatic, well-documented code following Rust ecosystem conventions.

MODIFIED PRINCIPLES: N/A
ADDED SECTIONS:
  - Principle VIII: Rust API Guidelines and Documentation (MANDATORY)
    - Documentation standards (rustdoc, examples, errors, panics)
    - Naming conventions (snake_case, PascalCase, SCREAMING_SNAKE_CASE)
    - Method organization and builder patterns
    - Quality gates per phase (fmt, clippy, check, doc)

REMOVED SECTIONS: N/A

TEMPLATES STATUS:
  ✅ plan-template.md - Constitution Check section aligns with principles (includes new Principle VIII)
  ✅ spec-template.md - Requirements structure aligns with quality standards
  ✅ tasks-template.md - Task organization aligns with TDD and parallel execution principles
  ✅ All command files - Generic guidance verified (no agent-specific references)

FOLLOW-UP TODOs:
  - Update existing Phase 2 code to remove phase markers from public documentation
  - Run quality gates (fmt, clippy, check, doc) after Phase 2 completion
  - Apply naming convention fixes (ctx → context, gen → generator)
-->

# Nebula Constitution

## Core Principles

### I. Type Safety First

**The Rust type system is our primary correctness tool.** All features MUST leverage compile-time guarantees over runtime checks wherever feasible. Type errors caught at compile time prevent entire classes of runtime failures.

**Rationale**: In a workflow automation system handling user data and executing arbitrary actions, type safety prevents data corruption, invalid state transitions, and security vulnerabilities. The investment in precise types pays dividends in reliability and maintainability.

**Rules**:
- MUST use newtype patterns for domain identifiers (ExecutionId, WorkflowId, NodeId)
- MUST use enums for exhaustive state representation
- MUST avoid `String` for typed data; use custom types
- MUST use sized types in type aliases (not `str`, use `String`)
- MUST provide explicit type annotations for complex generics in Rust 2024

### II. Isolated Error Handling

**Each crate defines its own error type.** Cross-crate error dependencies create coupling and circular dependency risks. Errors are converted at boundaries using `From`/`Into` traits.

**Rationale**: In a 16-crate workspace, shared error types become a central point of coupling. Isolated errors enable independent crate evolution and prevent build cascades from error type changes.

**Rules**:
- MUST NOT depend on `nebula-error` or any shared error crate
- MUST use `thiserror::Error` for error definitions
- MUST convert errors at crate boundaries with context
- MUST include actionable error messages with field details

### III. Test-Driven Development (NON-NEGOTIABLE)

**Tests written → Tests fail → Then implement.** The Red-Green-Refactor cycle is strictly enforced for all non-trivial functionality. Tests define expected behavior before implementation begins.

**Rationale**: Workflow automation systems have complex state transitions, async interactions, and edge cases that are impossible to verify manually. TDD ensures every code path has a specified behavior and regression protection.

**Rules**:
- MUST write tests before implementation for all new features
- MUST verify tests fail before writing implementation code
- MUST use `#[tokio::test(flavor = "multi_thread")]` for async tests
- MUST use `tokio::time::pause()` and `tokio::time::advance()` for time-based tests
- MAY skip TDD only for trivial refactorings (renaming, formatting)

### IV. Async Discipline

**Async code follows strict patterns to prevent deadlocks, resource leaks, and cancellation bugs.** All async operations MUST support cancellation, include timeouts, and use appropriate concurrency primitives.

**Rationale**: Workflow execution is inherently concurrent and long-running. Poor async patterns lead to resource exhaustion, stuck workflows, and silent failures that are difficult to debug.

**Rules**:
- MUST use `JoinSet` for scoped parallel tasks
- MUST include cancellation via `tokio::select!` for all long-running operations
- MUST apply timeouts: default 30s, database 5s, HTTP 10s
- MUST use bounded channels for work queues to prevent memory exhaustion
- MUST use `broadcast` only for stateless events
- MUST prefer `RwLock` over `Mutex` for shared state with read-heavy patterns

### V. Modular Workspace Architecture

**The 16-crate workspace enforces clear separation of concerns.** Dependencies flow in one direction through architectural layers. No circular dependencies are permitted.

**Rationale**: Clear module boundaries enable parallel development, independent testing, and evolutionary architecture. Violating layer boundaries creates coupling that prevents future refactoring.

**Rules**:
- MUST follow dependency flow: Infrastructure → Cross-Cutting → Core → Node → Execution → Business → Tools → Presentation
- MUST use `nebula-core` for shared types to prevent cycles
- MUST NOT add dependencies that violate layer boundaries
- MUST document architectural decisions in `docs/` when adding cross-crate dependencies

### VI. Observability by Design

**All runtime behavior MUST be observable through logs, metrics, and traces.** Silent failures are prohibited. Every error path, state transition, and resource operation MUST emit structured events.

**Rationale**: Distributed workflow execution involves multiple async tasks, external integrations, and user-defined logic. Without comprehensive observability, debugging production issues is impossible.

**Rules**:
- MUST use `tracing` for all logging (not `println!`, `log`, or `eprintln!`)
- MUST emit events for: workflow lifecycle, execution state changes, node start/completion, errors
- MUST include context fields: execution_id, workflow_id, node_id, tenant_id
- MUST log errors with full context before propagating
- MAY integrate OpenTelemetry for distributed tracing
- MAY expose Prometheus metrics for performance monitoring

### VII. Simplicity and YAGNI

**Start simple. Add complexity only when justified by concrete requirements.** Premature abstractions, excessive configurability, and speculative features increase cognitive load without delivering value.

**Rationale**: Workflow automation is inherently complex. Every line of code is a liability that must be understood, tested, and maintained. The simplest solution that meets requirements wins.

**Rules**:
- MUST justify complexity with concrete use cases (document in plan.md)
- MUST NOT add features "for future extensibility" without current need
- MUST prefer composition over inheritance
- MUST prefer data over code (declarative workflow definitions)
- MUST delete unused code rather than comment it out

### VIII. Rust API Guidelines and Documentation (MANDATORY)

**All code MUST follow Rust API Guidelines and rustdoc conventions.** Documentation, naming, and structure follow idiomatic Rust patterns without implementation phase markers or task references.

**Rationale**: Consistent, idiomatic code is easier to understand, review, and maintain. Following Rust conventions ensures the codebase feels natural to Rust developers and leverages ecosystem tooling effectively.

**Rules - Documentation**:
- MUST write rustdoc comments for all public items (modules, types, functions, traits)
- MUST use `///` for item documentation, `//!` for module/crate documentation
- MUST include `# Examples` section for non-trivial public APIs
- MUST include `# Errors` section documenting error conditions
- MUST include `# Panics` section if code can panic
- MUST use proper markdown formatting (code blocks with language tags)
- MUST NOT include phase markers (Phase 1, Phase 2) in public documentation
- MUST NOT include task references (T001, TODO) in public documentation
- MAY include phase/task markers only in private comments for implementation tracking

**Rules - Naming Conventions**:
- MUST use `snake_case` for functions, methods, variables, modules
- MUST use `PascalCase` for types, traits, enum variants
- MUST use `SCREAMING_SNAKE_CASE` for constants and statics
- MUST use clear, descriptive names (no abbreviations like `ctx`, prefer `context`)
- MUST use standard Rust terminology: `new()` for constructors, `from_*()` for conversions, `as_*()` for cheap references, `to_*()` for expensive conversions, `into_*()` for consuming conversions

**Rules - Method Organization**:
- MUST order methods: constructors (`new`, `with_*`), conversions (`from_*`, `as_*`, `to_*`, `into_*`), getters, setters, operations
- MUST use builder pattern with `with_*` methods for optional configuration
- MUST use `#[must_use]` for functions with important return values
- MUST mark deprecated APIs with `#[deprecated]` and migration guidance

**Rules - Quality Gates Per Phase**:
- MUST run after each phase completion (before marking phase complete):
  ```bash
  cargo fmt --all
  cargo clippy --workspace -- -D warnings  
  cargo check --workspace
  cargo doc --no-deps --workspace
  ```
- MUST fix all warnings before proceeding to next phase
- MUST ensure documentation builds without errors
- MUST verify examples compile in doc comments

## Code Quality Standards

### Formatting and Linting (MANDATORY)

All code MUST pass the following gates before merge:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
cargo audit
```

**Rationale**: Automated quality gates catch common bugs, enforce style consistency, and prevent security vulnerabilities. These checks run in CI and MUST NOT be disabled.

### Rust Edition and MSRV

- **Edition**: Rust 2024
- **MSRV**: 1.92
- MUST update MSRV only with documented justification

### Dependencies

- MUST use workspace dependencies defined in root `Cargo.toml`
- MUST justify new dependencies in PR description
- MUST prefer well-maintained, widely-used crates
- MUST run `cargo audit` to check for security advisories

### Documentation

- MUST document public APIs with doc comments
- MUST include examples in doc comments for non-obvious APIs
- MUST document error conditions and edge cases
- MUST update architecture docs in `docs/` for significant changes

## Development Workflow

### Branching Strategy

- **Prefix**: `feat/`, `fix/`, `docs/`, `refactor/`
- **Protection**: MUST NOT force push to `main`
- **Example**: `feat/expression-pipeline-operators`

### Commit Format

Use conventional commits: `type(scope): subject`

**Examples**:
- `feat(validator): add range validator for numbers`
- `fix(execution): prevent deadlock in node cancellation`
- `docs(architecture): update execution layer diagram`

**MUST NOT include**:
- "Generated with [Claude Code]" footers
- "Co-Authored-By: Claude Sonnet 4.5" footers
- Keep commits clean and professional

### Issue Tracking

- MUST use `bd` (beads) for issue tracking
- MUST create issues for remaining work at end of session
- MUST update issue status when starting/completing work
- MUST close issues only after `git push` succeeds

### Pull Request Requirements

- MUST link to related issues
- MUST include test coverage for new functionality
- MUST pass all CI checks
- MUST include migration plan for breaking changes

## Governance

### Amendment Process

1. Propose changes via PR to `.specify/memory/constitution.md`
2. Update version according to semantic versioning:
   - **MAJOR**: Backward incompatible governance/principle removals or redefinitions
   - **MINOR**: New principle/section added or materially expanded guidance
   - **PATCH**: Clarifications, wording, typo fixes
3. Update dependent templates in `.specify/templates/`
4. Update ratification date and version metadata
5. Require approval from project maintainer
6. Document change rationale in Sync Impact Report

### Versioning Policy

Constitution follows semantic versioning: `MAJOR.MINOR.PATCH`

Current version changes require:
- Code review for PATCH changes
- Architecture review for MINOR changes
- Project-wide consensus for MAJOR changes

### Compliance Review

- All PRs MUST verify compliance with constitution principles
- Code reviews MUST check for violations
- Complexity MUST be justified in plan.md when violating principle VII
- Violations require documented exception rationale

### Runtime Guidance

For detailed development guidance, consult:
- `CLAUDE.md` - AI assistant working instructions
- `AGENTS.md` - Session completion workflow, issue tracking
- Architecture docs in `docs/` - System design and patterns

**Version**: 1.1.0 | **Ratified**: 2026-01-28 | **Last Amended**: 2026-02-03
