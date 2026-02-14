//! TTL (Time To Live) cache eviction policy
//!
//! This module provides an implementation of the TTL cache eviction policy,
//! which evicts entries that have expired based on their time-to-live.

#![allow(clippy::excessive_nesting)]

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use super::{EvictionEntry, EvictionPolicy, VictimSelector};
use crate::cache::compute::{CacheEntry, CacheKey};

/// TTL (Time To Live) cache eviction policy
pub struct TtlPolicy<K>
where
    K: CacheKey,
{
    /// Default TTL for entries
    default_ttl: Duration,
    /// Custom TTLs for specific keys
    custom_ttls: HashMap<K, Duration>,
    /// Insertion times for entries
    insertion_times: HashMap<K, Instant>,
}

impl<K> TtlPolicy<K>
where
    K: CacheKey,
{
    /// Create a new TTL policy with the given default TTL
    #[must_use]
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            default_ttl,
            custom_ttls: HashMap::new(),
            insertion_times: HashMap::new(),
        }
    }

    /// Set a custom TTL for a specific key
    pub fn set_ttl(&mut self, key: K, ttl: Duration) {
        self.custom_ttls.insert(key, ttl);
    }

    /// Get the TTL for a key
    pub fn get_ttl(&self, key: &K) -> Duration {
        self.custom_ttls
            .get(key)
            .copied()
            .unwrap_or(self.default_ttl)
    }

    /// Clear all cached data
    pub fn clear(&mut self) {
        self.custom_ttls.clear();
        self.insertion_times.clear();
    }

    /// Check if an entry has expired
    #[allow(dead_code)] // planned for proactive expiration checks
    fn is_expired(&self, key: &K) -> bool {
        if let Some(insertion_time) = self.insertion_times.get(key) {
            let ttl = self.get_ttl(key);
            insertion_time.elapsed() > ttl
        } else {
            false
        }
    }

    /// Record insertion time for a key
    #[allow(dead_code)] // planned for proactive expiration checks
    fn record_insertion_time(&mut self, key: &K) {
        self.insertion_times.insert(key.clone(), Instant::now());
    }
}

