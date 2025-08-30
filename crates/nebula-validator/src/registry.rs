//! Validator registry for managing and discovering validators
//! 
//! This module provides a centralized registry for all available validators,
//! allowing for dynamic discovery, registration, and management of validation rules.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use crate::traits::Validatable;
use crate::types::{ValidatorId, ValidatorMetadata, ValidatorCategory, ValidationConfig};

// ==================== Validator Registry ====================

/// Central registry for managing all available validators
/// 
/// The registry provides a centralized way to discover, register, and manage
/// validators throughout the validation system. It supports dynamic registration
/// and provides metadata about available validators.
#[derive(Debug)]
pub struct ValidatorRegistry {
    /// Registered validators by ID
    validators: Arc<RwLock<HashMap<ValidatorId, RegisteredValidator>>>,
    /// Validators by category for quick lookup
    validators_by_category: Arc<RwLock<HashMap<ValidatorCategory, Vec<ValidatorId>>>>,
    /// Validators by tag for flexible grouping
    validators_by_tag: Arc<RwLock<HashMap<String, Vec<ValidatorId>>>>,
    /// Configuration for the registry
    config: ValidationConfig,
}

impl ValidatorRegistry {
    /// Create a new validator registry
    pub fn new(config: ValidationConfig) -> Self {
        info!("Creating new validator registry with config: {:?}", config);
        
        Self {
            validators: Arc::new(RwLock::new(HashMap::new())),
            validators_by_category: Arc::new(RwLock::new(HashMap::new())),
            validators_by_tag: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }
    
    /// Create a new registry with default configuration
    pub fn new_default() -> Self {
        Self::new(ValidationConfig::default())
    }
    
    /// Register a new validator
    /// 
    /// # Arguments
    /// * `validator` - The validator to register
    /// * `metadata` - Metadata describing the validator
    /// 
    /// # Returns
    /// * `Result<(), RegistryError>` - Success or failure
    pub async fn register<V: Validatable + 'static>(
        &self,
        validator: V,
        metadata: ValidatorMetadata,
    ) -> Result<(), RegistryError> {
        let id = metadata.id.clone();
        let category = metadata.category.clone();
        let tags = metadata.tags.clone();
        
        debug!("Registering validator: {} ({})", metadata.name, id.as_str());
        
        // Check if validator already exists
        {
            let validators = self.validators.read().await;
            if validators.contains_key(&id) {
                return Err(RegistryError::ValidatorAlreadyExists(id));
            }
        }
        
        // Register the validator
        {
            let mut validators = self.validators.write().await;
            let registered = RegisteredValidator {
                validator: Arc::new(validator),
                metadata: metadata.clone(),
            };
            validators.insert(id.clone(), registered);
        }
        
        // Update category index
        {
            let mut by_category = self.validators_by_category.write().await;
            by_category
                .entry(category)
                .or_insert_with(Vec::new)
                .push(id.clone());
        }
        
        // Update tag index
        {
            let mut by_tag = self.validators_by_tag.write().await;
            for tag in tags {
                by_tag
                    .entry(tag)
                    .or_insert_with(Vec::new)
                    .push(id.clone());
            }
        }
        
        info!("Successfully registered validator: {} ({})", metadata.name, id.as_str());
        Ok(())
    }
    
    /// Unregister a validator
    /// 
    /// # Arguments
    /// * `id` - ID of the validator to unregister
    /// 
    /// # Returns
    /// * `Result<(), RegistryError>` - Success or failure
    pub async fn unregister(&self, id: &ValidatorId) -> Result<(), RegistryError> {
        debug!("Unregistering validator: {}", id.as_str());
        
        // Get validator metadata for cleanup
        let metadata = {
            let validators = self.validators.read().await;
            validators
                .get(id)
                .map(|v| v.metadata.clone())
                .ok_or(RegistryError::ValidatorNotFound(id.clone()))?
        };
        
        // Remove from main registry
        {
            let mut validators = self.validators.write().await;
            validators.remove(id);
        }
        
        // Remove from category index
        {
            let mut by_category = self.validators_by_category.write().await;
            if let Some(ids) = by_category.get_mut(&metadata.category) {
                ids.retain(|x| x != id);
                if ids.is_empty() {
                    by_category.remove(&metadata.category);
                }
            }
        }
        
        // Remove from tag indices
        {
            let mut by_tag = self.validators_by_tag.write().await;
            for tag in &metadata.tags {
                if let Some(ids) = by_tag.get_mut(tag) {
                    ids.retain(|x| x != id);
                    if ids.is_empty() {
                        by_tag.remove(tag);
                    }
                }
            }
        }
        
        info!("Successfully unregistered validator: {} ({})", metadata.name, id.as_str());
        Ok(())
    }
    
