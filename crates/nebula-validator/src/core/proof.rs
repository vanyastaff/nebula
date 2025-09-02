//! Validation proof system for nebula-validator

use std::fmt::{self, Debug};
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use crate::types::{ValidatorId, ValidationId, ProofId};

/// Type of validation proof
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofType {
    /// Simple validation proof
    Simple,
    /// Composite validation proof (multiple validators)
    Composite,
    /// Cached validation proof
    Cached,
    /// Delegated validation proof
    Delegated,
    /// Custom proof type
    Custom(String),
}

/// Validation proof that demonstrates a value has been validated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationProof {
    /// Unique identifier for this proof
    pub id: ProofId,
    /// Type of proof
    pub proof_type: ProofType,
    /// ID of the validation session
    pub validation_id: ValidationId,
    /// ID of the validator that created this proof
    pub validator_id: ValidatorId,
    /// When this proof was created
    pub created_at: DateTime<Utc>,
    /// When this proof expires (if applicable)
    pub expires_at: Option<DateTime<Utc>>,
    /// Additional proof data
    pub data: ProofData,
}

/// Proof data container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofData {
    /// Raw proof data
    pub raw: serde_json::Value,
    /// Signature or hash (if applicable)
    pub signature: Option<String>,
    /// Metadata
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

impl ValidationProof {
    /// Create a simple validation proof
    pub fn simple(validator_id: ValidatorId) -> Self {
        Self {
            id: ProofId::new(),
            proof_type: ProofType::Simple,
            validation_id: ValidationId::new(),
            validator_id,
            created_at: Utc::now(),
            expires_at: None,
            data: ProofData {
                raw: serde_json::Value::Null,
                signature: None,
                metadata: std::collections::HashMap::new(),
            },
        }
    }
    
    /// Create a composite proof from multiple validators
    pub fn composite(validators: Vec<ValidatorId>) -> Self {
        Self {
            id: ProofId::new(),
            proof_type: ProofType::Composite,
            validation_id: ValidationId::new(),
            validator_id: ValidatorId::composite(validators),
            created_at: Utc::now(),
            expires_at: None,
            data: ProofData {
                raw: serde_json::Value::Null,
                signature: None,
                metadata: std::collections::HashMap::new(),
            },
        }
    }
    
    /// Check if the proof has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }
    
    /// Set expiration time
    pub fn with_expiration(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
    
    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.data.metadata.insert(key.into(), value.into());
        self
    }
}

/// Builder for validation proofs
#[derive(Debug, Default)]
pub struct ProofBuilder {
    proof_type: Option<ProofType>,
    expires_at: Option<DateTime<Utc>>,
    metadata: std::collections::HashMap<String, serde_json::Value>,
}

impl ProofBuilder {
    /// Create a new proof builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the proof type
    pub fn proof_type(mut self, proof_type: ProofType) -> Self {
        self.proof_type = Some(proof_type);
        self
    }
    
    /// Set expiration time
    pub fn expires_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
    
    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
    
    /// Build the proof
    pub fn build(self, validator_id: ValidatorId) -> ValidationProof {
        let proof_type = self.proof_type.unwrap_or(ProofType::Simple);
        
        ValidationProof {
            id: ProofId::new(),
            proof_type,
            validation_id: ValidationId::new(),
            validator_id,
            created_at: Utc::now(),
            expires_at: self.expires_at,
            data: ProofData {
                raw: serde_json::Value::Null,
                signature: None,
                metadata: self.metadata,
            },
        }
    }
}

/// Extension trait for validation proofs
pub trait ProofExt {
    /// Get the proof ID as a string
    fn id_string(&self) -> String;
    
    /// Check if the proof is still valid (not expired)
    fn is_valid(&self) -> bool;
    
    /// Get the age of the proof
    fn age(&self) -> std::time::Duration;
}

impl ProofExt for ValidationProof {
    fn id_string(&self) -> String {
        self.id.to_string()
    }
    
    fn is_valid(&self) -> bool {
        !self.is_expired()
    }
    
    fn age(&self) -> std::time::Duration {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.created_at);
        std::time::Duration::from_secs(duration.num_seconds() as u64)
    }
}