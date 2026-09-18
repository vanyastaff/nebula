//! Network address validators
//!
//! Validators for IP addresses, hostnames, and ports.
//! Uses `std::net` — no external dependencies.

use crate::foundation::{Validate, ValidationError};

/// Reject `input` unless it parses as `A`, naming the expected address family.
///
/// IPv4, IPv6, and the union all share this shape: `std::net`'s `FromStr` is
/// the parser, and the only difference is the rule code and human label.
fn require_parses<A>(input: &str, code: &'static str, label: &str) -> Result<(), ValidationError>
where
    A: std::str::FromStr,
{
    input.parse::<A>().map(|_| ()).map_err(|_| {
        ValidationError::new(code, format!("'{input}' is not a valid {label}"))
            .with_param("actual", input.to_string())
    })
}

// ============================================================================
// IPv4
// ============================================================================

/// Validates that a string is a valid IPv4 address (e.g. `"192.168.0.1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4;

impl Validate<str> for Ipv4 {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        require_parses::<std::net::Ipv4Addr>(input, "ipv4", "IPv4 address")
    }
}

/// Creates an IPv4 address validator.
#[must_use]
pub fn ipv4() -> Ipv4 {
    Ipv4
}

// ============================================================================
// IPv6
// ============================================================================

/// Validates that a string is a valid IPv6 address (e.g. `"::1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv6;

impl Validate<str> for Ipv6 {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        require_parses::<std::net::Ipv6Addr>(input, "ipv6", "IPv6 address")
    }
}

/// Creates an IPv6 address validator.
#[must_use]
pub fn ipv6() -> Ipv6 {
    Ipv6
}

// ============================================================================
// IP Address (v4 or v6)
// ============================================================================

/// Validates that a string is a valid IP address (IPv4 or IPv6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpAddr;

impl Validate<str> for IpAddr {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        require_parses::<std::net::IpAddr>(input, "ip_addr", "IP address")
    }
}

/// Creates an IP address validator (accepts both IPv4 and IPv6).
#[must_use]
pub fn ip_addr() -> IpAddr {
    IpAddr
}

// ============================================================================
// Hostname (RFC 1123)
// ============================================================================

/// Validates that a string is a valid hostname per RFC 1123.
///
/// Rules:
/// - Total length 1..=253
/// - Each label (dot-separated part) is 1..=63 characters
/// - Labels contain only `[a-zA-Z0-9-]`
/// - Labels do not start or end with a hyphen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hostname;

impl Validate<str> for Hostname {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() || input.len() > 253 {
            return Err(ValidationError::new(
                "hostname",
                "Hostname must be between 1 and 253 characters",
            )
            .with_param("actual_len", input.len().to_string()));
        }

        for label in input.trim_end_matches('.').split('.') {
            validate_label(label)?;
        }

        Ok(())
    }
}

/// Validate one dot-separated hostname label per RFC 1123.
fn validate_label(label: &str) -> Result<(), ValidationError> {
    let reason = if label.is_empty() || label.len() > 63 {
        "must be between 1 and 63 characters".to_string()
    } else if label.starts_with('-') || label.ends_with('-') {
        "must not start or end with a hyphen".to_string()
    } else if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        "contains invalid characters (only a-z, 0-9, - allowed)".to_string()
    } else {
        return Ok(());
    };

    Err(
        ValidationError::new("hostname", format!("Hostname label '{label}' {reason}"))
            .with_param("label", label.to_string()),
    )
}

/// Creates a hostname validator (RFC 1123).
#[must_use]
pub fn hostname() -> Hostname {
    Hostname
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

    // -- Ipv4 --

    #[test]
    fn ipv4_valid() {
        assert!(Ipv4.validate("192.168.0.1").is_ok());
        assert!(Ipv4.validate("0.0.0.0").is_ok());
        assert!(Ipv4.validate("255.255.255.255").is_ok());
    }

    #[test]
    fn ipv4_invalid() {
        assert!(Ipv4.validate("256.0.0.1").is_err());
        assert!(Ipv4.validate("192.168.0").is_err());
        assert!(Ipv4.validate("::1").is_err());
        assert!(Ipv4.validate("not-an-ip").is_err());
    }

    // -- Ipv6 --

    #[test]
    fn ipv6_valid() {
        assert!(Ipv6.validate("::1").is_ok());
        assert!(Ipv6.validate("2001:db8::1").is_ok());
        assert!(Ipv6.validate("fe80::1%eth0").is_err()); // zone id not supported by std
    }

    #[test]
    fn ipv6_invalid() {
        assert!(Ipv6.validate("192.168.0.1").is_err());
        assert!(Ipv6.validate("not-an-ip").is_err());
    }

    // -- IpAddr --

    #[test]
    fn ip_addr_accepts_both() {
        assert!(IpAddr.validate("192.168.0.1").is_ok());
        assert!(IpAddr.validate("::1").is_ok());
        assert!(IpAddr.validate("not-an-ip").is_err());
    }

    // -- Hostname --

    #[test]
    fn hostname_valid() {
        assert!(Hostname.validate("example.com").is_ok());
        assert!(Hostname.validate("sub.example.com").is_ok());
        assert!(Hostname.validate("localhost").is_ok());
        assert!(Hostname.validate("my-host").is_ok());
        assert!(Hostname.validate("xn--nxasmq6b").is_ok()); // punycode
    }

    #[test]
    fn hostname_invalid() {
        assert!(Hostname.validate("").is_err()); // empty
        assert!(Hostname.validate("-bad.com").is_err()); // starts with hyphen
        assert!(Hostname.validate("bad-.com").is_err()); // ends with hyphen
        assert!(Hostname.validate("bad_host.com").is_err()); // underscore
        assert!(Hostname.validate(&"a".repeat(254)).is_err()); // too long
        assert!(
            Hostname
                .validate(&format!("{}.com", "a".repeat(64)))
                .is_err()
        ); // label too long
    }
}
