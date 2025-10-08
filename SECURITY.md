# Security Policy

## Supported Versions

Currently supported versions of Nebula with security updates:

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please follow these steps:

### 1. **DO NOT** Open a Public Issue

Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.

### 2. Report Privately

Send an email to: **ivan.kondrashkin@gmail.com**

Include in your report:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)
- Your contact information

### 3. Response Timeline

- **Initial Response**: Within 48 hours
- **Status Update**: Within 7 days
- **Fix Timeline**: Depends on severity
  - Critical: 1-7 days
  - High: 7-14 days
  - Medium: 14-30 days
  - Low: 30-90 days

### 4. Disclosure Process

1. We will acknowledge receipt of your report
2. We will investigate and validate the issue
3. We will develop and test a fix
4. We will release a security advisory and patch
5. We will credit you (unless you prefer anonymity)

## Security Best Practices

### For Users

When using Nebula in your projects:

1. **Keep Dependencies Updated**
   ```bash
   cargo update
   ```

2. **Audit Dependencies**
   ```bash
   cargo audit
   ```

3. **Use Latest Stable Version**
   - Always use the latest patch release
   - Subscribe to security advisories

4. **Validate Inputs**
   - Never trust user input
   - Use Nebula validators appropriately
   - Implement additional security layers

5. **Secrets Management**
   - Never commit secrets to git
   - Use environment variables
   - Use secret management tools

### For Contributors

When contributing code:

1. **Never Commit Secrets**
   - No API keys, passwords, or tokens
   - Use `.gitignore` properly
   - Check commits before pushing

2. **Input Validation**
   - Validate all external inputs
   - Use type-safe APIs
   - Avoid unsafe code unless necessary

3. **Error Handling**
   - Don't expose sensitive information in errors
   - Use appropriate error types
   - Log securely

4. **Dependencies**
   - Minimize external dependencies
   - Use well-maintained crates
   - Review dependency code for critical features

5. **Testing**
   - Write security-focused tests
   - Test edge cases and boundary conditions
   - Fuzz test when appropriate

## Known Security Considerations

### Validator Security

1. **Regular Expressions**
   - Complex regex can cause ReDoS attacks
   - We limit regex complexity in default validators
   - Use `regex` crate which is DoS-resistant

2. **Memory Usage**
   - Large inputs can cause memory exhaustion
   - Use size limits in validators
   - Consider streaming for large data

3. **Timing Attacks**
   - Some validators may leak information via timing
   - Use constant-time comparison for sensitive data
   - Consider using `subtle` crate for crypto operations

### Safe Usage Guidelines

```rust
// ✅ GOOD: Limit input size
let validator = StringValidator::new()
    .max_length(1000)
    .validate(user_input)?;

// ❌ BAD: Unbounded input
let validator = StringValidator::new()
    .validate(user_input)?; // Can exhaust memory

// ✅ GOOD: Use type-safe APIs
let email: Email = Email::try_from(input)?;

// ❌ BAD: Manual parsing
let parts: Vec<&str> = input.split('@').collect();

// ✅ GOOD: Sanitize outputs
let safe_output = html_escape(user_input);

// ❌ BAD: Direct output
println!("{}", user_input); // XSS risk in some contexts
```

## Vulnerability Disclosure Policy

We follow responsible disclosure:

1. **Grace Period**: 90 days before public disclosure
2. **Coordination**: We work with reporters to coordinate disclosure
3. **CVE Assignment**: We request CVEs for confirmed vulnerabilities
4. **Public Advisory**: Published after fix is released

## Security Features

### Current Security Features

- ✅ Type-safe validation APIs
- ✅ Memory-safe Rust implementation
- ✅ No unsafe code in core validators
- ✅ Input size limits
- ✅ ReDoS-resistant regex
- ✅ Comprehensive error handling

### Planned Security Features

- 🔄 Fuzzing infrastructure
- 🔄 Formal verification for critical paths
- 🔄 Security audit
- 🔄 Timing-attack resistant comparisons
- 🔄 Constant-time operations for crypto

## Security Advisories

Security advisories are published at:
- GitHub Security Advisories
- RustSec Advisory Database
- Project changelog

## Bug Bounty

Currently, we do not offer a bug bounty program. However, we greatly appreciate security researchers who help us improve Nebula's security.

Acknowledgments are provided in:
- Security advisories
- Release notes
- CONTRIBUTORS.md

## Contact

For security-related questions or concerns:

- **Email**: ivan.kondrashkin@gmail.com
- **Response Time**: Within 48 hours

## Legal

By reporting vulnerabilities, you agree to:
- Give us reasonable time to fix the issue
- Not exploit the vulnerability
- Not disclose the vulnerability publicly before it's fixed

We commit to:
- Acknowledge your report promptly
- Keep you updated on progress
- Credit you appropriately (unless you prefer anonymity)
- Not take legal action against good-faith security research

---

Thank you for helping keep Nebula and its users safe! 🔒
