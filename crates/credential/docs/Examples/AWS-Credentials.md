---
title: "AWS Access Credentials"
tags: [example, aws, cloud, credentials, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 12
priority: P1
---

# AWS Access Credentials

> **TL;DR**: Используйте `Credential` trait из nebula-credential для безопасного хранения AWS access keys и secret keys с поддержкой credential chain.

## Обзор

AWS credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Secure Storage**: Access keys и secret keys хранятся в `SecretString`
- **Multiple Sources**: Environment variables, files, IAM roles
- **Testing**: `TestableCredential` для проверки AWS API access
- **Auto-Refresh**: Support for temporary credentials with expiration

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#cloud-credentials]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- AWS account с IAM user или role

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
aws-config = "1.1"
aws-sdk-s3 = "1.15"
aws-types = "1.1"
chrono = "0.4"
```

### Implementing AWS Credential

```rust
// File: examples/aws_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use aws_types::credentials::Credentials as AwsCredentials;
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// AWS credential configuration
#[derive(Debug, Clone)]
pub struct AwsConfig {
    pub region: String,
}

/// AWS credential output
#[derive(Debug, Clone)]
pub struct AwsCredential {
    pub config: AwsConfig,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
    pub expiration: Option<DateTime<Utc>>,
}

impl AwsCredential {
    pub fn new(
        config: AwsConfig,
        access_key_id: String,
        secret_access_key: SecretString,
        session_token: Option<SecretString>,
        expiration: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            config,
            access_key_id,
            secret_access_key,
            session_token,
            expiration,
        }
    }

    /// Convert to AWS SDK credentials
    pub fn to_aws_credentials(&self) -> AwsCredentials {
        AwsCredentials::new(
            &self.access_key_id,
            self.secret_access_key.expose(),
            self.session_token.as_ref().map(|t| t.expose().to_string()),
            self.expiration.map(|dt| dt.into()),
            "nebula-credential",
        )
    }

    /// Get redacted access key for logging
    pub fn access_key_redacted(&self) -> String {
        if self.access_key_id.len() > 8 {
            format!(
                "{}...{}",
                &self.access_key_id[..4],
                &self.access_key_id[self.access_key_id.len() - 4..]
            )
        } else {
            "***".to_string()
        }
    }

    /// Check if credential is expired
    pub fn is_expired(&self) -> bool {
        if let Some(exp) = self.expiration {
            Utc::now() >= exp
        } else {
            false
        }
    }
}

/// AWS credential provider
pub struct AwsCredentialProvider {
    config: AwsConfig,
    credential_source: CredentialSource,
}

#[derive(Debug, Clone)]
pub enum CredentialSource {
    Environment,
    Static {
        access_key_id: String,
        secret_access_key: SecretString,
    },
    SecretsManager {
        path: String,
    },
}

impl AwsCredentialProvider {
    pub fn new(config: AwsConfig, credential_source: CredentialSource) -> Self {
        Self {
            config,
            credential_source,
        }
    }

    async fn resolve_credentials(
        &self,
    ) -> Result<(String, SecretString, Option<SecretString>), CredentialError> {
        match &self.credential_source {
            CredentialSource::Environment => {
                let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
                    .map_err(|_| CredentialError::ConfigurationError(
                        "AWS_ACCESS_KEY_ID environment variable not found".to_string()
                    ))?;

                let secret_access_key = SecretString::new(
                    std::env::var("AWS_SECRET_ACCESS_KEY")
                        .map_err(|_| CredentialError::ConfigurationError(
                            "AWS_SECRET_ACCESS_KEY environment variable not found".to_string()
                        ))?
                );

                let session_token = std::env::var("AWS_SESSION_TOKEN")
                    .ok()
                    .map(SecretString::new);

                Ok((access_key_id, secret_access_key, session_token))
            }
            CredentialSource::Static {
                access_key_id,
                secret_access_key,
            } => Ok((access_key_id.clone(), secret_access_key.clone(), None)),
            CredentialSource::SecretsManager { path } => {
                // Integration with storage provider
                Err(CredentialError::ConfigurationError(format!(
                    "Secrets manager integration not yet implemented for path: {}",
                    path
                )))
            }
        }
    }
}

#[async_trait]
impl Credential for AwsCredentialProvider {
    type Output = AwsCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let (access_key_id, secret_access_key, session_token) =
            self.resolve_credentials().await?;

        Ok(AwsCredential::new(
            self.config.clone(),
            access_key_id,
            secret_access_key,
            session_token,
            None, // No expiration for long-term credentials
        ))
    }

    fn credential_type(&self) -> &'static str {
        "aws"
    }

    fn supports_refresh(&self) -> bool {
        false // Long-term credentials don't need refresh
    }
}

