# Archived From "docs/archive/nebula-all-docs.md"

## FILE: docs/guides/contributing.md
---

# Contributing to Nebula

Thank you for your interest in contributing to Nebula! This guide will help you get started.

## Getting Started

1. Fork the repository
2. Clone your fork
3. Create a feature branch
4. Make your changes
5. Submit a pull request

## Development Setup

```bash
# Clone the repo
git clone https://github.com/yourusername/nebula.git
cd nebula

# Install dependencies
cargo build

# Run tests
cargo test

# Run with all features
cargo test --all-features
```

## Code Style

We use standard Rust formatting:
```bash
cargo fmt -- --check
cargo clippy -- -D warnings
```

## Testing

All new features must include tests:
- Unit tests for individual components
- Integration tests for cross-crate functionality
- Documentation tests for examples

## Documentation

- All public APIs must be documented
- Include examples in doc comments
- Update relevant guides

## Pull Request Process

1. Update the CHANGELOG.md
2. Update documentation
3. Ensure all tests pass
4. Request review from maintainers

## Code of Conduct

Please note we have a code of conduct, please follow it in all your interactions with the project.

## Questions?

Feel free to open an issue or join our Discord community!

