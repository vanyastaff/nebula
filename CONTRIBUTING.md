# Contributing to Nebula

Thank you for your interest in contributing to Nebula! This document provides guidelines and instructions for contributing.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Style Guidelines](#style-guidelines)
- [Project Structure](#project-structure)

## Code of Conduct

This project adheres to a code of conduct. By participating, you are expected to:

- Be respectful and inclusive
- Welcome newcomers and help them get started
- Focus on constructive feedback
- Accept responsibility for your actions

## Getting Started

### Prerequisites

- Rust 1.70 or later
- Cargo (comes with Rust)
- Git

### Fork and Clone

1. Fork the repository on GitHub
2. Clone your fork locally:

```bash
git clone https://github.com/YOUR_USERNAME/nebula.git
cd nebula
```

3. Add the upstream remote:

```bash
git remote add upstream https://github.com/vanyastaff/nebula.git
```

## Development Setup

### Build the Project

```bash
cargo build
```

### Run Tests

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p nebula-validator

# Run tests with output
cargo test -- --nocapture
```

### Run Benchmarks

```bash
cargo bench
```

### Check Code Quality

```bash
# Check compilation
cargo check

# Run Clippy lints
cargo clippy -- -D warnings

# Format code
cargo fmt
```

## Making Changes

### Branch Naming

Use descriptive branch names:

- `feat/your-feature-name` - For new features
- `fix/bug-description` - For bug fixes
- `docs/update-readme` - For documentation
- `refactor/improve-xyz` - For refactoring
- `test/add-tests-for-xyz` - For adding tests

### Commit Messages

Follow conventional commits format:

```
type(scope): subject

body (optional)

footer (optional)
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks
- `perf`: Performance improvements

**Examples:**

```
feat(validator): Add email validation support

Implements a new EmailValidator that supports RFC 5322 compliant
email validation with customizable options.

Closes #123
```

```
fix(combinator): Fix memory leak in cached combinator

The LRU cache was not properly evicting old entries, causing
unbounded memory growth.
```

### Writing Tests

- Write tests for all new features
- Ensure existing tests pass
- Aim for high code coverage
- Include both unit and integration tests

Example test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_accepts_valid_input() {
        let validator = MyValidator::new();
        assert!(validator.validate(&"valid").is_ok());
    }

    #[test]
    fn test_validator_rejects_invalid_input() {
        let validator = MyValidator::new();
        let result = validator.validate(&"invalid");
        assert!(result.is_err());
    }
}
```

## Testing

### Test Organization

- **Unit tests**: In the same file as the code, in a `tests` module
- **Integration tests**: In `tests/` directory at crate root
- **Documentation tests**: In doc comments with `/// # Examples`

### Running Specific Tests

```bash
# Run a specific test
cargo test test_name

# Run tests for a specific module
cargo test module_name

# Run tests matching a pattern
cargo test email
```

### Test Coverage

We use `tarpaulin` for coverage:

```bash
cargo install cargo-tarpaulin
cargo tarpaulin --out Html
```

## Submitting Changes

### Before Submitting

1. **Run all tests**: `cargo test`
2. **Check formatting**: `cargo fmt --check`
3. **Run Clippy**: `cargo clippy -- -D warnings`
4. **Update documentation**: If you changed APIs
5. **Add to CHANGELOG**: Describe your changes

### Pull Request Process

1. **Update your branch** with latest upstream:

```bash
git fetch upstream
git rebase upstream/main
```

2. **Push to your fork**:

```bash
git push origin your-branch-name
```

3. **Create Pull Request** on GitHub:
   - Provide a clear title and description
   - Reference any related issues
   - Explain the changes and their impact
   - Add screenshots if UI changes

4. **Address review comments**:
   - Make requested changes
   - Push additional commits
   - Respond to reviewer feedback

5. **Wait for approval** and merge

### Pull Request Template

```markdown
## Description

Brief description of changes

## Type of Change

- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing

- [ ] All tests pass
- [ ] New tests added
- [ ] Manual testing performed

## Checklist

- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Comments added for complex code
- [ ] Documentation updated
- [ ] No new warnings introduced
```

## Style Guidelines

### Rust Code Style

Follow the official [Rust Style Guide](https://doc.rust-lang.org/stable/style-guide/):

- Use `rustfmt` for formatting (runs automatically with `cargo fmt`)
- Follow naming conventions:
  - `snake_case` for functions and variables
  - `PascalCase` for types and traits
  - `SCREAMING_SNAKE_CASE` for constants
- Keep functions focused and small
- Use meaningful variable names
- Add doc comments for public APIs

### Documentation

- Use `///` for doc comments on public items
- Include examples in doc comments
- Explain **why**, not just **what**
- Keep docs up-to-date with code changes

Example:

```rust
/// Validates that a string is a valid email address.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::EmailValidator;
///
/// let validator = EmailValidator::new();
/// assert!(validator.validate("user@example.com").is_ok());
/// ```
///
/// # Errors
///
/// Returns an error if the email format is invalid.
pub fn validate_email(email: &str) -> Result<(), ValidationError> {
    // Implementation
}
```

### Error Handling

- Use `Result` for operations that can fail
- Provide meaningful error messages
- Use custom error types when appropriate
- Document error conditions

### Performance

- Avoid unnecessary allocations
- Use iterators instead of loops where possible
- Benchmark performance-critical code
- Consider `#[inline]` for small, hot functions

## Project Structure

```
nebula/
├── crates/
│   ├── nebula-validator/     # Core validation library
│   ├── nebula-error/          # Error handling
│   ├── nebula-resource/       # Resource management
│   └── ...                    # Other crates
├── examples/                  # Example code
├── docs/                      # Documentation
├── tests/                     # Integration tests
└── benches/                   # Benchmarks
```

### Adding a New Crate

1. Create crate: `cargo new --lib crates/nebula-newcrate`
2. Add to workspace in root `Cargo.toml`
3. Document the crate purpose in its README
4. Add appropriate dependencies
5. Write tests

### Module Organization

- Keep modules focused and cohesive
- Use `mod.rs` for module roots
- Re-export public APIs in parent modules
- Use `pub(crate)` for internal APIs

## Need Help?

- Open an issue for questions
- Check existing issues and PRs
- Read the documentation
- Ask in discussions

## Recognition

Contributors will be recognized in:
- CHANGELOG.md
- Project README
- Release notes

Thank you for contributing to Nebula! 🚀
