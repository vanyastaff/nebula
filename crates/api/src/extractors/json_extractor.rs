//! Validated JSON Extractor
//!
//! JSON extractor с встроенной валидацией.

use crate::errors::ApiError;
use axum::{
    extract::{FromRequest, Request},
    Json,
};
use serde::de::DeserializeOwned;

/// JSON extractor с валидацией
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|err| ApiError::Validation(format!("Invalid JSON: {}", err)))?;

        value
            .validate()
            .map_err(ApiError::Validation)?;

        Ok(ValidatedJson(value))
    }
}

/// Trait для валидации типов
pub trait Validate {
    /// Validate the value
    fn validate(&self) -> Result<(), String>;
}


