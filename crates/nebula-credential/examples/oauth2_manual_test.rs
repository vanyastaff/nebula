//! Manual OAuth2 testing guide for oauth.com/playground
//!
//! This example shows step-by-step how to test our OAuth2 implementation
//! with real OAuth2 providers

use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║   OAuth2 Manual Testing with oauth.com/playground         ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    print_instructions();

    // Test different flows
    let flow = std::env::var("OAUTH_FLOW").unwrap_or_else(|_| "auth_code".to_string());

    match flow.as_str() {
        "auth_code" => demo_authorization_code().await?,
        "pkce" => demo_pkce_flow().await?,
        "client_creds" => demo_client_credentials().await?,
        _ => {
            println!("Unknown flow: {}", flow);
            println!("Use: auth_code, pkce, or client_creds");
        }
    }

    Ok(())
}

fn print_instructions() {
    println!("📋 Testing Instructions:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("1️⃣  Go to: https://www.oauth.com/playground/");
    println!("2️⃣  Choose one of:");
    println!("    • Authorization Code");
    println!("    • PKCE");
    println!("    • Client Credentials");
    println!("    • Device Code");
    println!("    • OpenID Connect\n");

    println!("3️⃣  Copy the generated values:");
    println!("    • Client ID");
    println!("    • Client Secret");
    println!("    • Authorization Endpoint");
    println!("    • Token Endpoint\n");

    println!("4️⃣  Set environment variables:");
    println!("    export OAUTH_CLIENT_ID=\"your_client_id\"");
    println!("    export OAUTH_CLIENT_SECRET=\"your_secret\"");
    println!("    export OAUTH_FLOW=\"auth_code\"  # or pkce, client_creds\n");

    println!("5️⃣  Run this example:");
    println!("    cargo run --example oauth2_manual_test\n");

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
}

async fn demo_authorization_code() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Testing: Authorization Code Flow\n");

    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let input = AuthorizationCodeInput {
        client_id: std::env::var("OAUTH_CLIENT_ID")?,
        client_secret: std::env::var("OAUTH_CLIENT_SECRET").ok(),
        authorization_endpoint: std::env::var("OAUTH_AUTH_ENDPOINT")
            .unwrap_or_else(|_| "https://www.oauth.com/playground/authorize.html".to_string()),
        token_endpoint: std::env::var("OAUTH_TOKEN_ENDPOINT")
            .unwrap_or_else(|_| "https://www.oauth.com/playground/access-token.html".to_string()),
        redirect_uri: std::env::var("OAUTH_REDIRECT_URI")
            .unwrap_or_else(|_| "http://localhost:8080/callback".to_string()),
        scopes: vec!["read".to_string(), "write".to_string()],
        use_pkce: false,
    };

    println!("📝 Configuration:");
    println!("  Client ID: {}", input.client_id);
    println!("  Auth Endpoint: {}", input.authorization_endpoint);
    println!("  Token Endpoint: {}", input.token_endpoint);
    println!("  Redirect URI: {}", input.redirect_uri);
    println!("  Scopes: {:?}\n", input.scopes);

    match credential.initialize(&input, &mut ctx).await? {
        InitializeResult::Pending {
            partial_state,
            next_step,
        } => {
            println!("✅ Step 1: Authorization URL generated\n");

            if let InteractionRequest::Redirect {
                url,
                validation_params,
                ..
            } = next_step
            {
                println!("🌐 Authorization URL:");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("{}", url);
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

                println!("📌 State parameter: {:?}\n", validation_params);

                println!("👉 Next steps:");
                println!("  1. Open the URL above in your browser");
                println!("  2. Authorize the application");
                println!("  3. Copy the 'code' parameter from the redirect URL");
                println!("  4. Call continue_initialization with the code\n");

                // Simulate continuation (in real app, this comes from callback)
                println!("💡 Example code to continue:");
                println!("   let mut params = HashMap::new();");
                println!(
                    "   params.insert(\"code\".to_string(), \"<authorization_code>\".to_string());"
                );
                println!("   params.insert(\"state\".to_string(), \"<state_value>\".to_string());");
                println!("   ");
                println!("   credential.continue_initialization(");
                println!("       partial_state,");
                println!("       UserInput::Callback {{ params }},");
                println!("       &mut ctx");
                println!("   ).await?;\n");

                // Save partial state for manual testing
                println!("🔖 Partial state saved (use for testing continuation)");
                println!("   Step: {}", partial_state.step);
                println!("   Created: {}", partial_state.created_at);
                println!("   TTL: {:?} seconds\n", partial_state.ttl_seconds);
            }
        }
        other => {
            println!("❌ Unexpected result: {:?}", other);
        }
    }

    Ok(())
}

