//! Quick OAuth2 test with hardcoded OAuth Playground credentials
//!
//! Just run: cargo run --example oauth2_quick_test

use nebula_credential::prelude::*;
use std::collections::HashMap;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║       OAuth2 Flow Tester (OAuth Playground)                  ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    // OAuth Playground credentials
    // Note: OAuth.com playground uses www.oauth.com as the authorization server
    let client_id = "google-id-123";
    let client_secret = "dummy-google-secret";
    let auth_endpoint = "https://oauth-mock.mock.beeceptor.com/oauth/authorize";
    let token_endpoint = "https://oauth-mock.mock.beeceptor.com/oauth/token/google";
    let redirect_uri = "http://localhost:8080/callback";

    println!("📋 OAuth Playground Configuration:");
    println!("  • Client ID: {}", client_id);
    println!("  • Client Secret: ***");
    println!("  • PKCE: enabled\n");

    // Step 1: Initialize OAuth2 flow
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🚀 Шаг 1: Генерация Authorization URL");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let input = AuthorizationCodeInput {
        client_id: client_id.to_string(),
        client_secret: Some(client_secret.to_string()),
        authorization_endpoint: auth_endpoint.to_string(),
        token_endpoint: token_endpoint.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scopes: vec!["photo".to_string(), "offline_access".to_string()],
        use_pkce: false,
    };

    let (partial_state, auth_url, state_param) = match credential.initialize(&input, &mut ctx).await? {
        InitializeResult::Pending {
            partial_state,
            next_step,
        } => {
            if let InteractionRequest::Redirect {
                url,
                validation_params,
                ..
            } = next_step
            {
                let state = validation_params.get("state").unwrap().clone();
                (partial_state, url, state)
            } else {
                return Err("Expected Redirect".into());
            }
        }
        _ => return Err("Expected Pending result".into()),
    };

    println!("✅ Authorization URL сгенерирован!\n");
    println!("🌐 URL для авторизации:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("{}", auth_url);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    if auth_url.contains("code_challenge") {
        println!("✓ PKCE enabled (code_challenge present)");
        println!("✓ code_challenge_method=S256\n");
    }

    // Step 2: User authorization
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("👤 Шаг 2: Авторизация пользователя");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("📝 Инструкция:");
    println!("  1. Скопируй URL выше");
    println!("  2. Открой в браузере");
    println!("  3. На странице OAuth Playground:");
    println!("     Login: hilarious-hawk@example.com");
    println!("     Password: Tame-Turkey-85");
    println!("  4. Нажми 'Authorize'");
    println!("  5. Скопируй redirect URL (или только параметр 'code')\n");

    print!("Вставь redirect URL (или code): ");
    io::stdout().flush()?;

    let mut callback_input = String::new();
    io::stdin().read_line(&mut callback_input)?;
    let callback_input = callback_input.trim();

    // Parse code and state
    let (code, received_state) = parse_callback(callback_input)?;

    println!("\n✅ Получен code: {}...", &code[..20.min(code.len())]);
    println!("✅ State: {}", received_state);

    // Verify state
    if received_state != state_param {
        println!("\n❌ ОШИБКА: State mismatch!");
        println!("   Expected: {}", state_param);
        println!("   Received: {}", received_state);
        return Err("State mismatch - possible CSRF attack".into());
    }

    println!("✅ State verified\n");

    // Step 3: Exchange code for token
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔄 Шаг 3: Обмен code на access token");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let mut params = HashMap::new();
    params.insert("code".to_string(), code);
    params.insert("state".to_string(), received_state);

    let user_input = UserInput::Callback { params };

    println!("🔄 Отправляем запрос на token endpoint...");
    println!("   Endpoint: {}\n", token_endpoint);

    match credential
        .continue_initialization(partial_state, user_input, &mut ctx)
        .await
    {
        Ok(InitializeResult::Complete(state)) => {
            println!("🎉 Успех! Access token получен!\n");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("📊 Token Information:");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  • Token Type: {}", state.token_type);
            println!("  • Expires At: {} (unix timestamp)", state.expires_at);
            println!("  • Has Refresh Token: {}", state.refresh_token.is_some());

            let token_preview = state.access_token.expose();
            let preview_len = 40.min(token_preview.len());
            println!("  • Access Token: {}...", &token_preview[..preview_len]);

            println!("\n✅ OAuth2 Authorization Code flow completed successfully! 🚀");
        }
        Ok(other) => {
            println!("❌ Unexpected result: {:?}", other);
        }
        Err(e) => {
            println!("❌ Ошибка при обмене code на token:");
            println!("   {}\n", e);

            println!("💡 Возможные причины:");
            println!("  • Code уже был использован (можно использовать только 1 раз)");
            println!("  • Неверный authorization code");
            println!("  • Неправильный redirect_uri");
            println!("  • Истек срок действия code (обычно 10 минут)");
            println!("  • Неверный client_secret");
            println!("\n💡 Решение: Заново сгенерируй authorization URL и повтори процесс");
        }
    }

    Ok(())
}

fn parse_callback(input: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    // Handle both full URL and just parameters
    let query_string = if input.contains('?') {
        input.split('?').nth(1).unwrap_or(input)
    } else if input.contains('=') {
        input
    } else {
        // Just the code value
        return Err("Expected URL with code parameter or 'code=...' format".into());
    };

    let mut code = None;
    let mut state = None;

    for pair in query_string.split('&') {
        let mut parts = pair.split('=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");

        match key {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            _ => {}
        }
    }

    let code = code.ok_or("Missing 'code' parameter in callback")?;
    let state = state.ok_or("Missing 'state' parameter in callback")?;

    Ok((code, state))
}
