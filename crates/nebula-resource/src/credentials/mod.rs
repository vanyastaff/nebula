//! Credential integration for secure resource authentication
//!
//! This module provides integration with nebula-credential for secure
//! credential management, automatic token refresh, and credential rotation.

#[cfg(feature = "credentials")]
use nebula_credential::prelude::*;

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
};
use std::sync::Arc;

#[cfg(feature = "credentials")]
use tokio::sync::RwLock;

/// Credential provider for resources
#[cfg(feature = "credentials")]
pub struct ResourceCredentialProvider {
    manager: Arc<CredentialManager>,
    credential_id: nebula_credential::core::CredentialId,
    cached_token: Arc<RwLock<Option<CachedCredential>>>,
}

#[cfg(feature = "credentials")]
struct CachedCredential {
    token: AccessToken,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(feature = "credentials")]
impl ResourceCredentialProvider {
    /// Create a new credential provider
    pub fn new(
        manager: Arc<CredentialManager>,
        credential_id: nebula_credential::core::CredentialId,
    ) -> Self {
        Self {
            manager,
            credential_id,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Get access token, refreshing if needed
    pub async fn get_token(&self) -> ResourceResult<AccessToken> {
        // Check cache first
        {
            let cached = self.cached_token.read().await;
            if let Some(ref cred) = *cached {
                let now = chrono::Utc::now();
                // Use token if it expires in more than 5 minutes
                if cred.expires_at > now + chrono::Duration::minutes(5) {
                    return Ok(cred.token.clone());
                }
            }
        }

        // Get fresh token from manager
        let token = self
            .manager
            .get_token(&self.credential_id)
            .await
            .map_err(|e| {
                ResourceError::internal(
                    "credential_provider",
                    format!("Failed to get credential: {}", e),
                )
            })?;

        // Update cache
        {
            let mut cached = self.cached_token.write().await;
            *cached = Some(CachedCredential {
                token: token.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1), // Default 1 hour
            });
        }

        Ok(token)
    }

    /// Get token as string (for convenience)
    pub async fn get_token_string(&self) -> ResourceResult<String> {
        let token = self.get_token().await?;
        Ok(token.to_string())
    }

    /// Clear cached token
    pub async fn invalidate(&self) {
        let mut cached = self.cached_token.write().await;
        *cached = None;
    }

    /// Get credential ID
    pub fn credential_id(&self) -> &nebula_credential::core::CredentialId {
        &self.credential_id
    }
}

/// Configuration for credential-based resources
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CredentialConfig {
    /// Credential ID to use
    pub credential_id: String,
    /// Whether to automatically refresh credentials
    pub auto_refresh: bool,
    /// Refresh threshold in minutes (refresh when token expires in less than this)
    pub refresh_threshold_minutes: i64,
}

impl Default for CredentialConfig {
    fn default() -> Self {
        Self {
            credential_id: String::new(),
            auto_refresh: true,
            refresh_threshold_minutes: 5,
        }
    }
}

/// Credential rotation handler
#[cfg(feature = "credentials")]
pub struct CredentialRotationHandler {
    provider: Arc<ResourceCredentialProvider>,
    rotation_callback: Option<
        Arc<
            dyn Fn(
                    String,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = ResourceResult<()>> + Send>,
                > + Send
                + Sync,
        >,
    >,
}

#[cfg(feature = "credentials")]
impl CredentialRotationHandler {
    /// Create a new rotation handler
    pub fn new(provider: Arc<ResourceCredentialProvider>) -> Self {
        Self {
            provider,
            rotation_callback: None,
        }
    }

    /// Set rotation callback
    pub fn with_rotation_callback<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ResourceResult<()>> + Send + 'static,
    {
        self.rotation_callback = Some(Arc::new(move |token| Box::pin(callback(token))));
        self
    }

