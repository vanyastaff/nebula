---
title: "AWS AssumeRole with STS"
tags: [example, aws, sts, temporary-credentials, advanced]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 10
priority: P1
---

# AWS AssumeRole with STS

> **TL;DR**: Используйте `Credential` trait из nebula-credential для получения temporary AWS credentials через STS AssumeRole с автоматическим refresh.

## Обзор

AWS STS AssumeRole credentials в nebula-credential управляются через `Credential` trait pattern с поддержкой:
- **Temporary Credentials**: Access keys с expiration time
- **Auto-Refresh**: `RotatableCredential` для automatic renewal
- **Cross-Account Access**: Assume roles в других AWS accounts
- **External ID**: Protection от Confused Deputy attacks

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#temporary-credentials]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[AWS-Credentials|Понимание AWS credentials]]
- IAM role с trust policy
- Permissions для `sts:AssumeRole`

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
aws-config = "1.1"
aws-sdk-sts = "1.13"
aws-sdk-s3 = "1.15"
aws-types = "1.1"
chrono = "0.4"
```

### Implementing AWS AssumeRole Credential

```rust
// File: examples/aws_assume_role_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    RotatableCredential, TestableCredential,
    SecretString, TestResult, TestDetails, OwnerId,
    RotationPolicy,
};
use async_trait::async_trait;
use aws_types::credentials::Credentials as AwsCredentials;
use aws_sdk_sts::Client as StsClient;
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};

/// AWS AssumeRole configuration
#[derive(Debug, Clone)]
pub struct AwsAssumeRoleConfig {
    pub role_arn: String,
    pub session_name: String,
    pub external_id: Option<String>,
    pub duration_seconds: i32,
    pub region: String,
}

/// AWS AssumeRole credential output
#[derive(Debug, Clone)]
pub struct AwsAssumeRoleCredential {
    pub config: AwsAssumeRoleConfig,
    pub access_key_id: String,
    pub secret_access_key: SecretString,
    pub session_token: SecretString,
    pub expiration: DateTime<Utc>,
}

impl AwsAssumeRoleCredential {
    pub fn new(
        config: AwsAssumeRoleConfig,
        access_key_id: String,
        secret_access_key: SecretString,
        session_token: SecretString,
        expiration: DateTime<Utc>,
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
            Some(self.session_token.expose().to_string()),
            Some(self.expiration.into()),
            "nebula-assume-role",
        )
    }

    /// Check if credential will expire soon
    pub fn needs_refresh(&self, threshold: Duration) -> bool {
        Utc::now() + threshold >= self.expiration
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
}

/// AWS AssumeRole credential provider
pub struct AwsAssumeRoleProvider {
    config: AwsAssumeRoleConfig,
    base_credentials: BaseCredentialSource,
}

#[derive(Debug, Clone)]
pub enum BaseCredentialSource {
    Environment,
    Static {
        access_key_id: String,
        secret_access_key: SecretString,
    },
}

impl AwsAssumeRoleProvider {
    pub fn new(config: AwsAssumeRoleConfig, base_credentials: BaseCredentialSource) -> Self {
        Self {
            config,
            base_credentials,
        }
    }

    async fn create_sts_client(&self) -> Result<StsClient, CredentialError> {
        let aws_config = match &self.base_credentials {
            BaseCredentialSource::Environment => {
                aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(self.config.region.clone()))
                    .load()
                    .await
            }
            BaseCredentialSource::Static { access_key_id, secret_access_key } => {
                let creds = AwsCredentials::new(
                    access_key_id,
                    secret_access_key.expose(),
                    None,
                    None,
                    "base-credentials",
                );
                
                aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(self.config.region.clone()))
                    .credentials_provider(creds)
                    .load()
                    .await
            }
        };

        Ok(StsClient::new(&aws_config))
    }
}

#[async_trait]
impl Credential for AwsAssumeRoleProvider {
    type Output = AwsAssumeRoleCredential;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let sts_client = self.create_sts_client().await?;

        let mut request = sts_client
            .assume_role()
            .role_arn(&self.config.role_arn)
            .role_session_name(&self.config.session_name)
            .duration_seconds(self.config.duration_seconds);

        if let Some(ref external_id) = self.config.external_id {
            request = request.external_id(external_id);
        }

        let response = request.send().await
            .map_err(|e| CredentialError::AuthenticationFailed(
                format!("AssumeRole failed: {}", e)
            ))?;

