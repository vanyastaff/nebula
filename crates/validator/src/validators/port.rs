//! Port number validator (1-65535).

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// PORT VALIDATOR
// ============================================================================

/// Validates TCP/UDP port numbers (1-65535).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Port;
/// use nebula_validator::foundation::Validate;
///
/// let validator = Port::new();
///
/// assert!(validator.validate(&80).is_ok());      // HTTP
/// assert!(validator.validate(&443).is_ok());     // HTTPS
/// assert!(validator.validate(&8080).is_ok());    // Alt HTTP
///
/// assert!(validator.validate(&0).is_err());      // Invalid (port 0)
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Port {
    allow_well_known: bool, // 1-1023
    allow_registered: bool, // 1024-49151
    allow_dynamic: bool,    // 49152-65535
}

impl Port {
    /// Creates a new port validator (allows all valid ports 1-65535).
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_well_known: true,
            allow_registered: true,
            allow_dynamic: true,
        }
    }

    /// Only allow well-known ports (1-1023).
    #[must_use = "builder methods must be chained or built"]
    pub fn well_known_only(mut self) -> Self {
        self.allow_well_known = true;
        self.allow_registered = false;
        self.allow_dynamic = false;
        self
    }

    /// Only allow registered ports (1024-49151).
    #[must_use = "builder methods must be chained or built"]
    pub fn registered_only(mut self) -> Self {
        self.allow_well_known = false;
        self.allow_registered = true;
        self.allow_dynamic = false;
        self
    }

    /// Only allow dynamic/private ports (49152-65535).
    #[must_use = "builder methods must be chained or built"]
    pub fn dynamic_only(mut self) -> Self {
        self.allow_well_known = false;
        self.allow_registered = false;
        self.allow_dynamic = true;
        self
    }

    /// Exclude well-known ports.
    #[must_use = "builder methods must be chained or built"]
    pub fn no_well_known(mut self) -> Self {
        self.allow_well_known = false;
        self
    }

    fn is_well_known(&self, port: u16) -> bool {
        (1..=1023).contains(&port)
    }

    fn is_registered(&self, port: u16) -> bool {
        (1024..=49151).contains(&port)
    }

    fn is_dynamic(&self, port: u16) -> bool {
        (49152..=65535).contains(&port)
    }
}

impl Default for Port {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for Port {
    type Input = u16;

    fn validate(&self, input: &u16) -> Result<(), ValidationError> {
        let port = *input;

        // Port 0 is invalid
        if port == 0 {
            return Err(ValidationError::new(
                "invalid_port",
                "Port number must be between 1 and 65535",
            ));
        }

        // Check ranges
        if self.is_well_known(port) && !self.allow_well_known {
            return Err(ValidationError::new(
                "well_known_port_not_allowed",
                format!("Well-known port {port} (1-1023) is not allowed"),
            ));
        }

        if self.is_registered(port) && !self.allow_registered {
            return Err(ValidationError::new(
                "registered_port_not_allowed",
                format!("Registered port {port} (1024-49151) is not allowed"),
            ));
        }

        if self.is_dynamic(port) && !self.allow_dynamic {
            return Err(ValidationError::new(
                "dynamic_port_not_allowed",
                format!("Dynamic port {port} (49152-65535) is not allowed"),
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        let mut tags = vec!["network".into(), "port".into()];

        if self.allow_well_known {
            tags.push("well-known".into());
        }
        if self.allow_registered {
            tags.push("registered".into());
        }
        if self.allow_dynamic {
            tags.push("dynamic".into());
        }

        ValidatorMetadata {
            name: "Port".into(),
            description: Some("Validates TCP/UDP port numbers (1-65535)".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_nanos(100)),
            tags,
            version: Some("1.0.0".into()),
            custom: Vec::new(),
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
    fn test_valid_ports() {
        let validator = Port::new();
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&80).is_ok());
        assert!(validator.validate(&443).is_ok());
        assert!(validator.validate(&8080).is_ok());
        assert!(validator.validate(&65535).is_ok());
    }

    #[test]
    fn test_invalid_port_zero() {
        let validator = Port::new();
        assert!(validator.validate(&0).is_err());
    }

    #[test]
    fn test_well_known_ports() {
        let validator = Port::new();
        assert!(validator.validate(&22).is_ok()); // SSH
        assert!(validator.validate(&80).is_ok()); // HTTP
        assert!(validator.validate(&443).is_ok()); // HTTPS
        assert!(validator.validate(&1023).is_ok());
    }

    #[test]
    fn test_registered_ports() {
        let validator = Port::new();
        assert!(validator.validate(&1024).is_ok());
        assert!(validator.validate(&8080).is_ok());
        assert!(validator.validate(&49151).is_ok());
    }

    #[test]
    fn test_dynamic_ports() {
        let validator = Port::new();
        assert!(validator.validate(&49152).is_ok());
        assert!(validator.validate(&60000).is_ok());
        assert!(validator.validate(&65535).is_ok());
    }

    #[test]
    fn test_well_known_only() {
        let validator = Port::new().well_known_only();
        assert!(validator.validate(&80).is_ok());
        assert!(validator.validate(&1024).is_err());
        assert!(validator.validate(&50000).is_err());
    }

    #[test]
    fn test_registered_only() {
        let validator = Port::new().registered_only();
        assert!(validator.validate(&8080).is_ok());
        assert!(validator.validate(&80).is_err());
        assert!(validator.validate(&50000).is_err());
    }

    #[test]
    fn test_dynamic_only() {
        let validator = Port::new().dynamic_only();
        assert!(validator.validate(&50000).is_ok());
        assert!(validator.validate(&80).is_err());
        assert!(validator.validate(&8080).is_err());
    }

    #[test]
    fn test_no_well_known() {
        let validator = Port::new().no_well_known();
        assert!(validator.validate(&80).is_err());
        assert!(validator.validate(&8080).is_ok());
        assert!(validator.validate(&50000).is_ok());
    }
}