    /// Get a validator by ID
    /// 
    /// # Arguments
    /// * `id` - ID of the validator to retrieve
    /// 
    /// # Returns
    /// * `Option<Arc<dyn Validatable>>` - The validator if found
    pub async fn get(&self, id: &ValidatorId) -> Option<Arc<dyn Validatable>> {
        let validators = self.validators.read().await;
        validators.get(id).map(|v| v.validator.clone())
    }
    
    /// Get validator metadata by ID
    /// 
    /// # Arguments
    /// * `id` - ID of the validator
    /// 
    /// # Returns
    /// * `Option<ValidatorMetadata>` - The metadata if found
    pub async fn get_metadata(&self, id: &ValidatorId) -> Option<ValidatorMetadata> {
        let validators = self.validators.read().await;
        validators.get(id).map(|v| v.metadata.clone())
    }
    
    /// List all registered validators
    /// 
    /// # Returns
    /// * `Vec<ValidatorMetadata>` - List of all validator metadata
    pub async fn list_all(&self) -> Vec<ValidatorMetadata> {
        let validators = self.validators.read().await;
        validators.values().map(|v| v.metadata.clone()).collect()
    }
    
    /// List validators by category
    /// 
    /// # Arguments
    /// * `category` - Category to filter by
    /// 
    /// # Returns
    /// * `Vec<ValidatorMetadata>` - List of validators in the category
    pub async fn list_by_category(&self, category: &ValidatorCategory) -> Vec<ValidatorMetadata> {
        let by_category = self.validators_by_category.read().await;
        let ids = by_category.get(category).cloned().unwrap_or_default();
        
        let validators = self.validators.read().await;
        ids.into_iter()
            .filter_map(|id| validators.get(&id).map(|v| v.metadata.clone()))
            .collect()
    }
    
    /// List validators by tag
    /// 
    /// # Arguments
    /// * `tag` - Tag to filter by
    /// 
    /// # Returns
    /// * `Vec<ValidatorMetadata>` - List of validators with the tag
    pub async fn list_by_tag(&self, tag: &str) -> Vec<ValidatorMetadata> {
        let by_tag = self.validators_by_tag.read().await;
        let ids = by_tag.get(tag).cloned().unwrap_or_default();
        
        let validators = self.validators.read().await;
        ids.into_iter()
            .filter_map(|id| validators.get(&id).map(|v| v.metadata.clone()))
            .collect()
    }
    
    /// Search validators by name pattern
    /// 
    /// # Arguments
    /// * `pattern` - Pattern to search for in validator names
    /// 
    /// # Returns
    /// * `Vec<ValidatorMetadata>` - List of matching validators
    pub async fn search_by_name(&self, pattern: &str) -> Vec<ValidatorMetadata> {
        let validators = self.validators.read().await;
        validators
            .values()
            .filter(|v| v.metadata.name.to_lowercase().contains(&pattern.to_lowercase()))
            .map(|v| v.metadata.clone())
            .collect()
    }
    
    /// Get registry statistics
    /// 
    /// # Returns
    /// * `RegistryStats` - Statistics about the registry
    pub async fn stats(&self) -> RegistryStats {
        let validators = self.validators.read().await;
        let by_category = self.validators_by_category.read().await;
        let by_tag = self.validators_by_tag.read().await;
        
        let total_validators = validators.len();
        let categories = by_category.len();
        let tags = by_tag.len();
        
        let mut category_counts = HashMap::new();
        for (category, ids) in by_category.iter() {
            category_counts.insert(category.clone(), ids.len());
        }
        
        RegistryStats {
            total_validators,
            categories,
            tags,
            category_counts,
        }
    }
    
    /// Clear all validators from the registry
    pub async fn clear(&self) {
        info!("Clearing all validators from registry");
        
        {
            let mut validators = self.validators.write().await;
            validators.clear();
        }
        
        {
            let mut by_category = self.validators_by_category.write().await;
            by_category.clear();
        }
        
        {
            let mut by_tag = self.validators_by_tag.write().await;
            by_tag.clear();
        }
        
        info!("Registry cleared successfully");
    }
    
    /// Check if a validator exists
    /// 
    /// # Arguments
    /// * `id` - ID of the validator to check
    /// 
    /// # Returns
    /// * `bool` - True if the validator exists
    pub async fn exists(&self, id: &ValidatorId) -> bool {
        let validators = self.validators.read().await;
        validators.contains_key(id)
    }
    
    /// Get the number of registered validators
    /// 
    /// # Returns
    /// * `usize` - Number of validators
    pub async fn count(&self) -> usize {
        let validators = self.validators.read().await;
        validators.len()
    }
}

// ==================== Registered Validator ====================

/// Internal representation of a registered validator
#[derive(Debug)]
struct RegisteredValidator {
    /// The actual validator implementation
    validator: Arc<dyn Validatable>,
    /// Metadata describing the validator
    metadata: ValidatorMetadata,
}