        let creds = response.credentials()
            .ok_or_else(|| CredentialError::InvalidResponse(
                "No credentials in AssumeRole response".to_string()
            ))?;

        let expiration = DateTime::parse_from_rfc3339(creds.expiration())
            .map_err(|e| CredentialError::InvalidResponse(
                format!("Invalid expiration format: {}", e)
            ))?
            .with_timezone(&Utc);

        Ok(AwsAssumeRoleCredential::new(
            self.config.clone(),
            creds.access_key_id().to_string(),
            SecretString::new(creds.secret_access_key().to_string()),
            SecretString::new(creds.session_token().to_string()),
            expiration,
        ))
    }

    fn credential_type(&self) -> &'static str {
        "aws_assume_role"
    }

    fn supports_refresh(&self) -> bool {
        true // Temporary credentials need refresh
    }
}

#[async_trait]
impl TestableCredential for AwsAssumeRoleProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        // Create AWS config with assumed credentials
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
                        ("assumed_role_arn".to_string(),
                         serde_json::json!(response.arn().unwrap_or("unknown"))),
                        ("account_id".to_string(),
                         serde_json::json!(response.account().unwrap_or("unknown"))),
                        ("expiration".to_string(),
                         serde_json::json!(credential.expiration.to_rfc3339())),
                    ]),
                };

                Ok(TestResult::success("AWS AssumeRole credentials valid")
                    .with_details(details))
            }
            Err(e) => Ok(TestResult::failure(format!(
                "AWS STS GetCallerIdentity failed: {}",
                e
            ))),
        }
    }

    fn test_description(&self) -> &str {
        "Testing AWS AssumeRole credentials by calling STS GetCallerIdentity"
    }
}

/// Rotation policy for temporary credentials
pub struct TemporaryCredentialRotationPolicy {
    pub refresh_threshold: Duration,
}

impl RotationPolicy for TemporaryCredentialRotationPolicy {
    fn should_rotate_by_age(&self, created_at: DateTime<Utc>) -> bool {
        // Not used for temporary credentials
        false
    }

    fn should_rotate_by_usage(&self, _usage_count: u64) -> bool {
        false
    }
}

#[async_trait]
impl RotatableCredential for AwsAssumeRoleProvider {
    type Policy = TemporaryCredentialRotationPolicy;

    async fn rotate(
        &self,
        current: &Self::Output,
        policy: &Self::Policy,
        ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        println!("🔄 Rotating AWS AssumeRole credentials");
        
        // Simply retrieve new credentials (AssumeRole again)
        self.retrieve(ctx).await
    }

    async fn needs_rotation(
        &self,
        credential: &Self::Output,
        policy: &Self::Policy,
    ) -> Result<bool, Self::Error> {
        // Check if credential expires soon
        Ok(credential.needs_refresh(policy.refresh_threshold))
    }
}
```

### Usage Example

```rust
// File: examples/use_aws_assume_role.rs
use nebula_credential::{
    Credential, TestableCredential, RotatableCredential,
    CredentialContext, OwnerId,
};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use chrono::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎭 AWS AssumeRole Credential Example\n");

    // 1. Configure AssumeRole
    let config = AwsAssumeRoleConfig {
        role_arn: "arn:aws:iam::222222222222:role/CrossAccountRole".to_string(),
        session_name: "my-app-session".to_string(),
        external_id: Some("unique-external-id-12345".to_string()),
        duration_seconds: 3600, // 1 hour
        region: "us-east-1".to_string(),
    };

    let provider = AwsAssumeRoleProvider::new(
        config,
        BaseCredentialSource::Environment,
    );

    // 2. Create credential context
    let ctx = CredentialContext::new(OwnerId::new("my-workflow"))
        .with_metadata("environment".to_string(), "production".to_string());

    // 3. Retrieve temporary credentials
    println!("📡 Assuming role...");
    let credential = provider.retrieve(&ctx).await?;

    println!("✅ Role assumed successfully");
    println!("   Access Key: {}", credential.access_key_redacted());
    println!("   Expiration: {}", credential.expiration);
    println!("   Credential type: {}", provider.credential_type());

    // 4. Test credential validity
    println!("\n🧪 Testing credential...");
    let test_result = provider.test(&ctx).await?;

    if test_result.success {
        println!("✅ Test passed: {}", test_result.message);
        if let Some(details) = test_result.details {
            println!("   Latency: {}ms", details.latency_ms);
            println!("   Assumed Role ARN: {:?}", details.metadata.get("assumed_role_arn"));
        }
    } else {
        println!("❌ Test failed: {}", test_result.message);
        return Ok(());
    }

    // 5. Use with AWS SDK
    println!("\n📦 Using assumed credentials with S3...");

    let aws_creds = credential.to_aws_credentials();
    let aws_config = aws_config::defaults(BehaviorVersion::latest())
        .region(aws_config::Region::new(credential.config.region.clone()))
        .credentials_provider(aws_creds)
        .load()
        .await;

    let s3_client = S3Client::new(&aws_config);

    match s3_client.list_buckets().send().await {
        Ok(output) => {
            println!("   ✓ Access granted to {} buckets", output.buckets().len());
        }
        Err(e) => {
            println!("   ✗ Error: {}", e);
        }
    }

    // 6. Check if rotation needed
    println!("\n🔄 Checking rotation status...");
    
    let policy = TemporaryCredentialRotationPolicy {
        refresh_threshold: Duration::minutes(5), // Refresh 5 min before expiry
    };

    let needs_rotation = provider.needs_rotation(&credential, &policy).await?;
    println!("   Needs rotation: {}", needs_rotation);

    Ok(())
}
```

## Key Concepts

### 1. Credential Trait для AssumeRole

```rust
impl Credential for AwsAssumeRoleProvider {
    type Output = AwsAssumeRoleCredential; // With expiration
    type Error = CredentialError;
    
