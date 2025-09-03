use crate::core::*;
use crate::manager::*;
use crate::traits::*;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

/// Mock state store with configurable behavior
pub struct MockStateStore {
    data: Arc<DashMap<String, (serde_json::Value, StateVersion)>>,
    fail_on_load: Arc<AtomicBool>,
    fail_on_save: Arc<AtomicBool>,
    load_count: Arc<AtomicU32>,
    save_count: Arc<AtomicU32>,
    delay: Option<Duration>,
}

impl MockStateStore {
    /// Create new mock store
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            fail_on_load: Arc::new(AtomicBool::new(false)),
            fail_on_save: Arc::new(AtomicBool::new(false)),
            load_count: Arc::new(AtomicU32::new(0)),
            save_count: Arc::new(AtomicU32::new(0)),
            delay: None,
        }
    }

    /// Make next load fail
    pub fn fail_next_load(&self) {
        self.fail_on_load.store(true, Ordering::SeqCst);
    }

    /// Make next save fail
    pub fn fail_next_save(&self) {
        self.fail_on_save.store(true, Ordering::SeqCst);
    }

    /// Set artificial delay for operations
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Get number of load operations
    pub fn load_count(&self) -> u32 {
        self.load_count.load(Ordering::SeqCst)
    }

    /// Get number of save operations
    pub fn save_count(&self) -> u32 {
        self.save_count.load(Ordering::SeqCst)
    }

    /// Insert test data directly
    pub fn insert_test_data(&self, id: &str, state: serde_json::Value) {
        self.data.insert(id.to_string(), (state, StateVersion(1)));
    }
}

#[async_trait]
impl StateStore for MockStateStore {
    async fn load(&self, id: &str) -> Result<(serde_json::Value, StateVersion)> {
        self.load_count.fetch_add(1, Ordering::SeqCst);

        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }

        if self.fail_on_load.swap(false, Ordering::SeqCst) {
            return Err(CredentialError::storage_failed("load", "mock failure"));
        }

        self.data
            .get(id)
            .map(|entry| entry.clone())
            .ok_or_else(|| CredentialError::not_found(id))
    }

    async fn save(
        &self,
        id: &str,
        version: StateVersion,
        state: &serde_json::Value,
    ) -> Result<StateVersion> {
        self.save_count.fetch_add(1, Ordering::SeqCst);

        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }

        if self.fail_on_save.swap(false, Ordering::SeqCst) {
            return Err(CredentialError::storage_failed("save", "mock failure"));
        }

        let new_version = StateVersion(version.0 + 1);

        if let Some(mut entry) = self.data.get_mut(id) {
            if entry.1.0 != version.0 {
                return Err(CredentialError::CasConflict);
            }
            *entry = (state.clone(), new_version);
        } else if version.0 == 0 {
            self.data
                .insert(id.to_string(), (state.clone(), new_version));
        } else {
            return Err(CredentialError::CasConflict);
        }

        Ok(new_version)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.data.remove(id);
        Ok(())
    }

    async fn exists(&self, id: &str) -> Result<bool> {
        Ok(self.data.contains_key(id))
    }

    async fn list(&self) -> Result<Vec<String>> {
        Ok(self.data.iter().map(|e| e.key().clone()).collect())
    }
}

/// Mock distributed lock with configurable behavior
pub struct MockLock {
    locks: Arc<DashMap<String, SystemTime>>,
    fail_on_acquire: Arc<AtomicBool>,
    acquire_count: Arc<AtomicU32>,
    contention_duration: Arc<RwLock<Option<Duration>>>,
}

impl MockLock {
    /// Create new mock lock
    pub fn new() -> Self {
        Self {
            locks: Arc::new(DashMap::new()),
            fail_on_acquire: Arc::new(AtomicBool::new(false)),
            acquire_count: Arc::new(AtomicU32::new(0)),
            contention_duration: Arc::new(RwLock::new(None)),
        }
    }

    /// Make next acquire fail
    pub fn fail_next_acquire(&self) {
        self.fail_on_acquire.store(true, Ordering::SeqCst);
    }

