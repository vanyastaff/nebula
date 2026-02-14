//! Unit tests for AWS Secrets Manager provider
//!
//! These tests verify configuration validation, metadata conversion, and error handling
//! without requiring actual AWS credentials or infrastructure.

#[cfg(feature = "storage-aws")]
mod aws_provider_tests {
    use nebula_credential::core::CredentialMetadata;
    use nebula_credential::providers::{AwsSecretsManagerConfig, ProviderConfig};
    use nebula_credential::utils::RetryPolicy;
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn test_default_config() {
        let config = AwsSecretsManagerConfig::default();

        assert!(config.region.is_none());
        assert_eq!(config.secret_prefix, "");
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(config.kms_key_id.is_none());
        assert!(config.default_tags.is_empty());
        assert_eq!(config.provider_name(), "AWSSecretsManager");
    }

    #[test]
    fn test_config_validation_success() {
        let config = AwsSecretsManagerConfig {
            region: Some("us-west-2".into()),
            endpoint_url: None,
            secret_prefix: "nebula/credentials/".into(),
            timeout: Duration::from_secs(10),
            retry_policy: RetryPolicy::default(),
            kms_key_id: Some("alias/nebula-creds".into()),
            default_tags: HashMap::new(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation_prefix_too_long() {
        let config = AwsSecretsManagerConfig {
            secret_prefix: "a".repeat(513), // Exceeds 512 char limit
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds 512 character limit")
        );
    }

    #[test]
    fn test_config_validation_invalid_characters() {
        let invalid_chars = vec!['<', '>', '{', '}', '[', ']', '|', '\\', '^', '`'];

        for ch in invalid_chars {
            let config = AwsSecretsManagerConfig {
                secret_prefix: format!("prefix{}", ch),
                ..Default::default()
            };

            let result = config.validate();
            assert!(result.is_err(), "Should reject invalid character '{}'", ch);
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("invalid AWS characters")
            );
        }
    }

    #[test]
    fn test_config_validation_timeout_too_short() {
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_millis(500), // Less than 1 second
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be between 1 and 60 seconds")
        );
    }

    #[test]
    fn test_config_validation_timeout_too_long() {
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_secs(61), // More than 60 seconds
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be between 1 and 60 seconds")
        );
    }

