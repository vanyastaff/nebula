//! Constants and configuration values for Nebula
//! 
//! This module defines system-wide constants that are used
//! throughout the Nebula workflow engine.

use std::time::Duration;

/// System-wide constants
pub const SYSTEM_NAME: &str = "Nebula";
pub const SYSTEM_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SYSTEM_DESCRIPTION: &str = "High-performance workflow engine";

/// Default timeouts
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_DATABASE_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_GRPC_TIMEOUT: Duration = Duration::from_secs(15);

/// Default retry settings
pub const DEFAULT_MAX_RETRIES: u32 = 3;
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(1);
pub const DEFAULT_MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Default circuit breaker settings
pub const DEFAULT_CIRCUIT_BREAKER_FAILURE_THRESHOLD: u32 = 5;
pub const DEFAULT_CIRCUIT_BREAKER_RESET_TIMEOUT: Duration = Duration::from_secs(60);

/// Default bulkhead settings
pub const DEFAULT_BULKHEAD_MAX_CONCURRENT: u32 = 10;
pub const DEFAULT_BULKHEAD_MAX_QUEUE_SIZE: u32 = 100;

/// Default memory and cache settings
pub const DEFAULT_MAX_MEMORY_MB: usize = 1024;
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);
pub const DEFAULT_MAX_CACHE_SIZE: usize = 10000;

/// Default workflow settings
pub const DEFAULT_MAX_WORKFLOW_NODES: usize = 1000;
pub const DEFAULT_MAX_WORKFLOW_DEPTH: usize = 50;
pub const DEFAULT_MAX_EXECUTION_TIME: Duration = Duration::from_secs(3600);

/// Default node settings
pub const DEFAULT_MAX_NODE_INPUT_SIZE: usize = 1024 * 1024; // 1MB
pub const DEFAULT_MAX_NODE_OUTPUT_SIZE: usize = 1024 * 1024; // 1MB
pub const DEFAULT_MAX_NODE_EXECUTION_TIME: Duration = Duration::from_secs(300);

/// Default action settings
pub const DEFAULT_MAX_ACTION_PARAMETERS: usize = 100;
pub const DEFAULT_MAX_ACTION_RESULT_SIZE: usize = 10 * 1024 * 1024; // 10MB

/// Default expression settings
pub const DEFAULT_MAX_EXPRESSION_LENGTH: usize = 10000;
pub const DEFAULT_MAX_EXPRESSION_DEPTH: usize = 100;
pub const DEFAULT_MAX_EXPRESSION_EXECUTION_TIME: Duration = Duration::from_secs(10);

/// Default event settings
pub const DEFAULT_MAX_EVENT_QUEUE_SIZE: usize = 10000;
pub const DEFAULT_EVENT_TTL: Duration = Duration::from_secs(3600);

/// Default storage settings
pub const DEFAULT_MAX_STORAGE_KEY_LENGTH: usize = 255;
pub const DEFAULT_MAX_STORAGE_VALUE_SIZE: usize = 100 * 1024 * 1024; // 100MB
pub const DEFAULT_STORAGE_BATCH_SIZE: usize = 1000;

/// Default cluster settings
pub const DEFAULT_CLUSTER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_CLUSTER_ELECTION_TIMEOUT: Duration = Duration::from_secs(100);
pub const DEFAULT_CLUSTER_MAX_NODES: usize = 100;

/// Default tenant settings
pub const DEFAULT_MAX_TENANTS: usize = 1000;
pub const DEFAULT_MAX_WORKFLOWS_PER_TENANT: usize = 10000;
pub const DEFAULT_MAX_EXECUTIONS_PER_TENANT: usize = 100000;

/// Default API settings
pub const DEFAULT_API_MAX_REQUEST_SIZE: usize = 10 * 1024 * 1024; // 10MB
pub const DEFAULT_API_RATE_LIMIT: u32 = 1000; // requests per minute
pub const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(30);

/// Default logging settings
pub const DEFAULT_LOG_LEVEL: &str = "info";
pub const DEFAULT_LOG_MAX_FILES: usize = 10;
pub const DEFAULT_LOG_MAX_FILE_SIZE: usize = 100 * 1024 * 1024; // 100MB