#[async_trait]
impl TestableCredential for AwsCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Create AWS config with credentials
        let aws_creds = credential.to_aws_credentials();
        let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(credential.config.region.clone()))
            .credentials_provider(aws_creds)
            .load()
            .await;

        // Test with STS GetCallerIdentity
        let sts_client = aws_sdk_sts::Client::new(&aws_config);

        match sts_client.get_caller_identity().send().await {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: format!("sts.{}.amazonaws.com", credential.config.region),
                    permissions_verified: vec!["sts:GetCallerIdentity".to_string()],
                    metadata: HashMap::from([
                        ("account_id".to_string(),
                         serde_json::json!(response.account().unwrap_or("unknown"))),
                        ("user_id".to_string(),
                         serde_json::json!(response.user_id().unwrap_or("unknown"))),
                        ("arn".to_string(),
                         serde_json::json!(response.arn().unwrap_or("unknown"))),
                    ]),
                };

                Ok(TestResult::success("AWS credentials valid")
                    .with_details(details))
            }
            Err(e) => Ok(TestResult::failure(format!(
                "AWS STS GetCallerIdentity failed: {}",
                e
            ))),
        }
    }

    fn test_description(&self) -> &str {
        "Testing AWS credentials by calling STS GetCallerIdentity"
    }
}
```

### Usage Example

```rust
// File: examples/use_aws_credential.rs
use nebula_credential::{
    Credential, TestableCredential, CredentialContext, OwnerId,
};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("☁️  AWS Credential Example\n");

    // 1. Configure AWS credential
    let config = AwsConfig {
        region: "us-east-1".to_string(),
    };

    let provider = AwsCredentialProvider::new(
        config,
        CredentialSource::Environment,
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve credential
    println!("📡 Retrieving AWS credential...");
    let credential = provider.retrieve(&ctx).await?;

    println!("✅ Credential retrieved");
    println!("   Access Key: {}", credential.access_key_redacted());
    println!("   Region: {}", credential.config.region);
    println!("   Session Token: {}", 
        if credential.session_token.is_some() { "present" } else { "none" });
    println!("   Credential type: {}", provider.credential_type());

    // 4. Test credential validity
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Account ID: {:?}", details.metadata.get("account_id"));
            println!("   ARN: {:?}", details.metadata.get("arn"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
        return Ok(());
    }

    // 5. Use with AWS SDK
    println!("\n📦 Using AWS SDK (S3)...");

    let aws_creds = credential.to_aws_credentials();
    let aws_config = aws_config::defaults(BehaviorVersion::latest())
        .region(aws_config::Region::new(credential.config.region.clone()))
        .credentials_provider(aws_creds)
        .load()
        .await;

    let s3_client = S3Client::new(&aws_config);

    // List buckets
    match s3_client.list_buckets().send().await {
        Ok(output) => {
            let bucket_count = output.buckets().len();
            println!("   ✓ Found {} S3 buckets", bucket_count);

            for (i, bucket) in output.buckets().iter().take(3).enumerate() {
                println!("      {}. {}", i + 1, bucket.name().unwrap_or("unnamed"));
            }
        }
        Err(e) => {
            println!("   ✗ Error listing buckets: {}", e);
        }
    }

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для AWS

```rust
impl Credential for AwsCredentialProvider {
    type Output = AwsCredential; // Contains access keys
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Resolve credentials from source
        // Return AwsCredential with SecretString
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. SecretString для AWS Keys

```rust
// ✅ GOOD: Secret key auto-zeroized
let secret_key = SecretString::new(env::var("AWS_SECRET_ACCESS_KEY")?);

// Expose only when needed
let aws_creds = AwsCredentials::new(
    &access_key_id,
    secret_key.expose(),
    None,
    None,
    "provider"
);

// Redacted logging
println!("Key: {}", credential.access_key_redacted()); // Shows AKIA...MPLE
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#secret-handling]]

### 3. TestableCredential Implementation

```rust
impl TestableCredential for AwsCredentialProvider {
    async fn test(&self, ctx: &CredentialContext) 
        -> Result<TestResult, CredentialError> {
        // Call STS GetCallerIdentity
        // Verify credentials are valid
        // Return account ID and ARN
    }
}
```

### 4. Multiple Credential Sources

```rust
// From environment
let provider = AwsCredentialProvider::new(
    config,
    CredentialSource::Environment,
);

// Static credentials
let provider = AwsCredentialProvider::new(
    config,
    CredentialSource::Static {
        access_key_id: "AKIA...".to_string(),
        secret_access_key: SecretString::new("secret"),
    },
);

// From secrets manager
let provider = AwsCredentialProvider::new(
    config,
    CredentialSource::SecretsManager {
        path: "aws/prod/credentials".to_string(),
    },
);
```

## Security Best Practices

> [!warning] Never Hardcode AWS Keys
> Always use environment variables, IAM roles, or secrets managers for AWS credentials.

**Least Privilege IAM Policy**:
```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["s3:GetObject", "s3:PutObject"],
    "Resource": "arn:aws:s3:::my-bucket/*"
  }]
}
```

**Credential Rotation**:
```rust
// Use RotatableCredential for periodic rotation
// See AWS-AssumeRole.md for temporary credentials
```

## Common Issues

### Issue 1: Access Denied

**Symptoms**: `CredentialError::AuthenticationFailed`

**Solution**: Check IAM permissions:
```bash
aws sts get-caller-identity
aws iam list-attached-user-policies --user-name my-user
```

### Issue 2: Invalid Access Key

**Symptoms**: `InvalidAccessKeyId`

**Solution**: Verify environment variables:
```bash
echo $AWS_ACCESS_KEY_ID
echo $AWS_SECRET_ACCESS_KEY
```

## Related Examples

- **Cloud Credentials**: [[AWS-AssumeRole]] - AWS temporary credentials with STS
- **Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials | [[Database-MySQL]] - MySQL credentials
- **Basic Auth**: [[API-Key-Basic]] - Simple API key authentication

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]
- [[../../specs/001-credential-docs/security-spec.md#cloud-credentials]]
- [[../../specs/001-credential-docs/technical-design.md#aws-credentials]]

## Sources

- [AWS SDK for Rust](https://github.com/awslabs/aws-sdk-rust)
- [nebula-credential API](../Reference/API-Reference.md)
