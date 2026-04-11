//! Environment types for webhook isolation

use std::fmt;

use serde::{Deserialize, Serialize};

/// Environment for webhook execution
///
/// Each environment has its own UUID namespace, ensuring that test
/// and production webhooks never interfere with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Test environment - for development and testing
    Test,
    /// Production environment - for live traffic
    #[default]
    Production,
}

impl Environment {
    /// Get the path prefix for this environment
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_webhook::Environment;
    ///
    /// assert_eq!(Environment::Test.path_prefix(), "test");
    /// assert_eq!(Environment::Production.path_prefix(), "prod");
    /// ```
    pub fn path_prefix(&self) -> &'static str {
        match self {
            Environment::Test => "test",
            Environment::Production => "prod",
        }
    }

    /// Check if this is the test environment
    pub fn is_test(&self) -> bool {
        matches!(self, Environment::Test)
    }

    /// Check if this is the production environment
    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Environment::Test => write!(f, "test"),
            Environment::Production => write!(f, "prod"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_prefix() {
        assert_eq!(Environment::Test.path_prefix(), "test");
        assert_eq!(Environment::Production.path_prefix(), "prod");
    }

    #[test]
    fn test_is_test() {
        assert!(Environment::Test.is_test());
        assert!(!Environment::Production.is_test());
    }

    #[test]
    fn test_is_production() {
        assert!(!Environment::Test.is_production());
        assert!(Environment::Production.is_production());
    }

    #[test]
    fn test_display() {
        assert_eq!(Environment::Test.to_string(), "test");
        assert_eq!(Environment::Production.to_string(), "prod");
    }

    #[test]
    fn test_default() {
        assert_eq!(Environment::default(), Environment::Production);
    }

    #[test]
    fn test_serde() {
        let test = Environment::Test;
        let json = serde_json::to_string(&test).unwrap();
        assert_eq!(json, "\"test\"");

        let prod = Environment::Production;
        let json = serde_json::to_string(&prod).unwrap();
        assert_eq!(json, "\"production\"");

        let deserialized: Environment = serde_json::from_str("\"test\"").unwrap();
        assert_eq!(deserialized, Environment::Test);
    }
}