/// Default metrics settings
pub const DEFAULT_METRICS_COLLECTION_INTERVAL: Duration = Duration::from_secs(15);
pub const DEFAULT_METRICS_RETENTION_PERIOD: Duration = Duration::from_secs(86400 * 7); // 7 days

/// Default security settings
pub const DEFAULT_MAX_PASSWORD_LENGTH: usize = 128;
pub const DEFAULT_MIN_PASSWORD_LENGTH: usize = 8;
pub const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(3600);
pub const DEFAULT_MAX_LOGIN_ATTEMPTS: u32 = 5;

/// Default validation settings
pub const DEFAULT_MAX_STRING_LENGTH: usize = 10000;
pub const DEFAULT_MAX_ARRAY_SIZE: usize = 10000;
pub const DEFAULT_MAX_OBJECT_PROPERTIES: usize = 1000;

/// Default serialization settings
pub const DEFAULT_MAX_SERIALIZATION_SIZE: usize = 100 * 1024 * 1024; // 100MB
pub const DEFAULT_SERIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Default testing settings
pub const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_TEST_MAX_ITERATIONS: usize = 1000;

/// Environment variable names
pub mod env {
    pub const NEBULA_ENV: &str = "NEBULA_ENV";
    pub const NEBULA_LOG_LEVEL: &str = "NEBULA_LOG_LEVEL";
    pub const NEBULA_CONFIG_PATH: &str = "NEBULA_CONFIG_PATH";
    pub const NEBULA_DATABASE_URL: &str = "NEBULA_DATABASE_URL";
    pub const NEBULA_REDIS_URL: &str = "NEBULA_REDIS_URL";
    pub const NEBULA_CLUSTER_NODES: &str = "NEBULA_CLUSTER_NODES";
    pub const NEBULA_TENANT_ID: &str = "NEBULA_TENANT_ID";
    pub const NEBULA_USER_ID: &str = "NEBULA_USER_ID";
}

/// Configuration file paths
pub mod paths {
    pub const DEFAULT_CONFIG_DIR: &str = "config";
    pub const DEFAULT_CONFIG_FILE: &str = "nebula.toml";
    pub const DEFAULT_LOG_DIR: &str = "logs";
    pub const DEFAULT_DATA_DIR: &str = "data";
    pub const DEFAULT_TEMP_DIR: &str = "temp";
    pub const DEFAULT_CACHE_DIR: &str = "cache";
}

/// HTTP status codes
pub mod http {
    pub const HTTP_OK: u16 = 200;
    pub const HTTP_CREATED: u16 = 201;
    pub const HTTP_ACCEPTED: u16 = 202;
    pub const HTTP_NO_CONTENT: u16 = 204;
    pub const HTTP_BAD_REQUEST: u16 = 400;
    pub const HTTP_UNAUTHORIZED: u16 = 401;
    pub const HTTP_FORBIDDEN: u16 = 403;
    pub const HTTP_NOT_FOUND: u16 = 404;
    pub const HTTP_METHOD_NOT_ALLOWED: u16 = 405;
    pub const HTTP_CONFLICT: u16 = 409;
    pub const HTTP_TOO_MANY_REQUESTS: u16 = 429;
    pub const HTTP_INTERNAL_SERVER_ERROR: u16 = 500;
    pub const HTTP_SERVICE_UNAVAILABLE: u16 = 503;
}

/// Error codes
pub mod error_codes {
    pub const VALIDATION_ERROR: &str = "VALIDATION_ERROR";
    pub const AUTHENTICATION_ERROR: &str = "AUTHENTICATION_ERROR";
    pub const AUTHORIZATION_ERROR: &str = "AUTHORIZATION_ERROR";
    pub const NOT_FOUND_ERROR: &str = "NOT_FOUND_ERROR";
    pub const CONFLICT_ERROR: &str = "CONFLICT_ERROR";
    pub const TIMEOUT_ERROR: &str = "TIMEOUT_ERROR";
    pub const RATE_LIMIT_ERROR: &str = "RATE_LIMIT_ERROR";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
    pub const SERVICE_UNAVAILABLE_ERROR: &str = "SERVICE_UNAVAILABLE_ERROR";
}

/// Feature flags
pub mod features {
    pub const ENABLE_METRICS: &str = "ENABLE_METRICS";
    pub const ENABLE_TRACING: &str = "ENABLE_TRACING";
    pub const ENABLE_PROFILING: &str = "ENABLE_PROFILING";
    pub const ENABLE_DEBUG: &str = "ENABLE_DEBUG";
    pub const ENABLE_TESTING: &str = "ENABLE_TESTING";
}

