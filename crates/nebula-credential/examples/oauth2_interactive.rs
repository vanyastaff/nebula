//! Interactive OAuth2 Authorization Code flow tester
//!
//! This example walks you through the complete OAuth2 flow step by step

use nebula_credential::prelude::*;
use std::collections::HashMap;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  Interactive OAuth2 Authorization Code Flow Tester           ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    // Example URL from OAuth Playground:
    // https://authorization-server.com/authorize?
    //   response_type=code
    //   &client_id=AxIKkzEyzIqNUvLUvftnL57O
    //   &redirect_uri=https://www.oauth.com/playground/authorization-code.html
    //   &scope=photo+offline_access
    //   &state=gNYGdoXUeHwh8sAJ

    println!("Давай протестируем OAuth2 flow! 🚀\n");

    // Step 1: Get configuration
    let config = get_oauth_config()?;

    // Step 2: Initialize flow and get authorization URL
    let (partial_state, auth_url, state_param) = generate_authorization_url(&config).await?;

    // Step 3: User authorizes
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📋 Шаг 2: Авторизация");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("🌐 Authorization URL:\n");
    println!("{}\n", auth_url);

    println!("👉 Действия:");
    println!("  1. Скопируй URL выше");
    println!("  2. Открой в браузере");
    println!("  3. Нажми 'Authorize' на странице OAuth Playground");
    println!("  4. Тебя редиректнет обратно с параметром 'code'\n");

    // Step 4: Get authorization code from user
    print!("Введи полный redirect URL (или просто 'code' параметр): ");
    io::stdout().flush()?;

    let mut callback_input = String::new();
    io::stdin().read_line(&mut callback_input)?;
    let callback_input = callback_input.trim();

    // Parse code and state from input
    let (code, received_state) = parse_callback(callback_input)?;

    let code_preview = &code[..20.min(code.len())];
    println!("\n✅ Получен authorization code: {}...", code_preview);
    println!("✅ State parameter: {}", received_state);

    // Verify state
    if received_state != state_param {
        println!("\n❌ ОШИБКА: State параметр не совпадает!");
        println!("   Ожидался: {}", state_param);
        println!("   Получен:  {}", received_state);
        return Err("State mismatch - возможная CSRF атака".into());
    }

    println!("✅ State проверен успешно\n");

    // Step 5: Exchange code for token
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔄 Шаг 3: Обмен code на access token");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let token_result =
        exchange_code_for_token(&config, partial_state, code.clone(), received_state.clone()).await;

    match token_result {
        Ok(state) => {
            println!("🎉 Успех! Access token получен!\n");
            println!("📊 Token информация:");
            println!("  • Token Type: {}", state.token_type);
            println!("  • Expires At: {} (unix timestamp)", state.expires_at);
            println!("  • Has Refresh Token: {}", state.refresh_token.is_some());

            let access_token_preview = state.access_token.expose();
            let preview_len = 30.min(access_token_preview.len());
            println!(
                "  • Access Token: {}...",
                &access_token_preview[..preview_len]
            );

            println!("\n✅ OAuth2 flow завершен успешно!");
        }
        Err(e) => {
            println!("❌ Ошибка при обмене code на token:");
            println!("   {}\n", e);

            println!("💡 Возможные причины:");
            println!("  • Неверный authorization code");
            println!("  • Code уже был использован");
            println!("  • Неправильный redirect_uri");
            println!("  • Истек срок действия code");
            println!("  • Неверный client_secret");
        }
    }

    Ok(())
}

struct OAuthConfig {
    client_id: String,
    client_secret: Option<String>,
    auth_endpoint: String,
    token_endpoint: String,
    redirect_uri: String,
    scopes: Vec<String>,
    use_pkce: bool,
}