    async fn retrieve(&self, ctx: &CredentialContext) 
        -> Result<Self::Output, Self::Error> {
        // Call STS AssumeRole
        // Return temporary credentials with expiration
    }
}
```

**Architecture**: См. [[Architecture#credential-trait-hierarchy]]

### 2. RotatableCredential for Auto-Refresh

```rust
impl RotatableCredential for AwsAssumeRoleProvider {
    async fn needs_rotation(&self, credential: &Self::Output, policy: &Self::Policy) 
        -> Result<bool, Self::Error> {
        // Check if credential expires within threshold
        Ok(credential.needs_refresh(policy.refresh_threshold))
    }
}
```

### 3. SecretString для Session Token

```rust
// ✅ GOOD: Session token auto-zeroized
let session_token = SecretString::new(creds.session_token().to_string());

// Convert to AWS SDK
let aws_creds = AwsCredentials::new(
    &access_key_id,
    secret_access_key.expose(),
    Some(session_token.expose().to_string()),
    Some(expiration),
    "provider"
);
```

**Security**: См. [[../../specs/001-credential-docs/security-spec.md#temporary-credentials]]

## Security Best Practices

> [!warning] External ID Required
> Всегда используйте External ID для cross-account roles (защита от Confused Deputy attacks).

**Minimum Duration**:
```rust
let config = AwsAssumeRoleConfig {
    duration_seconds: 900, // ✅ 15 min minimum
    // ...
};
```

**Session Name with Audit Info**:
```rust
let session_name = format!("{}@{}", username, app_name);
// CloudTrail will show who assumed the role
```

## Common Issues

### Issue 1: AccessDenied

**Symptoms**: `CredentialError::AuthenticationFailed`

**Solution**: Check trust policy allows AssumeRole from your account.

### Issue 2: Invalid ExternalId

**Symptoms**: `AccessDenied` даже с правильным role ARN

**Solution**: Verify external_id matches trust policy condition.

## Related Examples

- **Cloud Credentials**: [[AWS-Credentials]] - AWS access keys and profiles
- **Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials | [[Database-MySQL]] - MySQL credentials
- **Token-Based**: [[JWT-Validation]] - JWT token validation

## See Also

- [[Core-Concepts|Core Concepts]] - понимание Credential trait
- [[Rotate-Credentials|Rotation Guide]] - credential rotation
- [[API-Reference|API Reference]] - полная документация

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#credential-trait-hierarchy]]
- [[../../specs/001-credential-docs/security-spec.md#temporary-credentials]]
- [[../../specs/001-credential-docs/technical-design.md#aws-assume-role]]

## Sources

- [AWS STS AssumeRole](https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRole.html)
- [nebula-credential API](../Reference/API-Reference.md)