/// Magic numbers and identifiers
pub mod magic {
    pub const NEBULA_MAGIC: &[u8] = b"NEBULA";
    pub const NEBULA_MAGIC_LENGTH: usize = 6;
    pub const NEBULA_VERSION_MAJOR: u8 = 0;
    pub const NEBULA_VERSION_MINOR: u8 = 1;
    pub const NEBULA_VERSION_PATCH: u8 = 0;
}

/// Performance thresholds
pub mod performance {
    use super::*;
    
    pub const MAX_WORKFLOW_STARTUP_TIME: Duration = Duration::from_millis(100);
    pub const MAX_NODE_EXECUTION_TIME: Duration = Duration::from_millis(10);
    pub const MAX_EXPRESSION_EVALUATION_TIME: Duration = Duration::from_millis(1);
    pub const MAX_SERIALIZATION_TIME: Duration = Duration::from_millis(5);
    pub const MAX_DESERIALIZATION_TIME: Duration = Duration::from_millis(5);
}

/// Security constants
pub mod security {
    use super::*;
    
    pub const MIN_PASSWORD_ENTROPY: f64 = 3.0;
    pub const MAX_SESSION_DURATION: Duration = Duration::from_secs(86400 * 30); // 30 days
    pub const MIN_SESSION_DURATION: Duration = Duration::from_secs(300); // 5 minutes
    pub const MAX_API_KEY_LENGTH: usize = 64;
    pub const MIN_API_KEY_LENGTH: usize = 16;
}

/// Validation patterns
pub mod patterns {
    pub const IDENTIFIER_PATTERN: &str = r"^[a-zA-Z_][a-zA-Z0-9_-]*$";
    pub const EMAIL_PATTERN: &str = r"^[^@]+@[^@]+\.[^@]+$";
    pub const URL_PATTERN: &str = r"^https?://[^\s/$.?#].[^\s]*$";
    pub const VERSION_PATTERN: &str = r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$";
}

/// Default limits
pub mod limits {
    pub const MAX_WORKFLOW_NAME_LENGTH: usize = 255;
    pub const MAX_WORKFLOW_DESCRIPTION_LENGTH: usize = 1000;
    pub const MAX_NODE_NAME_LENGTH: usize = 255;
    pub const MAX_ACTION_NAME_LENGTH: usize = 255;
    pub const MAX_PARAMETER_NAME_LENGTH: usize = 255;
    pub const MAX_PARAMETER_VALUE_LENGTH: usize = 10000;
    pub const MAX_TAG_LENGTH: usize = 100;
    pub const MAX_TAGS_PER_ENTITY: usize = 50;
    pub const MAX_METADATA_KEYS: usize = 100;
    pub const MAX_METADATA_VALUE_LENGTH: usize = 1000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_are_defined() {
        assert!(!SYSTEM_NAME.is_empty());
        assert!(!SYSTEM_VERSION.is_empty());
        assert!(!SYSTEM_DESCRIPTION.is_empty());
    }

    #[test]
    fn test_timeouts_are_reasonable() {
        assert!(DEFAULT_TIMEOUT > Duration::from_secs(0));
        assert!(DEFAULT_DATABASE_TIMEOUT < DEFAULT_TIMEOUT);
        assert!(DEFAULT_HTTP_TIMEOUT < DEFAULT_TIMEOUT);
    }

    #[test]
    fn test_retry_settings_are_valid() {
        assert!(DEFAULT_MAX_RETRIES > 0);
        assert!(DEFAULT_RETRY_DELAY > Duration::from_secs(0));
        assert!(DEFAULT_MAX_RETRY_DELAY >= DEFAULT_RETRY_DELAY);
    }

    #[test]
    fn test_circuit_breaker_settings_are_valid() {
        assert!(DEFAULT_CIRCUIT_BREAKER_FAILURE_THRESHOLD > 0);
        assert!(DEFAULT_CIRCUIT_BREAKER_RESET_TIMEOUT > Duration::from_secs(0));
    }

    #[test]
    fn test_bulkhead_settings_are_valid() {
        assert!(DEFAULT_BULKHEAD_MAX_CONCURRENT > 0);
        assert!(DEFAULT_BULKHEAD_MAX_QUEUE_SIZE > 0);
    }

