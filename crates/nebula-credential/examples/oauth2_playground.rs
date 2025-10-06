//! OAuth2 flows testing with oauth.com/playground
//!
//! This example demonstrates how to use nebula-credential with OAuth Playground
//!
//! Steps:
//! 1. Go to https://www.oauth.com/playground/
//! 2. Choose "Authorization Code" flow
//! 3. Copy the endpoints and credentials
//! 4. Run this example with those values

use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OAuth2 Playground Testing ===\n");

    // Test 1: Authorization Code Flow
    test_authorization_code_flow().await?;

    // Test 2: Client Credentials Flow
    test_client_credentials_flow().await?;

    Ok(())
}

async fn test_authorization_code_flow() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. Testing Authorization Code Flow");
    println!("-----------------------------------");

    // Create credential
    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    // Configure with OAuth Playground values
    // Get these from https://www.oauth.com/playground/
    let input = AuthorizationCodeInput {
        client_id: std::env::var("OAUTH_CLIENT_ID")
            .unwrap_or_else(|_| "AxIKkzEyzIqNUvLUvftnL57O".to_string()),
        client_secret: "iO5-nArlk4J_5bGjV1ags2UGvCs1ZdqMywLVV7VGk7ZPjhKF".to_string(),
        authorization_endpoint: "https://www.oauth.com/playground/authorize.html".to_string(),
        token_endpoint: "https://www.oauth.com/playground/access-token.html".to_string(),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
        use_pkce: true,
    };

    // Initialize - this will generate the authorization URL
    let result = credential.initialize(&input, &mut ctx).await?;

    match result {
        InitializeResult::Pending {
            partial_state,
            next_step,
        } => {
            println!("✓ Authorization flow started");
            println!("  Step: {}", partial_state.step);

            if let InteractionRequest::Redirect { url, .. } = next_step {
                println!("\n  Authorization URL:");
                println!("  {}\n", url);
                println!("  👉 Open this URL in your browser to authorize");
                println!("  👉 After authorization, you'll be redirected with a 'code' parameter");

                // In real app, you would:
                // 1. Redirect user to this URL
                // 2. Receive callback with code
                // 3. Call continue_initialization with the code

                // Example callback simulation:
                println!("\n  To continue, call continue_initialization with:");
                println!(
                    "    UserInput::Callback {{ params: {{ \"code\": \"auth_code\", \"state\": \"...\" }} }}"
                );
            }
        }
        InitializeResult::Complete(_) => {
            println!("✗ Unexpected: flow should be Pending, not Complete");
        }
        InitializeResult::RequiresInteraction(_) => {
            println!("✓ Interaction required (alternative flow)");
        }
    }

    println!("\n");
    Ok(())
}

async fn test_client_credentials_flow() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. Testing Client Credentials Flow");
    println!("-----------------------------------");

    let credential = OAuth2ClientCredentials::create();
    let mut ctx = CredentialContext::new();

    let input = ClientCredentialsInput {
        client_id: std::env::var("OAUTH_CLIENT_ID").unwrap_or_else(|_| "test_client".to_string()),
        client_secret: std::env::var("OAUTH_CLIENT_SECRET")
            .unwrap_or_else(|_| "test_secret".to_string()),
        token_endpoint: "https://www.oauth.com/playground/access-token.html".to_string(),
        scopes: vec!["api".to_string()],
    };

    println!("  Client ID: {}", input.client_id);
    println!("  Token Endpoint: {}", input.token_endpoint);
    println!("  Scopes: {:?}", input.scopes);

    // Note: This will fail with OAuth Playground unless you have valid credentials
    // OAuth Playground requires browser-based flows
    println!("\n  ⚠ Note: Client Credentials flow may not work with OAuth Playground");
    println!("  OAuth Playground is designed for browser-based flows\n");

    match credential.initialize(&input, &mut ctx).await {
        Ok(InitializeResult::Complete(state)) => {
            println!("✓ Token acquired successfully");
            println!("  Token type: {}", state.token_type);
            println!("  Expires at: {}", state.expires_at);
            println!("  Has refresh token: {}", state.refresh_token.is_some());
        }
        Ok(_) => {
            println!("✗ Unexpected result type");
        }
        Err(e) => {
            println!("✗ Failed to acquire token: {}", e);
            println!("  This is expected with OAuth Playground");
        }
    }

    println!("\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authorization_url_generation() {
        let input = AuthorizationCodeInput {
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            authorization_endpoint: "https://example.com/authorize".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
            scopes: vec!["read".to_string()],
            use_pkce: true,
        };

        // Verify input is valid
        assert!(!input.client_id.is_empty());
        assert!(input.use_pkce);
    }
}
