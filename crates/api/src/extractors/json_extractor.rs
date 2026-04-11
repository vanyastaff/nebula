//! Validated JSON Extractor
//!
//! JSON extractor с встроенной валидацией.

use axum::{
    Json,
    extract::{FromRequest, Request},
};
use nebula_validator::foundation::Validate as NebulaValidate;
use serde::de::DeserializeOwned;

use crate::errors::ApiError;

/// JSON extractor с валидацией
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + NebulaValidate<T>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|err| ApiError::validation_message(format!("Invalid JSON: {}", err)))?;

        NebulaValidate::validate(&value, &value).map_err(ApiError::from)?;

        Ok(ValidatedJson(value))
    }
}