    #[test]
    fn test_workflow_limits_are_reasonable() {
        assert!(DEFAULT_MAX_WORKFLOW_NODES > 0);
        assert!(DEFAULT_MAX_WORKFLOW_DEPTH > 0);
        assert!(DEFAULT_MAX_EXECUTION_TIME > Duration::from_secs(0));
    }

    #[test]
    fn test_node_limits_are_reasonable() {
        assert!(DEFAULT_MAX_NODE_INPUT_SIZE > 0);
        assert!(DEFAULT_MAX_NODE_OUTPUT_SIZE > 0);
        assert!(DEFAULT_MAX_NODE_EXECUTION_TIME > Duration::from_secs(0));
    }

    #[test]
    fn test_action_limits_are_reasonable() {
        assert!(DEFAULT_MAX_ACTION_PARAMETERS > 0);
        assert!(DEFAULT_MAX_ACTION_RESULT_SIZE > 0);
    }

    #[test]
    fn test_expression_limits_are_reasonable() {
        assert!(DEFAULT_MAX_EXPRESSION_LENGTH > 0);
        assert!(DEFAULT_MAX_EXPRESSION_DEPTH > 0);
        assert!(DEFAULT_MAX_EXPRESSION_EXECUTION_TIME > Duration::from_secs(0));
    }

    #[test]
    fn test_event_limits_are_reasonable() {
        assert!(DEFAULT_MAX_EVENT_QUEUE_SIZE > 0);
        assert!(DEFAULT_EVENT_TTL > Duration::from_secs(0));
    }

    #[test]
    fn test_storage_limits_are_reasonable() {
        assert!(DEFAULT_MAX_STORAGE_KEY_LENGTH > 0);
        assert!(DEFAULT_MAX_STORAGE_VALUE_SIZE > 0);
        assert!(DEFAULT_STORAGE_BATCH_SIZE > 0);
    }

    #[test]
    fn test_cluster_settings_are_reasonable() {
        assert!(DEFAULT_CLUSTER_HEARTBEAT_INTERVAL > Duration::from_secs(0));
        assert!(DEFAULT_CLUSTER_ELECTION_TIMEOUT > Duration::from_secs(0));
        assert!(DEFAULT_CLUSTER_MAX_NODES > 0);
    }

    #[test]
    fn test_tenant_limits_are_reasonable() {
        assert!(DEFAULT_MAX_TENANTS > 0);
        assert!(DEFAULT_MAX_WORKFLOWS_PER_TENANT > 0);
        assert!(DEFAULT_MAX_EXECUTIONS_PER_TENANT > 0);
    }

    #[test]
    fn test_api_settings_are_reasonable() {
        assert!(DEFAULT_API_MAX_REQUEST_SIZE > 0);
        assert!(DEFAULT_API_RATE_LIMIT > 0);
        assert!(DEFAULT_API_TIMEOUT > Duration::from_secs(0));
    }

    #[test]
    fn test_logging_settings_are_reasonable() {
        assert!(!DEFAULT_LOG_LEVEL.is_empty());
        assert!(DEFAULT_LOG_MAX_FILES > 0);
        assert!(DEFAULT_LOG_MAX_FILE_SIZE > 0);
    }

    #[test]
    fn test_metrics_settings_are_reasonable() {
        assert!(DEFAULT_METRICS_COLLECTION_INTERVAL > Duration::from_secs(0));
        assert!(DEFAULT_METRICS_RETENTION_PERIOD > Duration::from_secs(0));
    }

    #[test]
    fn test_security_settings_are_reasonable() {
        assert!(DEFAULT_MAX_PASSWORD_LENGTH > DEFAULT_MIN_PASSWORD_LENGTH);
        assert!(DEFAULT_MIN_PASSWORD_LENGTH > 0);
        assert!(DEFAULT_SESSION_TIMEOUT > Duration::from_secs(0));
        assert!(DEFAULT_MAX_LOGIN_ATTEMPTS > 0);
    }

    #[test]
    fn test_validation_settings_are_reasonable() {
        assert!(DEFAULT_MAX_STRING_LENGTH > 0);
        assert!(DEFAULT_MAX_ARRAY_SIZE > 0);
        assert!(DEFAULT_MAX_OBJECT_PROPERTIES > 0);
    }