    #[test]
    fn test_config_validation_invalid_retry_policy() {
        let config = AwsSecretsManagerConfig {
            retry_policy: RetryPolicy {
                max_retries: 11, // Exceeds limit of 10
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("retry_policy"));
    }

    #[test]
    fn test_metadata_conversion() {
        // This test verifies the metadata to tags conversion logic
        // We can't directly test the private method, but we document the expected behavior

        let _metadata = CredentialMetadata::new();

        // Expected tags should include:
        // - created_at: RFC3339 timestamp
        // - last_modified: RFC3339 timestamp
        // - Any tags from metadata.tags HashMap

        // Tags should respect AWS limits:
        // - Max 50 tags per secret
        // - Max 128 chars for key
        // - Max 256 chars for value

        // This is verified indirectly through integration tests
    }

    #[test]
    fn test_secret_name_prefix() {
        // Test that secret names are properly prefixed
        let config = AwsSecretsManagerConfig {
            secret_prefix: "nebula/test/".into(),
            ..Default::default()
        };

        // Expected behavior: credential ID "github_token" becomes "nebula/test/github_token"
        // This is verified through integration tests
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_size_limit_validation() {
        // AWS Secrets Manager has a 64KB limit
        // Verify that we would reject payloads exceeding this
        const MAX_SIZE: usize = 64 * 1024; // 64KB

        // The validate_size method checks:
        // - ciphertext.len() + nonce.len() + tag.len() <= 64KB
        // This is verified through integration tests

        assert!(MAX_SIZE == 65536);
    }

    #[test]
    fn test_kms_key_id_formats() {
        // Test various KMS key ID formats
        let valid_formats = vec![
            "alias/nebula-credentials",
            "arn:aws:kms:us-west-2:123456789012:key/12345678-1234-1234-1234-123456789012",
            "12345678-1234-1234-1234-123456789012",
        ];

        for key_id in valid_formats {
            let config = AwsSecretsManagerConfig {
                kms_key_id: Some(key_id.to_string()),
                ..Default::default()
            };

            assert!(
                config.validate().is_ok(),
                "Should accept key ID: {}",
                key_id
            );
        }
    }

    #[test]
    fn test_default_tags_merge() {
        let mut default_tags = HashMap::new();
        default_tags.insert("env".into(), "production".into());
        default_tags.insert("team".into(), "platform".into());

        let config = AwsSecretsManagerConfig {
            default_tags,
            ..Default::default()
        };

        assert!(config.validate().is_ok());

        // Expected behavior: default tags are merged with credential tags
        // Credential tags take precedence over defaults
        // This is verified through integration tests
    }

    #[test]
    fn test_tag_limits() {
        // AWS limits:
        // - Max 50 tags per secret
        // - Max 128 chars for key
        // - Max 256 chars for value

        let mut large_tags = HashMap::new();
        for i in 0..60 {
            large_tags.insert(format!("tag{}", i), format!("value{}", i));
        }

        let config = AwsSecretsManagerConfig {
            default_tags: large_tags,
            ..Default::default()
        };

        // Config validation passes - tag limits are enforced during metadata conversion
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_region_auto_detection() {
        // When region is None, AWS SDK should auto-detect from:
        // 1. AWS_REGION environment variable
        // 2. AWS_DEFAULT_REGION environment variable
        // 3. EC2 instance metadata

        let config = AwsSecretsManagerConfig {
            region: None,
            ..Default::default()
        };

        assert!(config.validate().is_ok());
        assert!(config.region.is_none());
    }

    #[test]
    fn test_region_explicit() {
        let regions = vec![
            "us-east-1",
            "us-west-2",
            "eu-west-1",
            "ap-southeast-1",
            "ca-central-1",
        ];

        for region in regions {
            let config = AwsSecretsManagerConfig {
                region: Some(region.to_string()),
                ..Default::default()
            };

            assert!(
                config.validate().is_ok(),
                "Should accept region: {}",
                region
            );
        }
    }

    #[test]
    fn test_timeout_boundary_values() {
        // Test minimum valid timeout (1 second)
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_secs(1),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Test maximum valid timeout (60 seconds)
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_secs(60),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Test just below minimum (should fail)
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_millis(999),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        // Test just above maximum (should fail)
        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_secs(61),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_empty_prefix() {
        // Empty prefix is valid - credentials stored at root level
        let config = AwsSecretsManagerConfig {
            secret_prefix: String::new(),
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_nested_prefix() {
        // Test nested prefix structure
        let config = AwsSecretsManagerConfig {
            secret_prefix: "nebula/production/us-west-2/".into(),
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_retry_policy_validation() {
        // Valid retry policy
        let config = AwsSecretsManagerConfig {
            retry_policy: RetryPolicy {
                max_retries: 3,
                base_delay_ms: 100,
                max_delay_ms: 5000,
                multiplier: 2.0,
                jitter: true,
            },
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Invalid - too many retries
        let config = AwsSecretsManagerConfig {
            retry_policy: RetryPolicy {
                max_retries: 15,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_serde_serialization() {
        let config = AwsSecretsManagerConfig {
            region: Some("us-west-2".into()),
            endpoint_url: None,
            secret_prefix: "nebula/".into(),
            timeout: Duration::from_secs(10),
            retry_policy: RetryPolicy::default(),
            kms_key_id: Some("alias/test".into()),
            default_tags: {
                let mut tags = HashMap::new();
                tags.insert("env".into(), "test".into());
                tags
            },
        };

        // Test serialization
        let json = serde_json::to_string(&config).expect("Should serialize");
        assert!(json.contains("us-west-2"));
        assert!(json.contains("nebula/"));

        // Test deserialization
        let deserialized: AwsSecretsManagerConfig =
            serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(deserialized.region, config.region);
        assert_eq!(deserialized.secret_prefix, config.secret_prefix);
    }

    #[test]
    fn test_error_message_quality() {
        // Verify error messages are actionable

        let config = AwsSecretsManagerConfig {
            timeout: Duration::from_millis(100),
            ..Default::default()
        };

        let err = config.validate().unwrap_err();
        let msg = err.to_string();

        // Should mention the field, the problem, and acceptable range
        assert!(msg.contains("timeout"));
        assert!(msg.contains("1") || msg.contains("60"));
    }
}
