use serde::Serialize;
use specta::Type;

#[derive(Debug, thiserror::Error, Type)]
pub enum AppError {
    #[error("Auth error: {0}")]
    Auth(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Keyring error: {0}")]
    Keyring(String),
    #[allow(dead_code)]
    #[error("Store error: {0}")]
    Store(String),
    #[allow(dead_code)]
    #[error("Window error: {0}")]
    Window(String),
    #[allow(dead_code)]
    #[error("Config error: {0}")]
    Config(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
