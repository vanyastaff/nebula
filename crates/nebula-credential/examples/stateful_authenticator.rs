//! Stateful Authenticator Example
//!
//! This example demonstrates the StatefulAuthenticator pattern which gives
//! authenticators access to the full credential state, not just the token.
//!
//! This is useful for credentials that need multiple pieces of information:
//! - Database credentials (username + password + host)
//! - OAuth credentials (client_id + client_secret + tokens)
//! - Service accounts (multiple keys/certificates)

use async_trait::async_trait;
use nebula_credential::authenticator::{AuthenticateWithState, StatefulAuthenticator};
use nebula_credential::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState, Result,
    SecureString,
};
use nebula_credential::testing::{MockLock, MockStateStore, MockTokenCache};
use nebula_credential::traits::{Credential, StateStore, TokenCache};
use nebula_credential::{CredentialManager, CredentialRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ═══════════════════════════════════════════════════════════════
// Example 1: Database Credential
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseInput {
    host: String,
    port: u16,
    username: String,
    password: String,
    database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseState {
    host: String,
    port: u16,
    username: String,
    password: SecureString,
    database: String,
    rotation_count: u32,
}

impl CredentialState for DatabaseState {
    const KIND: &'static str = "database";
    const VERSION: u16 = 1;
}

struct DatabaseCredential;

#[async_trait]
impl Credential for DatabaseCredential {
    type Input = DatabaseInput;
    type State = DatabaseState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "database",
            name: "Database Credentials",
            description: "Username/password for database connection",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        let state = DatabaseState {
            host: input.host.clone(),
            port: input.port,
            username: input.username.clone(),
            password: SecureString::new(input.password.clone()),
            database: input.database.clone(),
            rotation_count: 0,
        };

        // Token is just password for simple cases
        let token = AccessToken::bearer(input.password.clone())
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken> {
        // Simulate password rotation
        state.rotation_count += 1;
        let new_password = format!("rotated_password_{}", state.rotation_count);
        state.password = SecureString::new(new_password.clone());

        let token = AccessToken::bearer(new_password)
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok(token)
    }
}

// ═══════════════════════════════════════════════════════════════
// Mock Database Connection
// ═══════════════════════════════════════════════════════════════

#[derive(Debug)]
struct DatabaseConnection {
    connection_string: String,
    connected_at: SystemTime,
}

struct DatabaseConnectOptions {
    host: String,
    port: u16,
    database: String,
}

impl DatabaseConnectOptions {
    fn new(host: impl Into<String>, port: u16, database: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            database: database.into(),
        }
    }

    async fn connect(
        self,
        username: &str,
        password: &str,
    ) -> std::result::Result<DatabaseConnection, String> {
        // Simulate connection
        let connection_string = format!(
            "postgresql://{}:{}@{}:{}/{}",
            username, password, self.host, self.port, self.database
        );

        Ok(DatabaseConnection {
            connection_string,
            connected_at: SystemTime::now(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// Database Authenticator using StatefulAuthenticator
// ═══════════════════════════════════════════════════════════════

struct DatabaseAuthenticator;

#[async_trait]
impl StatefulAuthenticator<DatabaseCredential> for DatabaseAuthenticator {
    type Target = DatabaseConnectOptions;
    type Output = DatabaseConnection;

    async fn authenticate(
        &self,
        options: Self::Target,
        state: &DatabaseState,
    ) -> std::result::Result<Self::Output, CredentialError> {
        // ✅ We have access to FULL state: username, password, host, etc!
        let connection = options
            .connect(&state.username, state.password.expose())
            .await
            .map_err(|e| CredentialError::InvalidConfiguration {
                reason: format!("Connection failed: {}", e),
            })?;

        Ok(connection)
    }
}

// ═══════════════════════════════════════════════════════════════
// Example 2: Service Account Credential (multiple keys)
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceAccountInput {
    project_id: String,
    private_key: String,
    client_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceAccountState {
    project_id: String,
    private_key: SecureString,
    client_email: String,
    scopes: Vec<String>,
}

impl CredentialState for ServiceAccountState {
    const KIND: &'static str = "service_account";
    const VERSION: u16 = 1;
}

struct ServiceAccountCredential;

#[async_trait]
impl Credential for ServiceAccountCredential {
    type Input = ServiceAccountInput;
    type State = ServiceAccountState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "service_account",
            name: "Service Account",
            description: "GCP/AWS service account credentials",
            supports_refresh: false,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        let state = ServiceAccountState {
            project_id: input.project_id.clone(),
            private_key: SecureString::new(input.private_key.clone()),
            client_email: input.client_email.clone(),
            scopes: vec!["https://www.googleapis.com/auth/cloud-platform".to_string()],
        };

        let token = AccessToken::bearer("service-account-token".to_string())
            .with_expiration(SystemTime::now() + Duration::from_secs(3600));

        Ok((state, Some(token)))
    }
}

#[derive(Debug)]
struct ServiceAccountClient {
    project_id: String,
    client_email: String,
}

struct ServiceAccountAuthenticator;

#[async_trait]
impl StatefulAuthenticator<ServiceAccountCredential> for ServiceAccountAuthenticator {
    type Target = ();
    type Output = ServiceAccountClient;

    async fn authenticate(
        &self,
        _target: Self::Target,
        state: &ServiceAccountState,
    ) -> std::result::Result<Self::Output, CredentialError> {
        // ✅ Access to project_id, private_key, email, scopes
        println!(
            "   Creating client for project: {}, email: {}",
            state.project_id, state.client_email
        );

        Ok(ServiceAccountClient {
            project_id: state.project_id.clone(),
            client_email: state.client_email.clone(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// Main Example
// ═══════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║   Nebula Credential - Stateful Authenticator Example    ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════════
    // Setup
    // ═══════════════════════════════════════════════════════════════
    println!("📦 Setup: Creating CredentialManager");

    let store = Arc::new(MockStateStore::new());
    let cache = Arc::new(MockTokenCache::new());
    let registry = Arc::new(CredentialRegistry::new());

    registry.register_credential(DatabaseCredential);
    registry.register_credential(ServiceAccountCredential);

    let manager = CredentialManager::builder()
        .with_store(store as Arc<dyn StateStore>)
        .with_cache(cache as Arc<dyn TokenCache>)
        .with_lock(MockLock::new())
        .with_registry(registry)
        .build()?;

    println!("   ✓ CredentialManager ready\n");

    // ═══════════════════════════════════════════════════════════════
    // Example 1: Database Credential with StatefulAuthenticator
    // ═══════════════════════════════════════════════════════════════
    println!("🗄️  Example 1: Database Connection with Full State");

    let db_cred_id = manager
        .create_credential(
            "database",
            serde_json::json!({
                "host": "localhost",
                "port": 5432,
                "username": "app_user",
                "password": "secret_password_123",
                "database": "production_db"
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", db_cred_id);

    // In real usage, State would be obtained from CredentialManager internally
    // For this example, we'll create state manually to show the concept
    let state = DatabaseState {
        host: "localhost".to_string(),
        port: 5432,
        username: "app_user".to_string(),
        password: SecureString::new("secret_password_123".to_string()),
        database: "production_db".to_string(),
        rotation_count: 0,
    };

    println!("\n   State contains:");
    println!("      ├─ Host: {}", state.host);
    println!("      ├─ Port: {}", state.port);
    println!("      ├─ Username: {}", state.username);
    println!("      ├─ Password: {} chars (secured)", state.password.with_exposed(|s| s.len()));
    println!("      └─ Database: {}", state.database);

    // Create connection using StatefulAuthenticator
    println!("\n   Creating database connection...");
    let authenticator = DatabaseAuthenticator;
    let options = DatabaseConnectOptions::new("localhost", 5432, "production_db");

    let connection = options
        .authenticate_with_state(&authenticator, &state)
        .await?;

    println!("   ✓ Connected!");
    println!("      ├─ Connection string: {}", connection.connection_string);
    println!("      └─ Connected at: {:?}", connection.connected_at);

    // ═══════════════════════════════════════════════════════════════
    // Example 2: Service Account Credential
    // ═══════════════════════════════════════════════════════════════
    println!("\n🔑 Example 2: Service Account Client");

    let sa_cred_id = manager
        .create_credential(
            "service_account",
            serde_json::json!({
                "project_id": "my-gcp-project",
                "private_key": "-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----",
                "client_email": "service@project.iam.gserviceaccount.com"
            }),
        )
        .await?;

    println!("   ✓ Credential created: {}", sa_cred_id);

    // Create state manually for demo
    let sa_state = ServiceAccountState {
        project_id: "my-gcp-project".to_string(),
        private_key: SecureString::new("-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----".to_string()),
        client_email: "service@project.iam.gserviceaccount.com".to_string(),
        scopes: vec!["https://www.googleapis.com/auth/cloud-platform".to_string()],
    };

    let sa_auth = ServiceAccountAuthenticator;
    let client = ().authenticate_with_state(&sa_auth, &sa_state).await?;

    println!("   ✓ Service account client created");
    println!("      ├─ Project: {}", client.project_id);
    println!("      └─ Email: {}", client.client_email);

    // ═══════════════════════════════════════════════════════════════
    // Example 3: Credential Rotation (Conceptual)
    // ═══════════════════════════════════════════════════════════════
    println!("\n🔄 Example 3: Credential Rotation (Conceptual)");

    println!("   Simulating credential rotation...");

    // Simulate rotated state
    let new_state = DatabaseState {
        host: "localhost".to_string(),
        port: 5432,
        username: "app_user".to_string(),
        password: SecureString::new("rotated_password_1".to_string()),
        database: "production_db".to_string(),
        rotation_count: 1,
    };

    println!("   ✓ Credential rotated");
    println!("      ├─ Rotation count: {}", new_state.rotation_count);
    println!("      └─ New password: {} chars", new_state.password.with_exposed(|s| s.len()));

    // Reconnect with new credentials
    let options = DatabaseConnectOptions::new("localhost", 5432, "production_db");
    let new_connection = options
        .authenticate_with_state(&authenticator, &new_state)
        .await?;

    println!("   ✓ Reconnected with rotated credentials");
    println!("      └─ New connection string includes rotated password");

    // ═══════════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════════
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║                        Summary                           ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ StatefulAuthenticator has access to full state        ║");
    println!("║ ✓ Perfect for multi-field credentials (DB, OAuth, etc)  ║");
    println!("║ ✓ Type-safe: Authenticator knows exact State type       ║");
    println!("║ ✓ Works with credential rotation                        ║");
    println!("║ ✓ Extension trait for fluent API                        ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Key Differences:");
    println!("   ClientAuthenticator:");
    println!("      • Gets only AccessToken (string)");
    println!("      • Good for: HTTP Bearer, API keys");
    println!("      • Example: request.authenticate_with(&HttpBearer, &token)");
    println!();
    println!("   StatefulAuthenticator:");
    println!("      • Gets full Credential State (typed struct)");
    println!("      • Good for: Databases, OAuth, multi-field credentials");
    println!("      • Example: options.authenticate_with_state(&auth, &state)");

    println!("\n💡 Use Cases:");
    println!("   • PostgreSQL: username, password, host, port, database");
    println!("   • MongoDB: username, password, replica set, auth db");
    println!("   • OAuth2: client_id, client_secret, refresh_token, scopes");
    println!("   • Service Accounts: project_id, private_key, scopes");
    println!("   • SSH Keys: username, private_key, passphrase");

    Ok(())
}
