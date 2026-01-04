# OAuth2 Testing Examples

Примеры для тестирования OAuth2 flows с реальными провайдерами.

## Быстрый старт с OAuth Playground

### 1. Открой OAuth Playground
Перейди на https://www.oauth.com/playground/

### 2. Выбери flow
- **Authorization Code** - стандартный OAuth2 flow с browser redirect
- **PKCE** - Authorization Code с Proof Key for Code Exchange (для мобильных/SPA)
- **Client Credentials** - server-to-server authentication
- **Device Code** - для устройств без браузера
- **OpenID Connect** - аутентификация пользователя

### 3. Получи credentials

После выбора flow, OAuth Playground сгенерирует:
- `Client ID` - публичный идентификатор
- `Client Secret` - секретный ключ (если требуется)
- `Authorization Endpoint` - URL для получения кода
- `Token Endpoint` - URL для обмена кода на токен

### 4. Запусти пример

#### Windows (PowerShell):
```powershell
$env:OAUTH_CLIENT_ID="your_client_id_from_playground"
$env:OAUTH_CLIENT_SECRET="your_client_secret"
$env:OAUTH_FLOW="auth_code"

cargo run --example oauth2_manual_test
```

#### Linux/Mac:
```bash
export OAUTH_CLIENT_ID="your_client_id_from_playground"
export OAUTH_CLIENT_SECRET="your_client_secret"
export OAUTH_FLOW="auth_code"

cargo run --example oauth2_manual_test
```

## Доступные flows

### Authorization Code (`auth_code`)
Стандартный OAuth2 flow для веб-приложений:

```bash
export OAUTH_FLOW="auth_code"
cargo run --example oauth2_manual_test
```

**Шаги:**
1. Программа выведет authorization URL
2. Открой URL в браузере
3. Авторизуйся
4. Скопируй `code` из redirect URL
5. Используй code для получения токена

### PKCE (`pkce`)
Authorization Code с дополнительной защитой:

```bash
export OAUTH_FLOW="pkce"
cargo run --example oauth2_manual_test
```

**Отличия:**
- Не требует `client_secret`
- Использует `code_challenge` и `code_verifier`
- Безопасен для публичных клиентов (мобильные приложения, SPA)

### Client Credentials (`client_creds`)
Server-to-server authentication:

```bash
export OAUTH_FLOW="client_creds"
cargo run --example oauth2_manual_test
```

**Примечание:** OAuth Playground может не поддерживать этот flow напрямую.

## Примеры кода

### Базовое использование

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = OAuth2AuthorizationCode::new();
    let mut ctx = CredentialContext::new();

    let input = AuthorizationCodeInput {
        client_id: "your_client_id".to_string(),
        client_secret: Some("your_secret".to_string()),
        authorization_endpoint: "https://auth.provider.com/authorize".to_string(),
        token_endpoint: "https://auth.provider.com/token".to_string(),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
        use_pkce: true,
    };

    match credential.initialize(&input, &mut ctx).await? {
        InitializeResult::Pending { partial_state, next_step } => {
            if let InteractionRequest::Redirect { url, .. } = next_step {
                println!("Open this URL: {}", url);
                // Redirect user, then continue with callback code
            }
        }
        _ => {}
    }

    Ok(())
}
```

### PKCE Flow

```rust
let input = AuthorizationCodeInput {
    client_id: "public_client_id".to_string(),
    client_secret: None, // PKCE doesn't need secret
    use_pkce: true,      // Enable PKCE
    // ... other fields
};
```

### Client Credentials

```rust
use nebula_credential::prelude::*;

let credential = OAuth2ClientCredentials::create();
let mut ctx = CredentialContext::new();

let input = ClientCredentialsInput {
    client_id: "service_account_id".to_string(),
    client_secret: "service_account_secret".to_string(),
    token_endpoint: "https://auth.provider.com/token".to_string(),
    scopes: vec!["api".to_string()],
};

match credential.initialize(&input, &mut ctx).await? {
    InitializeResult::Complete(state) => {
        println!("Access token acquired!");
        println!("Expires at: {}", state.expires_at);
    }
    _ => {}
}
```

## Тестирование с реальными провайдерами

### GitHub OAuth

```bash
export OAUTH_CLIENT_ID="your_github_app_client_id"
export OAUTH_CLIENT_SECRET="your_github_app_client_secret"
export OAUTH_AUTH_ENDPOINT="https://github.com/login/oauth/authorize"
export OAUTH_TOKEN_ENDPOINT="https://github.com/login/oauth/access_token"
export OAUTH_REDIRECT_URI="http://localhost:8080/callback"

cargo run --example oauth2_manual_test
```

### Google OAuth

```bash
export OAUTH_CLIENT_ID="your_google_client_id"
export OAUTH_CLIENT_SECRET="your_google_client_secret"
export OAUTH_AUTH_ENDPOINT="https://accounts.google.com/o/oauth2/v2/auth"
export OAUTH_TOKEN_ENDPOINT="https://oauth2.googleapis.com/token"
export OAUTH_REDIRECT_URI="http://localhost:8080/callback"

cargo run --example oauth2_manual_test
```

## Структура примеров

- `oauth2_playground.rs` - Базовый пример с OAuth Playground
- `oauth2_manual_test.rs` - Детальное пошаговое тестирование
- `README.md` - Эта документация

## Troubleshooting

### "Error: NotPresent"
Не заданы environment variables. Убедись что экспортировал `OAUTH_CLIENT_ID`.

### "Invalid redirect_uri"
Redirect URI в коде должен совпадать с настройками OAuth приложения.

### "State mismatch"
Параметр `state` не совпадает. Используй тот же `state` из authorization URL.

### PKCE errors
Убедись что OAuth провайдер поддерживает PKCE (RFC 7636).

## Полезные ссылки

- [OAuth 2.0 Playground](https://www.oauth.com/playground/)
- [OAuth 2.0 RFC 6749](https://datatracker.ietf.org/doc/html/rfc6749)
- [PKCE RFC 7636](https://datatracker.ietf.org/doc/html/rfc7636)
- [OpenID Connect](https://openid.net/connect/)
