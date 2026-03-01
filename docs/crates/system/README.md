# nebula-system

Cross-platform system information and utilities for the Nebula workflow ecosystem.

## Scope

- **In scope:**
  - CPU, memory, disk, network, process information
  - Memory pressure detection (Low/Medium/High/Critical)
  - Hardware detection (cores, cache, NUMA)
  - Cross-platform abstraction (Linux, macOS, Windows)
  - Feature-gated modules for minimal footprint

- **Out of scope:**
  - Low-level hardware control
  - Real-time OS operations
  - Kernel-level operations
  - Platform-specific optimizations (use platform APIs directly)

## Current State

- **Maturity:** Pre-release; core APIs stable, extended features evolving
- **Key strengths:** Unified API across platforms, minimal dependencies, pressure detection
- **Key risks:** sysinfo API changes, platform-specific edge cases

## Target State

- **Production criteria:** Stable API surface, comprehensive platform coverage, documented failure modes
- **Compatibility guarantees:** Semver; feature flags additive; deprecation window 2 minor versions

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
