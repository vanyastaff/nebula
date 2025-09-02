//! Identifier types used throughout the validation system

use std::fmt::{self, Display};
use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique identifier for a validator
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValidatorId(String);

impl ValidatorId {
    /// Create a new validator ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Generate a unique validator ID
    pub fn generate() -> Self {
        Self(format!("validator_{}", uuid::Uuid::new_v4()))
    }
    
    /// Get the ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
    
    /// Convert to string
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Display for ValidatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ValidatorId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ValidatorId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Unique identifier for a validation instance
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValidationId(uuid::Uuid);

impl ValidationId {
    /// Create a new validation ID
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    
    /// Create from UUID
    pub fn from_uuid(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
    
    /// Get as UUID
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
    
    /// Convert to string
    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl Default for ValidationId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for ValidationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a validation proof
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProofId(uuid::Uuid);

impl ProofId {
    /// Create a new proof ID
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    
    /// Get as UUID
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl Default for ProofId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for ProofId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
