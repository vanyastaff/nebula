---
title: "SecretString Usage: Redaction and Zeroization"
tags: [example, secret-string, security, zeroize, redaction]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [beginner, intermediate]
estimated_reading: 12
priority: P1
---

# SecretString Usage: Redaction and Zeroization

> **TL;DR**: Use SecretString to prevent accidental credential exposure in logs and ensure secure memory cleanup with automatic zeroization.

## Use Case

`SecretString` is a security-focused wrapper that prevents secrets from appearing in logs, debug output, error messages, or stack traces. It automatically zeroizes memory when dropped, protecting against memory dump attacks and accidental exposure.

**When to use**:
- Storing passwords, API keys, tokens, or any sensitive strings
- Preventing secrets from leaking into logs or monitoring systems
- Ensuring credentials are securely erased from memory
- Implementing security-compliant credential handling (SOC2, PCI-DSS, HIPAA)

## Prerequisites

- nebula-credential v0.1.0+
- Understanding of: [[Core-Concepts#SecretString]]
- Knowledge of Rust's Drop trait
- 10 minutes

## Full Code Example

```rust
// File: examples/secret_string_usage.rs
// Description: Comprehensive SecretString usage demonstrating redaction and zeroization
// 
// To run:
//   cargo run --example secret_string_usage

use nebula_credential::SecretString;
use std::fmt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔒 SecretString Usage Examples\n");
    
    // ============================================================================
    // Example 1: Basic Redaction
    // ============================================================================
    println!("=== Example 1: Automatic Redaction ===");
    
    let password = SecretString::from("super_secret_password_123");
    
    // ❌ Direct display is redacted
    println!("Password (direct): {}", password);
    // Output: SecretString([REDACTED])
    
    // ❌ Debug output is also redacted
    println!("Password (debug): {:?}", password);
    // Output: SecretString([REDACTED])
    
    // ❌ Even in error messages, secrets stay hidden
    let error = format!("Authentication failed for password: {}", password);
    println!("Error message: {}", error);
    // Output: Authentication failed for password: SecretString([REDACTED])
    
    println!("✓ Secrets never leak into logs\n");
    
    // ============================================================================
    // Example 2: Safe Access with expose_secret
    // ============================================================================
    println!("=== Example 2: Safe Secret Access ===");
    
    let api_key = SecretString::from("sk_live_abc123def456");
    
    // ✅ Access the secret within a closure scope
    api_key.expose_secret(|key| {
        println!("  Inside closure, key is: {}", key);
        
        // Perform operations with the key
        let key_length = key.len();
        let prefix = &key[..7];
        
        println!("  Key length: {}", key_length);
        println!("  Key prefix: {}", prefix);
        
        // Make API calls here
        // http_client.get("https://api.example.com").bearer_auth(key).send()
    });
    
    // ❌ Outside the closure, secret is redacted again
    println!("After closure: {}", api_key);
    // Output: SecretString([REDACTED])
    
    println!("✓ Secret only accessible within controlled scope\n");
    
    // ============================================================================
    // Example 3: Automatic Zeroization on Drop
    // ============================================================================
    println!("=== Example 3: Automatic Memory Zeroization ===");
    
    {
        let temp_secret = SecretString::from("temporary_secret_data");
        println!("  Secret created (redacted): {}", temp_secret);
        
        // Use the secret
        temp_secret.expose_secret(|s| {
            println!("  Using secret: {} bytes", s.len());
        });
        
        // When temp_secret goes out of scope, memory is zeroized
    } // <-- Memory containing "temporary_secret_data" is now zeroed
    
    println!("✓ Secret automatically zeroized when dropped\n");
    
    // ============================================================================
    // Example 4: SecretString in Structs
    // ============================================================================
    println!("=== Example 4: SecretString in Data Structures ===");
    
    #[derive(Debug)]
    struct UserCredentials {
        username: String,
        password: SecretString,  // Sensitive field
        email: String,
    }
    
    let user = UserCredentials {
        username: "alice".to_string(),
        password: SecretString::from("alice_password_456"),
        email: "alice@example.com".to_string(),
    };
    
    // Debug output redacts the password
    println!("User: {:?}", user);
    // Output: UserCredentials { username: "alice", password: SecretString([REDACTED]), email: "alice@example.com" }
    
    println!("✓ Struct fields automatically redacted\n");
    
    // ============================================================================
    // Example 5: Comparing Secrets Safely
    // ============================================================================
    println!("=== Example 5: Constant-Time Comparison ===");
    
    let secret1 = SecretString::from("my_secret_value");
    let secret2 = SecretString::from("my_secret_value");
    let secret3 = SecretString::from("different_value");
    
    // ✅ Constant-time comparison (prevents timing attacks)
    let are_equal = secret1.expose_secret(|s1| {
        secret2.expose_secret(|s2| {
            use subtle::ConstantTimeEq;
            s1.as_bytes().ct_eq(s2.as_bytes()).into()
        })
    });
    
    println!("  secret1 == secret2: {}", are_equal);
    
    let are_different = secret1.expose_secret(|s1| {
        secret3.expose_secret(|s3| {
            use subtle::ConstantTimeEq;
            !s1.as_bytes().ct_eq(s3.as_bytes()).into()
        })
    });
    
    println!("  secret1 != secret3: {}", are_different);
    println!("✓ Secrets compared safely without timing leaks\n");
    
    // ============================================================================
    // Example 6: Logging Best Practices
    // ============================================================================
    println!("=== Example 6: Safe Logging Practices ===");
    
    let db_password = SecretString::from("database_password_789");
    
    // ❌ This is safe - password is automatically redacted
    log_authentication_attempt("user123", &db_password);
    
    // ✅ Log only non-sensitive metadata
    println!("  Logging metadata only:");
    db_password.expose_secret(|pwd| {
        println!("    - Password length: {}", pwd.len());
        println!("    - Has special chars: {}", pwd.chars().any(|c| !c.is_alphanumeric()));
        // Never log the actual password!
    });
    
    println!("✓ Logs contain no sensitive data\n");
    
    // ============================================================================
    // Example 7: Error Handling Without Leaks
    // ============================================================================
    println!("=== Example 7: Error Handling ===");
    
    let result = authenticate_user("admin", SecretString::from("wrong_password"));
    
    match result {
        Ok(_) => println!("  Authentication successful"),
        Err(e) => {
            // Error message doesn't contain the secret
            println!("  Error: {}", e);
        }
    }
    
    println!("✓ Errors don't leak secrets\n");
    
    // ============================================================================
    // Example 8: Converting and Cloning
    // ============================================================================
    println!("=== Example 8: Conversions ===");
    
    // From String
    let from_string = SecretString::from("secret".to_string());
    println!("  From String: {}", from_string);
    
    // From &str
    let from_str = SecretString::from("another_secret");
    println!("  From &str: {}", from_str);
    
    // Clone (creates new protected copy)
    let cloned = from_str.clone();
    println!("  Cloned: {}", cloned);
    
    println!("✓ All conversions maintain protection\n");
    
    println!("🎉 SecretString usage examples complete!");
    
    Ok(())
}

// Helper function: Safe logging
fn log_authentication_attempt(username: &str, password: &SecretString) {
    println!("  🔐 Authentication attempt:");
    println!("    - Username: {}", username);
    println!("    - Password: {}", password);  // Automatically redacted
}

// Helper function: Error handling example
fn authenticate_user(username: &str, password: SecretString) -> Result<(), String> {
    // Simulate authentication logic
    password.expose_secret(|pwd| {
        if pwd == "correct_password" {
            Ok(())
        } else {
            // ✅ Error message doesn't contain the password
            Err(format!("Authentication failed for user: {}", username))
        }
    })
}
```

## Dependencies

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }

# For constant-time comparison (included with nebula-credential)
subtle = "2.5"
```

## Explanation of Key Parts

### Part 1: Automatic Redaction

```rust
let password = SecretString::from("super_secret_password_123");
println!("Password: {}", password);
// Output: SecretString([REDACTED])
```

**What's happening**:
- `SecretString` implements custom `Display` and `Debug` traits
- Both traits return `"SecretString([REDACTED])"` instead of the actual value
- This prevents accidental exposure in logs, error messages, or debug output
- Even if you try to print it, you'll only see `[REDACTED]`

### Part 2: Controlled Access with expose_secret

```rust
api_key.expose_secret(|key| {
    println!("Key: {}", key);
    make_api_call(key);
});
```

**What's happening**:
- `expose_secret()` is the **only** way to access the underlying value
- It takes a closure that receives a reference to the secret
- The secret is only accessible within the closure scope
- This forces developers to consciously handle secrets
- Prevents accidental leakage outside the controlled scope

### Part 3: Automatic Zeroization

```rust
{
    let secret = SecretString::from("sensitive_data");
    // Use secret
} // <-- Memory is zeroized here
```

**What's happening**:
- `SecretString` implements `Zeroize` and `ZeroizeOnDrop` traits
- When the `SecretString` is dropped (goes out of scope), its memory is overwritten with zeros
- This prevents secrets from lingering in memory after use
- Protects against memory dump attacks and forensic analysis
- No manual cleanup required - it's automatic

## Expected Output

When you run the example, you should see:

```
🔒 SecretString Usage Examples

=== Example 1: Automatic Redaction ===
Password (direct): SecretString([REDACTED])
Password (debug): SecretString([REDACTED])
Error message: Authentication failed for password: SecretString([REDACTED])
✓ Secrets never leak into logs

=== Example 2: Safe Secret Access ===
  Inside closure, key is: sk_live_abc123def456
  Key length: 20
  Key prefix: sk_live
After closure: SecretString([REDACTED])
✓ Secret only accessible within controlled scope

=== Example 3: Automatic Memory Zeroization ===
  Secret created (redacted): SecretString([REDACTED])
  Using secret: 21 bytes
✓ Secret automatically zeroized when dropped

=== Example 4: SecretString in Data Structures ===
User: UserCredentials { username: "alice", password: SecretString([REDACTED]), email: "alice@example.com" }
✓ Struct fields automatically redacted

=== Example 5: Constant-Time Comparison ===
  secret1 == secret2: true
  secret1 != secret3: true
✓ Secrets compared safely without timing leaks

=== Example 6: Safe Logging Practices ===
  🔐 Authentication attempt:
    - Username: user123
    - Password: SecretString([REDACTED])
  Logging metadata only:
    - Password length: 21
    - Has special chars: true
✓ Logs contain no sensitive data

=== Example 7: Error Handling ===
  Error: Authentication failed for user: admin
✓ Errors don't leak secrets

=== Example 8: Conversions ===
  From String: SecretString([REDACTED])
  From &str: SecretString([REDACTED])
  Cloned: SecretString([REDACTED])
✓ All conversions maintain protection

🎉 SecretString usage examples complete!
```

## Variations

### Variation 1: SecretString with Serde (Serialization)

For APIs that need JSON serialization without exposing secrets:

```rust
use serde::{Deserialize, Serialize};
use nebula_credential::SecretString;

#[derive(Serialize, Deserialize)]
struct ApiConfig {
    endpoint: String,
    #[serde(with = "secret_string_serde")]
    api_key: SecretString,
}

// Custom serialization module
mod secret_string_serde {
    use nebula_credential::SecretString;
    use serde::{Deserialize, Deserializer, Serializer};
    
    pub fn serialize<S>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as [REDACTED] instead of the actual value
        serializer.serialize_str("[REDACTED]")
    }
    
    pub fn deserialize<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(SecretString::from(s))
    }
}

