use super::{NegativeCache, RefreshPolicy};
use crate::core::{AccessToken, CredentialContext, CredentialError, CredentialId};
use crate::registry::CredentialRegistry;
use crate::traits::{DistributedLock, LockGuard, StateStore, StateVersion, TokenCache};
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Wrapper for any distributed lock implementation
pub(crate) struct AnyLock {
    inner: Arc<dyn AnyDistributedLock>,
}

impl AnyLock {
    /// Create from a concrete lock implementation
    pub fn new<L: DistributedLock + 'static>(lock: L) -> Self {
        Self {
            inner: Arc::new(lock),
        }
    }

    /// Acquire the lock
    pub async fn acquire(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Box<dyn LockGuard>, crate::traits::LockError> {
        self.inner.acquire_dyn(key, ttl).await
    }
}

/// Internal trait for type erasure
#[async_trait::async_trait]
trait AnyDistributedLock: Send + Sync {
    async fn acquire_dyn(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Box<dyn LockGuard>, crate::traits::LockError>;
}

#[async_trait::async_trait]
impl<L: DistributedLock> AnyDistributedLock for L
where
    <L as DistributedLock>::Guard: 'static,
{
    async fn acquire_dyn(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Box<dyn LockGuard>, crate::traits::LockError> {
        let guard = self.acquire(key, ttl).await?;
        Ok(Box::new(guard))
    }
}

/// Main credential manager
pub struct CredentialManager {
    store: Arc<dyn StateStore>,
    lock: AnyLock,
    cache: Option<Arc<dyn TokenCache>>,
    registry: Arc<CredentialRegistry>,
    policy: RefreshPolicy,
    negative_cache: Arc<DashMap<String, NegativeCache>>,
}

impl CredentialManager {
    pub(crate) fn new(
        store: Arc<dyn StateStore>,
        lock: AnyLock,
        cache: Option<Arc<dyn TokenCache>>,
        policy: RefreshPolicy,
        registry: Arc<CredentialRegistry>,
        negative_cache: Arc<DashMap<String, NegativeCache>>,
    ) -> Self {
        Self {
            store,
            lock,
            cache,
            registry,
            policy,
            negative_cache,
        }
    }
    /// Create new manager with builder
    pub fn builder() -> super::ManagerBuilder {
        super::ManagerBuilder::new()
    }

    /// Get token with automatic refresh
    pub async fn get_token(
        &self,
        credential_id: &CredentialId,
    ) -> Result<AccessToken, CredentialError> {
        // Check negative cache first
        if let Some(neg) = self.negative_cache.get(credential_id.as_str()) {
            if neg.until > SystemTime::now() {
                return Err(neg.error.clone());
            }
            // Expired negative cache entry
            self.negative_cache.remove(credential_id.as_str());
        }

        // Check cache
        if let Some(cache) = &self.cache {
            match cache.get(credential_id.as_str()).await {
                Ok(Some(token)) if !self.should_refresh(&token) => {
                    return Ok(token);
                }
                _ => {}
            }
        }

        // Acquire lock and refresh
        let lock_key = format!("credential:{credential_id}");

        let _guard = self
            .lock
            .acquire(&lock_key, Duration::from_secs(30))
            .await
            .map_err(|e| CredentialError::LockFailed {
                resource: format!("credential:{credential_id}"),
                reason: e.to_string(),
            })?;

        // Re-check cache inside lock
        if let Some(cache) = &self.cache
            && let Ok(Some(token)) = cache.get(credential_id.as_str()).await
            && !self.should_refresh(&token)
        {
            return Ok(token);
        }

        // Refresh the token
        self.refresh_internal(credential_id).await
    }

    /// Create a new credential
    pub async fn create_credential(
        &self,
        credential_type: &str,
        input: serde_json::Value,
    ) -> Result<CredentialId, CredentialError> {
        // Get factory from registry
        let factory = self.registry.get(credential_type).ok_or_else(|| {
            CredentialError::TypeNotRegistered {
                credential_type: credential_type.to_string(),
            }
        })?;

        // Create and initialize
        let mut ctx = CredentialContext::new();
        let (state, token) = factory.create_and_init(input, &mut ctx).await?;

        // Generate ID and save
        let credential_id = CredentialId::new();

        let state_json = serde_json::to_value(&state)
            .map_err(|e| CredentialError::SerializationFailed(e.to_string()))?;

        // Add metadata to state
        let mut state_with_meta = state_json;
        if let Some(obj) = state_with_meta.as_object_mut() {
            obj.insert("_type".to_string(), serde_json::json!(credential_type));
            obj.insert(
                "_created_at".to_string(),
                serde_json::json!(crate::core::unix_now()),
            );
        }

        self.store
            .save(credential_id.as_str(), StateVersion(0), &state_with_meta)
            .await?;

        // Cache initial token if provided
        if let Some(token) = token
            && let Some(cache) = &self.cache
        {
            let ttl = token.ttl().unwrap_or(Duration::from_secs(300));
            let _ = cache.put(credential_id.as_str(), &token, ttl).await;
        }

        Ok(credential_id)
    }

    /// Delete a credential
    pub async fn delete_credential(
        &self,
        credential_id: &CredentialId,
    ) -> Result<(), CredentialError> {
        // Clear cache
        if let Some(cache) = &self.cache {
            let _ = cache.del(credential_id.as_str()).await;
        }

        // Clear negative cache
        self.negative_cache.remove(credential_id.as_str());

        // Delete from store
        self.store.delete(credential_id.as_str()).await
    }

    /// List all credential IDs
    pub async fn list_credentials(&self) -> Result<Vec<String>, CredentialError> {
        self.store.list().await
    }

    /// Internal refresh implementation
    async fn refresh_internal(
        &self,
        credential_id: &CredentialId,
    ) -> Result<AccessToken, CredentialError> {
        let mut last_error = None;

        for attempt in 0..self.policy.max_retries {
            // Load state
            let (mut state_json, version) = self.store.load(credential_id.as_str()).await?;

            // Get credential type from state
            let credential_type = state_json
                .get("_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CredentialError::invalid_input("state", "missing _type field"))?
                .to_string();

            // Get factory
            let factory = self.registry.get(&credential_type).ok_or_else(|| {
                CredentialError::TypeNotRegistered {
                    credential_type: credential_type.clone(),
                }
            })?;

            // Clean metadata before refresh
            if let Some(obj) = state_json.as_object_mut() {
                obj.remove("_type");
                obj.remove("_created_at");
                obj.remove("_updated_at");
            }

            // Try refresh
            let mut ctx = CredentialContext::new();
            match factory.refresh(state_json.clone(), &mut ctx).await {
                Ok((new_state, token)) => {
                    // Save updated state
                    let mut new_state_json = serde_json::to_value(&new_state)
                        .map_err(|e| CredentialError::SerializationFailed(e.to_string()))?;

                    // Add metadata back
                    if let Some(obj) = new_state_json.as_object_mut() {
                        obj.insert("_type".to_string(), serde_json::json!(credential_type));
                        obj.insert(
                            "_updated_at".to_string(),
                            serde_json::json!(crate::core::unix_now()),
                        );
                    }

                    // Save new state with CAS
                    let save_result = self
                        .store
                        .save(credential_id.as_str(), version, &new_state_json)
                        .await;

                    match save_result {
                        Err(CredentialError::CasConflict) => {
                            // Retry with fresh state
                            continue;
                        }
                        Err(e) => return Err(e),
                        Ok(_) => {
                            // Success: cache token and clear negative cache
                            self.cache_token_if_available(credential_id.as_str(), &token)
                                .await;
                            self.negative_cache.remove(credential_id.as_str());
                            return Ok(token);
                        }
                    }
                }
                Err(e) if e.is_retryable() && attempt < self.policy.max_retries - 1 => {
                    last_error = Some(e);

                    // Calculate backoff and sleep
                    let backoff = self.calculate_backoff(attempt);
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    // Add to negative cache
                    self.negative_cache.insert(
                        credential_id.to_string(),
                        NegativeCache {
                            until: SystemTime::now() + self.policy.negative_cache_ttl,
                            error: e.clone(),
                        },
                    );
                    return Err(e);
                }
            }
        }

        let error = last_error
            .unwrap_or_else(|| CredentialError::internal("Refresh failed after max retries"));
        self.negative_cache.insert(
            credential_id.to_string(),
            NegativeCache {
                until: SystemTime::now() + self.policy.negative_cache_ttl,
                error: error.clone(),
            },
        );
        Err(error)
    }

    async fn cache_token_if_available(&self, credential_id: &str, token: &AccessToken) {
        if let Some(cache) = &self.cache {
            let ttl = token.ttl().unwrap_or(Duration::from_secs(300));
            let _ = cache.put(credential_id, token, ttl).await;
        }
    }

    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let base = self.policy.backoff_base.as_millis() as u64;
        let factor = self.policy.backoff_factor;
        let max = self.policy.backoff_max.as_millis() as u64;

        let backoff_ms = (base as f64 * factor.powi(attempt as i32)) as u64;
        let clamped = backoff_ms.min(max);

        // Add jitter to avoid thundering herd
        use rand::Rng;
        let jitter = rand::thread_rng().gen_range(0..clamped / 4);
        Duration::from_millis(clamped + jitter)
    }

    fn should_refresh(&self, token: &AccessToken) -> bool {
        if let Some(expires_at) = token.expires_at {
            let now = SystemTime::now();
            let ttl = expires_at
                .duration_since(token.issued_at)
                .unwrap_or_default();
            let age = now.duration_since(token.issued_at).unwrap_or_default();

            age.as_secs_f64() / ttl.as_secs_f64() >= f64::from(self.policy.threshold)
                || expires_at <= now + self.policy.skew
        } else if let Some(max_age) = self.policy.max_age {
            // For eternal tokens, check max age
            let age = SystemTime::now()
                .duration_since(token.issued_at)
                .unwrap_or_default();
            age >= max_age
        } else {
            false
        }
    }

    /// Get reference to the registry (for testing)
    pub fn registry(&self) -> &Arc<CredentialRegistry> {
        &self.registry
    }

    /// Get reference to the cache (for testing)
    pub fn cache(&self) -> Option<&Arc<dyn TokenCache>> {
        self.cache.as_ref()
    }
}
