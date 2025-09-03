//! Utility functions

use std::sync::OnceLock;

/// Generate a unique request ID
pub fn generate_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static PREFIX: OnceLock<String> = OnceLock::new();

    let prefix = PREFIX.get_or_init(|| format!("{:x}", std::process::id()));

    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    format!("{}-{:x}-{:x}", prefix, timestamp, count)
}

/// Check if running in a container
pub fn is_containerized() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
        || std::env::var("CONTAINER").is_ok()
}

/// Detect environment from common indicators
pub fn detect_environment() -> String {
    if let Ok(env) = std::env::var("NEBULA_ENV") {
        return env;
    }

    if let Ok(env) = std::env::var("RUST_ENV") {
        return env;
    }

    if let Ok(env) = std::env::var("ENVIRONMENT") {
        return env;
    }

    if cfg!(debug_assertions) {
        "development".to_string()
    } else if is_containerized() {
        "production".to_string()
    } else {
        "local".to_string()
    }
}

/// Format duration in human-readable format
pub fn format_duration(duration: std::time::Duration) -> String {
    let total_ms = duration.as_millis();

    if total_ms < 1000 {
        format!("{}ms", total_ms)
    } else if total_ms < 60_000 {
        format!("{:.2}s", duration.as_secs_f64())
    } else {
        let mins = total_ms / 60_000;
        let secs = (total_ms % 60_000) / 1000;
        format!("{}m {}s", mins, secs)
    }
}

/// Truncate string to max length with ellipsis
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_generation() {
        let id1 = generate_request_id();
        let id2 = generate_request_id();

        assert_ne!(id1, id2);
        assert!(id1.contains('-'));
        assert!(id2.contains('-'));
    }

    #[test]
    fn test_format_duration() {
        use std::time::Duration;

        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.50s");
        assert_eq!(format_duration(Duration::from_millis(65_000)), "1m 5s");
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 8), "hello...");
        assert_eq!(truncate_string("hello", 3), "...");
    }
}
