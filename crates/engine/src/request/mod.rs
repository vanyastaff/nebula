//! Request module for creating and executing HTTP requests
//! Provides a flexible and convenient API for working with HTTP

mod auth;
mod client;
mod error;
mod method;
mod options;
mod protocol;
mod proxy;
mod response;

pub use auth::RequestAuth;
pub use error::RequestError;
pub use method::RequestMethod;
pub use options::{DEFAULT_TIMEOUT, RequestOptions, RequestOptionsBuilder};
pub use protocol::RequestProtocol;
pub use proxy::RequestProxy;
pub use response::{Response, ResponseError};

/// Main trait for components that can be applied to a request
pub trait ApplyToRequest {
    /// Applies the component to request options
    fn apply_to_options(&self, _options: &mut RequestOptions) -> Result<(), RequestError> {
        Ok(())
    }

    /// Checks compatibility with another component
    fn is_compatible_with<T: ApplyToRequest + ?Sized>(&self, _other: &T) -> bool {
        true
    }
}