fn get_oauth_config() -> Result<OAuthConfig, Box<dyn std::error::Error>> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📋 Шаг 1: Конфигурация OAuth2");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    println!("💡 Инструкция:");
    println!("   1. Открой https://www.oauth.com/playground/");
    println!("   2. Выбери 'Authorization Code' или 'PKCE'");
    println!("   3. Playground покажет тебе Client ID и Client Secret\n");

    // Prompt for flow type
    println!("Какой flow хочешь использовать?");
    println!("  1) Authorization Code (с client secret)");
    println!("  2) PKCE (без client secret, безопаснее)\n");

    print!("Выбери (1 или 2): ");
    io::stdout().flush()?;
    let mut flow_choice = String::new();
    io::stdin().read_line(&mut flow_choice)?;
    let use_pkce = flow_choice.trim() == "2";

    println!();

    // Get Client ID
    print!("📝 Client ID (из OAuth Playground): ");
    io::stdout().flush()?;
    let mut client_id = String::new();
    io::stdin().read_line(&mut client_id)?;
    let client_id = client_id.trim().to_string();

    if client_id.is_empty() {
        return Err("Client ID обязателен".into());
    }

    // Get Client Secret (optional for PKCE)
    let client_secret = if use_pkce {
        println!("✓ PKCE режим - Client Secret не требуется");
        None
    } else {
        print!("🔑 Client Secret: ");
        io::stdout().flush()?;
        let mut secret = String::new();
        io::stdin().read_line(&mut secret)?;
        let secret = secret.trim();

        if secret.is_empty() {
            None
        } else {
            Some(secret.to_string())
        }
    };

    println!("\n✅ Конфигурация:");
    println!(
        "  • Flow Type: {}",
        if use_pkce {
            "PKCE"
        } else {
            "Authorization Code"
        }
    );
    println!("  • Client ID: {}", client_id);
    println!(
        "  • Client Secret: {}",
        if client_secret.is_some() {
            "***"
        } else {
            "None"
        }
    );

    Ok(OAuthConfig {
        client_id,
        client_secret,
        auth_endpoint: "https://authorization-server.com/authorize".to_string(),
        token_endpoint: "https://authorization-server.com/token".to_string(),
        redirect_uri: "https://www.oauth.com/playground/authorization-code.html".to_string(),
        scopes: vec!["photo".to_string(), "offline_access".to_string()],
        use_pkce,
    })
}

async fn generate_authorization_url(
    config: &OAuthConfig,
) -> Result<(PartialState, String, String), Box<dyn std::error::Error>> {
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔗 Шаг 1: Генерация Authorization URL");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let input = AuthorizationCodeInput {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        authorization_endpoint: config.auth_endpoint.clone(),
        token_endpoint: config.token_endpoint.clone(),
        redirect_uri: config.redirect_uri.clone(),
        scopes: config.scopes.clone(),
        use_pkce: config.use_pkce,
    };

    match credential.initialize(&input, &mut ctx).await? {
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
                let state_param = validation_params
                    .get("state")
                    .expect("State parameter missing")
                    .clone();

                println!("✅ Authorization URL сгенерирован");

                if config.use_pkce {
                    println!("🔐 PKCE включен:");
                    if url.contains("code_challenge") {
                        println!("   ✓ code_challenge присутствует");
                        println!("   ✓ code_challenge_method=S256");
                    }
                }

                Ok((partial_state, url, state_param))
            } else {
                Err("Unexpected interaction type".into())
            }
        }
        _ => Err("Expected Pending result".into()),
    }
}

fn parse_callback(input: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    // Handle both full URL and just parameters
    let query_string = if input.contains("?") {
        input.split('?').nth(1).unwrap_or(input)
    } else {
        input
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

    let code = code.ok_or("Missing 'code' parameter")?;
    let state = state.ok_or("Missing 'state' parameter")?;

    Ok((code, state))
}

async fn exchange_code_for_token(
    config: &OAuthConfig,
    partial_state: PartialState,
    code: String,
    state: String,
) -> Result<OAuth2State, Box<dyn std::error::Error>> {
    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let mut params = HashMap::new();
    params.insert("code".to_string(), code);
    params.insert("state".to_string(), state);

    let user_input = UserInput::Callback { params };

    println!("🔄 Обмениваем code на token...");
    println!("   Endpoint: {}\n", config.token_endpoint);

    match credential
        .continue_initialization(partial_state, user_input, &mut ctx)
        .await?
    {
        InitializeResult::Complete(state) => Ok(state),
        other => Err(format!("Unexpected result: {:?}", other).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_url() {
        let url =
            "https://www.oauth.com/playground/authorization-code.html?code=ABC123&state=XYZ789";
        let (code, state) = parse_callback(url).unwrap();
        assert_eq!(code, "ABC123");
        assert_eq!(state, "XYZ789");
    }

    #[test]
    fn test_parse_params_only() {
        let params = "code=ABC123&state=XYZ789";
        let (code, state) = parse_callback(params).unwrap();
        assert_eq!(code, "ABC123");
        assert_eq!(state, "XYZ789");
    }
}