async fn demo_pkce_flow() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Testing: PKCE (Proof Key for Code Exchange)\n");

    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let input = AuthorizationCodeInput {
        client_id: std::env::var("OAUTH_CLIENT_ID")?,
        client_secret: None, // PKCE doesn't require client secret
        authorization_endpoint: std::env::var("OAUTH_AUTH_ENDPOINT")
            .unwrap_or_else(|_| "https://www.oauth.com/playground/authorize.html".to_string()),
        token_endpoint: std::env::var("OAUTH_TOKEN_ENDPOINT")
            .unwrap_or_else(|_| "https://www.oauth.com/playground/access-token.html".to_string()),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["openid".to_string(), "profile".to_string()],
        use_pkce: true, // Enable PKCE
    };

    println!("📝 Configuration:");
    println!("  Client ID: {}", input.client_id);
    println!("  PKCE Enabled: {}", input.use_pkce);
    println!("  Client Secret: None (PKCE flow)\n");

    match credential.initialize(&input, &mut ctx).await? {
        InitializeResult::Pending { next_step, .. } => {
            if let InteractionRequest::Redirect { url, .. } = next_step {
                println!("✅ PKCE flow initiated\n");
                println!("🔑 The URL includes:");
                println!("  • code_challenge");
                println!("  • code_challenge_method=S256\n");

                println!("🌐 Authorization URL:");
                println!("{}\n", url);

                // Verify PKCE parameters
                if url.contains("code_challenge") && url.contains("code_challenge_method") {
                    println!("✅ PKCE parameters present in URL");
                } else {
                    println!("❌ PKCE parameters missing!");
                }
            }
        }
        other => {
            println!("❌ Unexpected result: {:?}", other);
        }
    }

    Ok(())
}

async fn demo_client_credentials() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Testing: Client Credentials Flow\n");
    println!("⚠️  Note: This flow is for server-to-server authentication");
    println!("    OAuth Playground may not support this flow directly\n");

    let credential = OAuth2ClientCredentials::create();
    let mut ctx = CredentialContext::new();

    let input = ClientCredentialsInput {
        client_id: std::env::var("OAUTH_CLIENT_ID")?,
        client_secret: std::env::var("OAUTH_CLIENT_SECRET")?,
        token_endpoint: std::env::var("OAUTH_TOKEN_ENDPOINT")
            .unwrap_or_else(|_| "https://www.oauth.com/playground/access-token.html".to_string()),
        scopes: vec!["api".to_string()],
    };

    println!("📝 Making token request...");

    match credential.initialize(&input, &mut ctx).await {
        Ok(InitializeResult::Complete(state)) => {
            println!("✅ Token acquired!\n");
            println!("  Token Type: {}", state.token_type);
            println!("  Expires At: {} (unix timestamp)", state.expires_at);
            println!("  Has Refresh Token: {}", state.refresh_token.is_some());
        }
        Err(e) => {
            println!("❌ Failed: {}\n", e);
            println!("💡 This is expected if:");
            println!("  • OAuth Playground doesn't support client credentials");
            println!("  • Credentials are invalid");
            println!("  • Token endpoint is incorrect");
        }
        other => {
            println!("❌ Unexpected result: {:?}", other);
        }
    }

    Ok(())
}
