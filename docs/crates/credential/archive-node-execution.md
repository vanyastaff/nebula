# Archived From "docs/archive/node-execution.md"

### nebula-credential
**Назначение:** Безопасное управление учетными данными с автоматической ротацией и шифрованием.

**Ключевые возможности:**
- Различные типы аутентификации
- Автоматическое обновление токенов
- Шифрование в памяти
- Audit trail

```rust
// Типы credentials
pub enum AuthData {
    ApiKey { key: SecretString },
    Bearer { token: SecretString },
    Basic { username: String, password: SecretString },
    OAuth2 { 
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        expires_at: Option<SystemTime>,
    },
    Certificate { cert: Vec<u8>, private_key: SecretBytes },
}

// OAuth2 с автоматическим refresh
pub struct OAuth2Credential {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    expires_at: Option<SystemTime>,
    auto_refresh: bool,
}

impl Credential for OAuth2Credential {
    async fn get_auth_data(&self, context: &CredentialContext) -> Result<AuthData> {
        // Автоматически обновляем если истек
        if let Some(expires) = self.expires_at {
            if SystemTime::now() > expires && self.auto_refresh {
                self.refresh().await?;
            }
        }
        Ok(self.build_auth_data())
    }
}

// Использование в Action
let slack_client = context
    .get_authenticated_client::<SlackClient>("slack_token")
    .await?;
```

---

## Execution Layer