    /// Simulate lock contention for duration
    pub async fn simulate_contention(&self, duration: Duration) {
        *self.contention_duration.write().await = Some(duration);
    }

    /// Get number of acquire attempts
    pub fn acquire_count(&self) -> u32 {
        self.acquire_count.load(Ordering::SeqCst)
    }
}

pub struct MockLockGuard {
    key: String,
    locks: Arc<DashMap<String, SystemTime>>,
}

#[async_trait]
impl LockGuard for MockLockGuard {
    async fn release(self: Box<Self>) -> std::result::Result<(), LockError> {
        self.locks.remove(&self.key);
        Ok(())
    }
}

impl Drop for MockLockGuard {
    fn drop(&mut self) {
        self.locks.remove(&self.key);
    }
}

#[async_trait]
impl DistributedLock for MockLock {
    type Guard = MockLockGuard;

    async fn acquire(
        &self,
        key: &str,
        ttl: Duration,
    ) -> std::result::Result<Self::Guard, LockError> {
        self.acquire_count.fetch_add(1, Ordering::SeqCst);

        if self.fail_on_acquire.swap(false, Ordering::SeqCst) {
            return Err(LockError::Backend("mock failure".into()));
        }

        // Check for simulated contention
        if let Some(duration) = *self.contention_duration.read().await {
            tokio::time::sleep(duration).await;
            *self.contention_duration.write().await = None;
        }

        let deadline = SystemTime::now() + ttl;
        self.locks.insert(key.to_string(), deadline);

        Ok(MockLockGuard {
            key: key.to_string(),
            locks: self.locks.clone(),
        })
    }

    async fn try_acquire(
        &self,
        key: &str,
        ttl: Duration,
    ) -> std::result::Result<Option<Self::Guard>, LockError> {
        if let Some(existing) = self.locks.get(key) {
            if *existing > SystemTime::now() {
                return Ok(None);
            }
        }

        let deadline = SystemTime::now() + ttl;
        self.locks.insert(key.to_string(), deadline);

        Ok(Some(MockLockGuard {
            key: key.to_string(),
            locks: self.locks.clone(),
        }))
    }
}

/// Mock token cache with metrics
pub struct MockTokenCache {
    cache: Arc<DashMap<String, (AccessToken, SystemTime)>>,
    hit_count: Arc<AtomicU32>,
    miss_count: Arc<AtomicU32>,
    put_count: Arc<AtomicU32>,
}

impl MockTokenCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            hit_count: Arc::new(AtomicU32::new(0)),
            miss_count: Arc::new(AtomicU32::new(0)),
            put_count: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn hit_rate(&self) -> f32 {
        let hits = self.hit_count.load(Ordering::SeqCst) as f32;
        let total = hits + self.miss_count.load(Ordering::SeqCst) as f32;
        if total > 0.0 { hits / total } else { 0.0 }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hit_count.load(Ordering::SeqCst),
            misses: self.miss_count.load(Ordering::SeqCst),
            puts: self.put_count.load(Ordering::SeqCst),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hits: u32,
    pub misses: u32,
    pub puts: u32,
}

#[async_trait]
impl TokenCache for MockTokenCache {
    async fn get(&self, key: &str) -> Result<Option<AccessToken>> {
        if let Some(entry) = self.cache.get(key) {
            let (token, expires_at) = entry.value();
            if *expires_at > SystemTime::now() {
                self.hit_count.fetch_add(1, Ordering::SeqCst);
                return Ok(Some(token.clone()));
            }
            drop(entry);
            self.cache.remove(key);
        }
        self.miss_count.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }

    async fn put(&self, key: &str, token: &AccessToken, ttl: Duration) -> Result<()> {
        self.put_count.fetch_add(1, Ordering::SeqCst);
        let expires_at = SystemTime::now() + ttl;
        self.cache
            .insert(key.to_string(), (token.clone(), expires_at));
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<()> {
        self.cache.remove(key);
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.cache.clear();
        Ok(())
    }
}
