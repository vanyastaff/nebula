use crate::error::AppError;

const SERVICE: &str = "nebula-desktop";
const ACCESS_TOKEN_USER: &str = "access_token";
const REFRESH_TOKEN_USER: &str = "refresh_token";

fn entry(user: &str) -> Result<keyring::Entry, AppError> {
    keyring::Entry::new(SERVICE, user).map_err(|e| AppError::Keyring(e.to_string()))
}

/// Stores access (and optionally refresh) tokens in the OS keyring.
pub async fn store_tokens(access: &str, refresh: Option<&str>) -> Result<(), AppError> {
    let access = access.to_owned();
    let refresh = refresh.map(|s| s.to_owned());

    tokio::task::spawn_blocking(move || {
        entry(ACCESS_TOKEN_USER)?
            .set_password(&access)
            .map_err(|e| AppError::Keyring(e.to_string()))?;

        if let Some(ref token) = refresh {
            entry(REFRESH_TOKEN_USER)?
                .set_password(token)
                .map_err(|e| AppError::Keyring(e.to_string()))?;
        }

        Ok(())
    })
    .await
    .map_err(|e| AppError::Keyring(e.to_string()))?
}

/// Retrieves the access token from the OS keyring.
pub async fn get_access_token() -> Result<String, AppError> {
    tokio::task::spawn_blocking(|| {
        entry(ACCESS_TOKEN_USER)?
            .get_password()
            .map_err(|e| AppError::Keyring(e.to_string()))
    })
    .await
    .map_err(|e| AppError::Keyring(e.to_string()))?
}

/// Retrieves the refresh token from the OS keyring, if stored.
pub async fn get_refresh_token() -> Result<Option<String>, AppError> {
    tokio::task::spawn_blocking(|| {
        match entry(REFRESH_TOKEN_USER)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::Keyring(e.to_string())),
        }
    })
    .await
    .map_err(|e| AppError::Keyring(e.to_string()))?
}

/// Deletes all stored tokens from the OS keyring.
pub async fn delete_tokens() -> Result<(), AppError> {
    tokio::task::spawn_blocking(|| {
        // Delete access token
        match entry(ACCESS_TOKEN_USER)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AppError::Keyring(e.to_string())),
        }

        // Delete refresh token
        match entry(REFRESH_TOKEN_USER)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AppError::Keyring(e.to_string())),
        }

        Ok(())
    })
    .await
    .map_err(|e| AppError::Keyring(e.to_string()))?
}