    /// Check if rotation is needed and perform if necessary
    pub async fn check_and_rotate(&self) -> ResourceResult<bool> {
        // Invalidate cache to force refresh
        self.provider.invalidate().await;

        // Get new token
        let new_token = self.provider.get_token_string().await?;

        // Call rotation callback if set
        if let Some(callback) = &self.rotation_callback {
            callback(new_token).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get the provider
    pub fn provider(&self) -> &Arc<ResourceCredentialProvider> {
        &self.provider
    }
}

/// Helper to extract connection string with credentials
#[cfg(feature = "credentials")]
pub async fn build_connection_string_with_credentials(
    base_url: &str,
    provider: &ResourceCredentialProvider,
) -> ResourceResult<String> {
    let token = provider.get_token_string().await?;

    // Replace placeholders in connection string
    let url = base_url
        .replace("{token}", &token)
        .replace("{password}", &token)
        .replace("{credential}", &token);

    Ok(url)
}

/// Credential rotation scheduler
#[cfg(feature = "credentials")]
pub struct CredentialRotationScheduler {
    handlers: Arc<parking_lot::RwLock<Vec<Arc<CredentialRotationHandler>>>>,
    rotation_interval: std::time::Duration,
    running: Arc<tokio::sync::RwLock<bool>>,
}

#[cfg(feature = "credentials")]
impl CredentialRotationScheduler {
    /// Create a new rotation scheduler
    pub fn new(rotation_interval: std::time::Duration) -> Self {
        Self {
            handlers: Arc::new(parking_lot::RwLock::new(Vec::new())),
            rotation_interval,
            running: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }

    /// Add a rotation handler
    pub fn add_handler(&self, handler: Arc<CredentialRotationHandler>) {
        let mut handlers = self.handlers.write();
        handlers.push(handler);
    }

    /// Start the rotation scheduler
    pub async fn start(&self) -> ResourceResult<()> {
        let mut running = self.running.write().await;
        if *running {
            return Err(ResourceError::internal(
                "rotation_scheduler",
                "Scheduler is already running",
            ));
        }

        *running = true;
        drop(running);

        let handlers = Arc::clone(&self.handlers);
        let interval = self.rotation_interval;
        let running_flag = Arc::clone(&self.running);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;

                // Check if still running
                {
                    let is_running = running_flag.read().await;
                    if !*is_running {
                        break;
                    }
                }

                // Perform rotation checks
                let handlers_list = handlers.read();
                for handler in handlers_list.iter() {
                    match handler.check_and_rotate().await {
                        Ok(rotated) => {
                            if rotated {
                                // Log rotation success
                                #[cfg(feature = "tracing")]
                                tracing::info!(
                                    credential_id = %handler.provider().credential_id(),
                                    "Credential rotated successfully"
                                );
                            }
                        }
                        Err(e) => {
                            // Log rotation failure
                            #[cfg(feature = "tracing")]
                            tracing::error!(
                                credential_id = %handler.provider().credential_id(),
                                error = %e,
                                "Credential rotation failed"
                            );
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the rotation scheduler
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// Get number of registered handlers
    pub fn handler_count(&self) -> usize {
        self.handlers.read().len()
    }
}

impl Default for CredentialRotationScheduler {
    fn default() -> Self {
        Self::new(std::time::Duration::from_secs(3600)) // Default: 1 hour
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_config_default() {
        let config = CredentialConfig::default();
        assert!(config.auto_refresh);
        assert_eq!(config.refresh_threshold_minutes, 5);
    }

    #[cfg(feature = "credentials")]
    #[tokio::test]
    async fn test_connection_string_builder() {
        // This would require a mock CredentialManager
        // Skipping for now - will add integration test
    }

    #[cfg(feature = "credentials")]
    #[tokio::test]
    async fn test_rotation_scheduler() {
        let scheduler = CredentialRotationScheduler::default();
        assert_eq!(scheduler.handler_count(), 0);

        scheduler.stop().await;
    }
}
