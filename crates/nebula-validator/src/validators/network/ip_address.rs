//! IP Address validators for IPv4 and IPv6.
//!
//! Validates IP addresses using Rust's standard library `std::net`.

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata, ValidationComplexity};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

// ============================================================================
// IP ADDRESS VALIDATOR
// ============================================================================

/// Validates IP addresses (both IPv4 and IPv6).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::IpAddress;
/// use nebula_validator::core::TypedValidator;
///
/// let validator = IpAddress::new();
///
/// // Valid IPv4
/// assert!(validator.validate("192.168.1.1").is_ok());
/// assert!(validator.validate("127.0.0.1").is_ok());
///
/// // Valid IPv6
/// assert!(validator.validate("2001:0db8:85a3:0000:0000:8a2e:0370:7334").is_ok());
/// assert!(validator.validate("::1").is_ok());
///
/// // Invalid
/// assert!(validator.validate("999.999.999.999").is_err());
/// assert!(validator.validate("not-an-ip").is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IpAddress {
    allow_v4: bool,
    allow_v6: bool,
}

impl IpAddress {
    /// Creates a new IP address validator (allows both IPv4 and IPv6).
    pub fn new() -> Self {
        Self {
            allow_v4: true,
            allow_v6: true,
        }
    }

    /// Only allow IPv4 addresses.
    pub fn v4_only(mut self) -> Self {
        self.allow_v4 = true;
        self.allow_v6 = false;
        self
    }

    /// Only allow IPv6 addresses.
    pub fn v6_only(mut self) -> Self {
        self.allow_v4 = false;
        self.allow_v6 = true;
        self
    }

    /// Check if address is private (RFC 1918, RFC 4193).
    pub fn is_private(&self, addr: &IpAddr) -> bool {
        match addr {
            IpAddr::V4(ipv4) => {
                // 10.0.0.0/8
                ipv4.octets()[0] == 10
                    // 172.16.0.0/12
                    || (ipv4.octets()[0] == 172 && (ipv4.octets()[1] & 0xf0) == 16)
                    // 192.168.0.0/16
                    || (ipv4.octets()[0] == 192 && ipv4.octets()[1] == 168)
            }
            IpAddr::V6(ipv6) => {
                // fc00::/7 (Unique Local Addresses)
                (ipv6.segments()[0] & 0xfe00) == 0xfc00
            }
        }
    }

    /// Check if address is loopback.
    pub fn is_loopback(&self, addr: &IpAddr) -> bool {
        addr.is_loopback()
    }
}

impl Default for IpAddress {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for IpAddress {
    type Input = str;
    type Output = IpAddr;
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        let addr = IpAddr::from_str(input).map_err(|_| {
            ValidationError::new("invalid_ip_address", format!("'{}' is not a valid IP address", input))
        })?;

        match addr {
            IpAddr::V4(_) if !self.allow_v4 => {
                Err(ValidationError::new("ipv4_not_allowed", "IPv4 addresses are not allowed"))
            }
            IpAddr::V6(_) if !self.allow_v6 => {
                Err(ValidationError::new("ipv6_not_allowed", "IPv6 addresses are not allowed"))
            }
            _ => Ok(addr),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "IpAddress".to_string(),
            description: Some(format!(
                "Validates IP addresses (IPv4: {}, IPv6: {})",
                self.allow_v4, self.allow_v6
            )),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec!["network".to_string(), "ip".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// IPV4 VALIDATOR
// ============================================================================

/// Validates IPv4 addresses only.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ipv4;

impl TypedValidator for Ipv4 {
    type Input = str;
    type Output = Ipv4Addr;
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        Ipv4Addr::from_str(input).map_err(|_| {
            ValidationError::new("invalid_ipv4", format!("'{}' is not a valid IPv4 address", input))
        })
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Ipv4".to_string(),
            description: Some("Validates IPv4 addresses".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(3)),
            tags: vec!["network".to_string(), "ipv4".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// IPV6 VALIDATOR
// ============================================================================

/// Validates IPv6 addresses only.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ipv6;

impl TypedValidator for Ipv6 {
    type Input = str;
    type Output = Ipv6Addr;
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        Ipv6Addr::from_str(input).map_err(|_| {
            ValidationError::new("invalid_ipv6", format!("'{}' is not a valid IPv6 address", input))
        })
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Ipv6".to_string(),
            description: Some("Validates IPv6 addresses".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(4)),
            tags: vec!["network".to_string(), "ipv6".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_address_valid_ipv4() {
        let validator = IpAddress::new();
        assert!(validator.validate("192.168.1.1").is_ok());
        assert!(validator.validate("127.0.0.1").is_ok());
        assert!(validator.validate("0.0.0.0").is_ok());
        assert!(validator.validate("255.255.255.255").is_ok());
    }

    #[test]
    fn test_ip_address_valid_ipv6() {
        let validator = IpAddress::new();
        assert!(validator.validate("2001:0db8:85a3:0000:0000:8a2e:0370:7334").is_ok());
        assert!(validator.validate("::1").is_ok());
        assert!(validator.validate("::").is_ok());
        assert!(validator.validate("fe80::1").is_ok());
    }

    #[test]
    fn test_ip_address_invalid() {
        let validator = IpAddress::new();
        assert!(validator.validate("999.999.999.999").is_err());
        assert!(validator.validate("192.168.1").is_err());
        assert!(validator.validate("not-an-ip").is_err());
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_ipv4_only() {
        let validator = IpAddress::new().v4_only();
        assert!(validator.validate("192.168.1.1").is_ok());
        assert!(validator.validate("::1").is_err());
    }

    #[test]
    fn test_ipv6_only() {
        let validator = IpAddress::new().v6_only();
        assert!(validator.validate("::1").is_ok());
        assert!(validator.validate("192.168.1.1").is_err());
    }

    #[test]
    fn test_is_private() {
        let validator = IpAddress::new();

        // Private IPv4
        let addr = validator.validate("192.168.1.1").unwrap();
        assert!(validator.is_private(&addr));

        let addr = validator.validate("10.0.0.1").unwrap();
        assert!(validator.is_private(&addr));

        let addr = validator.validate("172.16.0.1").unwrap();
        assert!(validator.is_private(&addr));

        // Public IPv4
        let addr = validator.validate("8.8.8.8").unwrap();
        assert!(!validator.is_private(&addr));
    }

    #[test]
    fn test_is_loopback() {
        let validator = IpAddress::new();

        let addr = validator.validate("127.0.0.1").unwrap();
        assert!(validator.is_loopback(&addr));

        let addr = validator.validate("::1").unwrap();
        assert!(validator.is_loopback(&addr));

        let addr = validator.validate("8.8.8.8").unwrap();
        assert!(!validator.is_loopback(&addr));
    }

    #[test]
    fn test_ipv4_validator() {
        let validator = Ipv4;
        assert!(validator.validate("192.168.1.1").is_ok());
        assert!(validator.validate("::1").is_err());
    }

    #[test]
    fn test_ipv6_validator() {
        let validator = Ipv6;
        assert!(validator.validate("::1").is_ok());
        assert!(validator.validate("192.168.1.1").is_err());
    }
}