// Usage
let config = ApiConfig {
    endpoint: "https://api.example.com".to_string(),
    api_key: SecretString::from("sk_live_secret"),
};

let json = serde_json::to_string(&config)?;
println!("{}", json);
// Output: {"endpoint":"https://api.example.com","api_key":"[REDACTED]"}
```

### Variation 2: SecretString in Error Types

Create error types that safely handle secrets:

```rust
use nebula_credential::SecretString;
use std::fmt;

#[derive(Debug)]
enum AuthError {
    InvalidCredentials {
        username: String,
        password: SecretString,  // Stored but never displayed
    },
    NetworkError(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::InvalidCredentials { username, password } => {
                // Password is automatically redacted
                write!(f, "Invalid credentials for user: {} (password: {})", username, password)
            }
            AuthError::NetworkError(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

impl std::error::Error for AuthError {}

// Usage
let error = AuthError::InvalidCredentials {
    username: "alice".to_string(),
    password: SecretString::from("alice_password"),
};

println!("{}", error);
// Output: Invalid credentials for user: alice (password: SecretString([REDACTED]))
```

### Variation 3: Temporary Exposure with Guard

For cases requiring temporary exposure with automatic cleanup:

```rust
use nebula_credential::SecretString;
use std::ops::Deref;

struct SecretGuard<'a> {
    secret: &'a str,
}

impl<'a> Deref for SecretGuard<'a> {
    type Target = str;
    
    fn deref(&self) -> &Self::Target {
        self.secret
    }
}

impl<'a> Drop for SecretGuard<'a> {
    fn drop(&mut self) {
        println!("  🧹 SecretGuard dropped, secret no longer accessible");
    }
}

fn expose_with_guard(secret: &SecretString) -> SecretGuard {
    secret.expose_secret(|s| SecretGuard { secret: s })
}

// Usage
let api_key = SecretString::from("sk_live_abc123");

{
    let guard = expose_with_guard(&api_key);
    println!("  Using secret: {}", &*guard);
} // <-- Guard drops here, cleanup happens

println!("✓ Secret access revoked");
```

## Important Notes

> [!warning] Critical Security Rule
> **NEVER** extract the secret from `expose_secret()` closure:
> ```rust
> // ❌ NEVER DO THIS - defeats the purpose
> let leaked = secret.expose_secret(|s| s.to_string());
> ```
> Always perform operations **inside** the closure.

> [!tip] Best Practice: Logging
> When logging operations involving secrets:
> - Log only metadata (length, format validation results, timestamps)
> - Never log the actual secret value
> - Use structured logging with explicit field redaction
> - Example: `log!("API call with key_length: {}", key.len())`

> [!tip] Best Practice: Error Messages
> Structure errors to be informative without exposing secrets:
> ```rust
> // ✅ GOOD: Describes the problem without revealing secrets
> return Err("Invalid API key format: expected 'sk_live_' prefix");
> 
> // ❌ BAD: Exposes the secret in the error
> return Err(format!("Invalid API key: {}", key));
> ```

> [!info] Memory Safety
> `SecretString` uses the `zeroize` crate which provides:
> - Compiler fence to prevent optimization removal
> - Volatile writes to ensure zeroing happens
> - Works even in release builds with aggressive optimizations

## Common Pitfalls

**❌ DON'T**: Clone the secret outside the closure

```rust
// WRONG - defeats the purpose
let leaked = secret.expose_secret(|s| s.clone());
println!("{}", leaked);  // Secret is now exposed!
```

**✅ DO**: Keep secrets inside the closure

```rust
// CORRECT
secret.expose_secret(|s| {
    make_api_call(s);  // Use it here
});
// Secret never leaves the closure
```

**❌ DON'T**: Store secrets in regular Strings

```rust
// WRONG - no protection
let password = String::from("my_password");
println!("{}", password);  // Exposed!
```

**✅ DO**: Always use SecretString for sensitive data

```rust
// CORRECT
let password = SecretString::from("my_password");
println!("{}", password);  // Redacted
```

## Related Examples

- **API Key Management**: [[Examples/API-Key-Basic]] - Using SecretString with API keys
- **OAuth2 Tokens**: [[Examples/OAuth2-GitHub]] - SecretString in OAuth2 workflows
- **Database Credentials**: [[Examples/Database-Rotation]] - Secure database password handling

## See Also

- **Concept**: [[Core-Concepts#SecretString]] - Understanding SecretString design
- **Concept**: [[Security/Encryption#Memory Protection]] - Memory security details
- **How-To**: [[How-To/Store-Credentials]] - Storing SecretString values
- **Reference**: [[API-Reference#SecretString]] - Complete SecretString API
- **Architecture**: [[Architecture#Type System]] - Type-level security guarantees
- **Troubleshooting**: [[Troubleshooting/Common-Issues#SecretString Issues]] - Common problems

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Demonstrates redaction in multiple contexts (display, debug, errors)
- [x] Shows proper expose_secret usage
- [x] Explains zeroization behavior
- [x] Includes constant-time comparison example
- [x] Three practical variations provided
- [x] Security warnings for common pitfalls
- [x] Best practices for logging and error handling