/// Реализация `VictimSelector` для TTL policy
impl<K, V> VictimSelector<K, V> for TtlPolicy<K>
where
    K: CacheKey + Send + Sync,
{
    fn select_victim(&self, entries: &[EvictionEntry<'_, K, V>]) -> Option<K> {
        if entries.is_empty() {
            return None;
        }

        let now = Instant::now();

        // Сначала ищем просроченные записи
        let mut oldest_key = None;
        let mut oldest_time = now;

        for &(key, _) in entries {
            if let Some(insert_time) = self.insertion_times.get(key) {
                let ttl = self.custom_ttls.get(key).unwrap_or(&self.default_ttl);

                // Проверяем, истек ли срок действия
                if now.duration_since(*insert_time) >= *ttl {
                    return Some(key.clone());
                }

                // Запоминаем самую старую запись для возможного использования позже
                if *insert_time < oldest_time {
                    oldest_time = *insert_time;
                    oldest_key = Some(key.clone());
                }
            }
        }

        // Если нашли хотя бы одну запись с временем вставки, возвращаем самую старую
        if let Some(key) = oldest_key {
            return Some(key);
        }

        // Если нет просроченных записей, возвращаем None
        None
    }
}

impl<K, V> EvictionPolicy<K, V> for TtlPolicy<K>
where
    K: CacheKey + Send + Sync,
{
    fn record_access(&mut self, key: &K) {
        // TTL policy doesn't change based on access
        let _ = key;
    }

    fn record_insertion(&mut self, key: &K, _entry: &CacheEntry<V>) {
        // Запоминаем время вставки
        self.insertion_times.insert(key.clone(), Instant::now());
    }

    fn record_removal(&mut self, key: &K) {
        // Удаляем информацию о времени вставки
        self.insertion_times.remove(key);
    }

    fn as_victim_selector(&self) -> &dyn VictimSelector<K, V> {
        self
    }

    fn name(&self) -> &'static str {
        "TTL"
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn test_ttl_policy() {
        let mut policy: TtlPolicy<String> = TtlPolicy::new(Duration::from_millis(100));

        // Insert some keys
        let entry = CacheEntry::new(42);
        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);

        // Set a custom TTL for key2
        policy.set_ttl("key2".to_string(), Duration::from_millis(200));

        // Wait for key1 to expire
        thread::sleep(Duration::from_millis(150));

        // The expired key should be key1
        let key1_str = "key1".to_string();
        let key2_str = "key2".to_string();
        let entries: Vec<(&String, &CacheEntry<i32>)> =
            vec![(&key1_str, &entry), (&key2_str, &entry)];
        let victim = policy.as_victim_selector().select_victim(&entries);

        assert_eq!(victim, Some("key1".to_string()));

        // Wait for key2 to expire as well
        thread::sleep(Duration::from_millis(100));

        // Now both keys are expired, but key1 is older
        let key1_str2 = "key1".to_string();
        let key2_str2 = "key2".to_string();
        let entries: Vec<(&String, &CacheEntry<i32>)> =
            vec![(&key1_str2, &entry), (&key2_str2, &entry)];
        let victim = policy.as_victim_selector().select_victim(&entries);

        assert_eq!(victim, Some("key1".to_string()));

        // Remove key1
        let key1 = "key1".to_string();
        <TtlPolicy<String> as EvictionPolicy<String, i32>>::record_removal(&mut policy, &key1);

        // Now only key2 is left and it's expired
        let key2_only = "key2".to_string();
        let entries: Vec<(&String, &CacheEntry<i32>)> = vec![(&key2_only, &entry)];
        let victim = policy.as_victim_selector().select_victim(&entries);

        assert_eq!(victim, Some("key2".to_string()));
    }

    #[test]
    fn test_ttl_fallback() {
        let mut policy: TtlPolicy<String> = TtlPolicy::new(Duration::from_secs(1));

        // Insert some keys
        let entry = CacheEntry::new(42);
        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);

        // Access key1 to make it more recently used
        let key1 = "key1".to_string();
        <TtlPolicy<String> as EvictionPolicy<String, i32>>::record_access(&mut policy, &key1);

        // No keys have expired, so TTL policy falls back to oldest insertion
        // key1 was inserted first, so it should be selected as victim
        let key1_fb = "key1".to_string();
        let key2_fb = "key2".to_string();
        let entries: Vec<(&String, &CacheEntry<i32>)> =
            vec![(&key1_fb, &entry), (&key2_fb, &entry)];
        let victim = policy.as_victim_selector().select_victim(&entries);

        // When no keys are expired, TTL policy returns the oldest entry (key1)
        assert_eq!(victim, Some("key1".to_string()));
    }

    #[test]
    fn test_ttl_clear() {
        let mut policy = TtlPolicy::<String>::new(Duration::from_millis(100));

        // Insert some keys
        let entry = CacheEntry::new(42);
        policy.record_insertion(&"key1".to_string(), &entry);
        policy.record_insertion(&"key2".to_string(), &entry);

        // Set a custom TTL for key2
        policy.set_ttl("key2".to_string(), Duration::from_millis(200));

        // Wait for key1 to expire
        thread::sleep(Duration::from_millis(150));

        // Before clear: key1 should be expired
        let key1_before = "key1".to_string();
        let key2_before = "key2".to_string();
        let entries_before: Vec<(&String, &CacheEntry<i32>)> =
            vec![(&key1_before, &entry), (&key2_before, &entry)];
        let victim_before = policy.as_victim_selector().select_victim(&entries_before);
        assert_eq!(victim_before, Some("key1".to_string()));

        // Clear the policy - this resets insertion times and custom TTLs
        policy.clear();

        // After clear, all TTL tracking is reset
        // Need to re-insert to test properly
        policy.record_insertion(&"key3".to_string(), &entry);
        let key3 = "key3".to_string();
        let entries_after: Vec<(&String, &CacheEntry<i32>)> = vec![(&key3, &entry)];
        let victim_after = policy.as_victim_selector().select_victim(&entries_after);
        assert_eq!(victim_after, Some("key3".to_string()));
    }
}