    #[test]
    fn test_serialization_settings_are_reasonable() {
        assert!(DEFAULT_MAX_SERIALIZATION_SIZE > 0);
        assert!(DEFAULT_SERIALIZATION_TIMEOUT > Duration::from_secs(0));
    }

    #[test]
    fn test_testing_settings_are_reasonable() {
        assert!(DEFAULT_TEST_TIMEOUT > Duration::from_secs(0));
        assert!(DEFAULT_TEST_MAX_ITERATIONS > 0);
    }

    #[test]
    fn test_environment_variables_are_defined() {
        assert!(!env::NEBULA_ENV.is_empty());
        assert!(!env::NEBULA_LOG_LEVEL.is_empty());
        assert!(!env::NEBULA_CONFIG_PATH.is_empty());
    }

    #[test]
    fn test_paths_are_defined() {
        assert!(!paths::DEFAULT_CONFIG_DIR.is_empty());
        assert!(!paths::DEFAULT_CONFIG_FILE.is_empty());
        assert!(!paths::DEFAULT_LOG_DIR.is_empty());
    }

    #[test]
    fn test_http_status_codes_are_valid() {
        assert!(http::HTTP_OK >= 200 && http::HTTP_OK < 300);
        assert!(http::HTTP_BAD_REQUEST >= 400 && http::HTTP_BAD_REQUEST < 500);
        assert!(http::HTTP_INTERNAL_SERVER_ERROR >= 500 && http::HTTP_INTERNAL_SERVER_ERROR < 600);
    }

    #[test]
    fn test_error_codes_are_defined() {
        assert!(!error_codes::VALIDATION_ERROR.is_empty());
        assert!(!error_codes::AUTHENTICATION_ERROR.is_empty());
        assert!(!error_codes::AUTHORIZATION_ERROR.is_empty());
    }

    #[test]
    fn test_feature_flags_are_defined() {
        assert!(!features::ENABLE_METRICS.is_empty());
        assert!(!features::ENABLE_TRACING.is_empty());
        assert!(!features::ENABLE_DEBUG.is_empty());
    }

    #[test]
    fn test_magic_numbers_are_valid() {
        assert_eq!(magic::NEBULA_MAGIC, b"NEBULA");
        assert_eq!(magic::NEBULA_MAGIC_LENGTH, 6);
    }

    #[test]
    fn test_performance_thresholds_are_reasonable() {
        assert!(performance::MAX_WORKFLOW_STARTUP_TIME > Duration::from_millis(0));
        assert!(performance::MAX_NODE_EXECUTION_TIME > Duration::from_millis(0));
        assert!(performance::MAX_EXPRESSION_EVALUATION_TIME > Duration::from_millis(0));
    }

    #[test]
    fn test_security_constants_are_reasonable() {
        assert!(security::MIN_PASSWORD_ENTROPY > 0.0);
        assert!(security::MAX_SESSION_DURATION > security::MIN_SESSION_DURATION);
        assert!(security::MAX_API_KEY_LENGTH > security::MIN_API_KEY_LENGTH);
    }

    #[test]
    fn test_validation_patterns_are_defined() {
        assert!(!patterns::IDENTIFIER_PATTERN.is_empty());
        assert!(!patterns::EMAIL_PATTERN.is_empty());
        assert!(!patterns::URL_PATTERN.is_empty());
        assert!(!patterns::VERSION_PATTERN.is_empty());
    }

    #[test]
    fn test_limits_are_reasonable() {
        assert!(limits::MAX_WORKFLOW_NAME_LENGTH > 0);
        assert!(limits::MAX_WORKFLOW_DESCRIPTION_LENGTH > 0);
        assert!(limits::MAX_NODE_NAME_LENGTH > 0);
        assert!(limits::MAX_ACTION_NAME_LENGTH > 0);
        assert!(limits::MAX_PARAMETER_NAME_LENGTH > 0);
        assert!(limits::MAX_PARAMETER_VALUE_LENGTH > 0);
        assert!(limits::MAX_TAG_LENGTH > 0);
        assert!(limits::MAX_TAGS_PER_ENTITY > 0);
        assert!(limits::MAX_METADATA_KEYS > 0);
        assert!(limits::MAX_METADATA_VALUE_LENGTH > 0);
    }
}
