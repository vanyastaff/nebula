use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Token refresh policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshPolicy {
    /// Refresh when this percentage of TTL has elapsed (0.0 - 1.0)
    pub threshold: f32,

    /// Safety margin before expiry
    pub skew: Duration,

    /// Maximum age for eternal tokens
    pub max_age: Option<Duration>,

    /// Maximum number of refresh retries
    pub max_retries: u32,

    /// Base backoff duration
    pub backoff_base: Duration,

    /// Backoff multiplication factor
    pub backoff_factor: f64,

    /// Maximum backoff duration
    pub backoff_max: Duration,

    /// Negative cache TTL
    pub negative_cache_ttl: Duration,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self {
            threshold: 0.8,
            skew: Duration::from_secs(45),
            max_age: Some(Duration::from_secs(3600)),
            max_retries: 3,
            backoff_base: Duration::from_millis(100),
            backoff_factor: 2.0,
            backoff_max: Duration::from_secs(10),
            negative_cache_ttl: Duration::from_secs(60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_refresh_policy_default_values() {
        let policy = RefreshPolicy::default();

        assert_eq!(policy.threshold, 0.8);
        assert_eq!(policy.skew, Duration::from_secs(45));
        assert_eq!(policy.max_age, Some(Duration::from_secs(3600)));
        assert_eq!(policy.max_retries, 3);
        assert_eq!(policy.backoff_base, Duration::from_millis(100));
        assert_eq!(policy.backoff_factor, 2.0);
        assert_eq!(policy.backoff_max, Duration::from_secs(10));
        assert_eq!(policy.negative_cache_ttl, Duration::from_secs(60));
    }

    #[test]
    fn test_refresh_policy_custom_threshold() {
        let policy = RefreshPolicy {
            threshold: 0.5,
            ..Default::default()
        };
        assert_eq!(policy.threshold, 0.5);
    }

    #[test]
    fn test_refresh_policy_custom_skew() {
        let policy = RefreshPolicy {
            skew: Duration::from_secs(120),
            ..Default::default()
        };
        assert_eq!(policy.skew, Duration::from_secs(120));
    }

    #[test]
    fn test_refresh_policy_no_max_age() {
        let policy = RefreshPolicy {
            max_age: None,
            ..Default::default()
        };
        assert!(policy.max_age.is_none());
    }

    #[test]
    fn test_refresh_policy_max_retries() {
        let policy = RefreshPolicy {
            max_retries: 5,
            ..Default::default()
        };
        assert_eq!(policy.max_retries, 5);
    }

    #[test]
    fn test_refresh_policy_backoff_configuration() {
        let policy = RefreshPolicy {
            backoff_base: Duration::from_millis(200),
            backoff_factor: 3.0,
            backoff_max: Duration::from_secs(30),
            ..Default::default()
        };

        assert_eq!(policy.backoff_base, Duration::from_millis(200));
        assert_eq!(policy.backoff_factor, 3.0);
        assert_eq!(policy.backoff_max, Duration::from_secs(30));
    }

    #[test]
    fn test_refresh_policy_negative_cache_ttl() {
        let policy = RefreshPolicy {
            negative_cache_ttl: Duration::from_secs(300),
            ..Default::default()
        };
        assert_eq!(policy.negative_cache_ttl, Duration::from_secs(300));
    }

    #[test]
    fn test_refresh_policy_serialization() {
        let policy = RefreshPolicy::default();
        let json = serde_json::to_string(&policy).expect("serialization should work");
        let deserialized: RefreshPolicy =
            serde_json::from_str(&json).expect("deserialization should work");

        assert_eq!(policy.threshold, deserialized.threshold);
        assert_eq!(policy.max_retries, deserialized.max_retries);
    }

    #[test]
    fn test_refresh_policy_clone() {
        let original = RefreshPolicy::default();
        let cloned = original.clone();

        assert_eq!(original.threshold, cloned.threshold);
        assert_eq!(original.skew, cloned.skew);
        assert_eq!(original.max_retries, cloned.max_retries);
    }
}