// ==================== Registry Statistics ====================

/// Statistics about the validator registry
#[derive(Debug, Clone)]
pub struct RegistryStats {
    /// Total number of registered validators
    pub total_validators: usize,
    /// Number of categories with validators
    pub categories: usize,
    /// Number of unique tags
    pub tags: usize,
    /// Count of validators per category
    pub category_counts: HashMap<ValidatorCategory, usize>,
}

// ==================== Registry Errors ====================

/// Errors that can occur during registry operations
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Validator already exists in the registry
    #[error("Validator with ID '{}' already exists", .0.as_str())]
    ValidatorAlreadyExists(ValidatorId),
    
    /// Validator not found in the registry
    #[error("Validator with ID '{}' not found", .0.as_str())]
    ValidatorNotFound(ValidatorId),
    
    /// Invalid validator metadata
    #[error("Invalid validator metadata: {}", .0)]
    InvalidMetadata(String),
    
    /// Registry operation failed
    #[error("Registry operation failed: {}", .0)]
    OperationFailed(String),
}

// ==================== Registry Builder ====================

/// Builder for creating validator registries with custom configuration
#[derive(Debug)]
pub struct RegistryBuilder {
    config: ValidationConfig,
}

impl RegistryBuilder {
    /// Create a new registry builder
    pub fn new() -> Self {
        Self {
            config: ValidationConfig::default(),
        }
    }
    
    /// Set custom configuration
    pub fn with_config(mut self, config: ValidationConfig) -> Self {
        self.config = config;
        self
    }
    
    /// Disable caching
    pub fn without_caching(mut self) -> Self {
        self.config.enable_caching = false;
        self
    }
    
    /// Set cache TTL
    pub fn with_cache_ttl(mut self, ttl_seconds: u64) -> Self {
        self.config.cache_ttl_seconds = ttl_seconds;
        self
    }
    
    /// Set maximum depth
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.config.max_depth = depth;
        self
    }
    
    /// Set performance budget
    pub fn with_performance_budget(mut self, budget_ms: u64) -> Self {
        self.config.performance_budget_ms = budget_ms;
        self
    }
    
    /// Build the registry
    pub fn build(self) -> ValidatorRegistry {
        ValidatorRegistry::new(self.config)
    }
}

impl Default for RegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Registry Extensions ====================

/// Extension trait for adding convenience methods to the registry
#[async_trait::async_trait]
pub trait RegistryExt {
    /// Register a validator with auto-generated metadata
    async fn register_simple<V: Validatable + 'static>(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        category: ValidatorCategory,
        validator: V,
    ) -> Result<(), RegistryError>;
    
    /// Register multiple validators at once
    async fn register_batch(
        &self,
        validators: Vec<(Box<dyn Validatable + Send + Sync>, ValidatorMetadata)>,
    ) -> Result<(), RegistryError>;
    
    /// Get validators by multiple categories
    async fn get_by_categories(
        &self,
        categories: &[ValidatorCategory],
    ) -> Vec<ValidatorMetadata>;
    
    /// Get validators by multiple tags
    async fn get_by_tags(
        &self,
        tags: &[String],
    ) -> Vec<ValidatorMetadata>;
}

#[async_trait::async_trait]
impl RegistryExt for ValidatorRegistry {
    async fn register_simple<V: Validatable + 'static>(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        category: ValidatorCategory,
        validator: V,
    ) -> Result<(), RegistryError> {
        let metadata = ValidatorMetadata::new(id, name, category);
        self.register(validator, metadata).await
    }
    
    async fn register_batch(
        &self,
        validators: Vec<(Box<dyn Validatable + Send + Sync>, ValidatorMetadata)>,
    ) -> Result<(), RegistryError> {
        for (validator, metadata) in validators {
            // Convert Box<dyn Validatable> to concrete type for registration
            // This is a limitation of the current design - we'd need to use
            // type erasure or a different approach for truly dynamic registration
            return Err(RegistryError::OperationFailed(
                "Batch registration not yet implemented".to_string()
            ));
        }
        Ok(())
    }
    
    async fn get_by_categories(
        &self,
        categories: &[ValidatorCategory],
    ) -> Vec<ValidatorMetadata> {
        let mut result = Vec::new();
        for category in categories {
            let validators = self.list_by_category(category).await;
            result.extend(validators);
        }
        result
    }
    
    async fn get_by_tags(
        &self,
        tags: &[String],
    ) -> Vec<ValidatorMetadata> {
        let mut result = Vec::new();
        for tag in tags {
            let validators = self.list_by_tag(tag).await;
            result.extend(validators);
        }
        result
    }
}

// ==================== Re-exports ====================

pub use ValidatorRegistry as Registry;
pub use RegistryBuilder as Builder;
pub use RegistryExt as Ext;
pub use RegistryStats as Stats;
pub use RegistryError as Error;
