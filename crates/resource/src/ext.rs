//! Typed resource access extension trait.
//!
//! Provides ergonomic `ctx.resource::<R>().await?` pattern for any context
//! implementing `HasResources`. This is the primary API for action/trigger
//! authors to acquire resources.

use std::future::Future;

use nebula_core::context::HasResources;

use crate::{
    Resource, ResourceGuard,
    error::{Error, ErrorKind},
};

/// Typed resource access for any context implementing `HasResources`.
///
/// # Examples
///
/// ```ignore
/// use nebula_resource::HasResourcesExt;
///
/// let pool = ctx.resource::<PostgresResource>().await?;
/// let cache = ctx.try_resource::<RedisResource>().await?;
/// ```
pub trait HasResourcesExt: HasResources {
    /// Acquire a typed resource guard. Returns error if not found or acquisition fails.
    fn resource<R: Resource>(&self) -> impl Future<Output = Result<ResourceGuard<R>, Error>> + Send
    where
        Self: Sized;

    /// Try to acquire a typed resource guard. Returns `Ok(None)` if the resource
    /// is not registered, `Err` if registered but acquisition fails.
    fn try_resource<R: Resource>(
        &self,
    ) -> impl Future<Output = Result<Option<ResourceGuard<R>>, Error>> + Send
    where
        Self: Sized;
}

impl<C: HasResources + ?Sized> HasResourcesExt for C {
    async fn resource<R: Resource>(&self) -> Result<ResourceGuard<R>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        let boxed = self.resources().acquire_any(&key).await.map_err(|e| {
            Error::new(ErrorKind::Permanent, e.to_string()).with_resource_key(key.clone())
        })?;

        boxed
            .downcast::<ResourceGuard<R>>()
            .map(|b| *b)
            .map_err(|_| {
                Error::new(
                    ErrorKind::Permanent,
                    format!(
                        "resource type mismatch: expected ResourceGuard<{}> for key `{key}`",
                        std::any::type_name::<R>(),
                    ),
                )
                .with_resource_key(key)
            })
    }

    async fn try_resource<R: Resource>(&self) -> Result<Option<ResourceGuard<R>>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        match self.resources().try_acquire_any(&key).await {
            Ok(Some(boxed)) => {
                let guard = boxed
                    .downcast::<ResourceGuard<R>>()
                    .map(|b| *b)
                    .map_err(|_| {
                        Error::new(
                            ErrorKind::Permanent,
                            format!(
                                "resource type mismatch: expected ResourceGuard<{}> for key `{key}`",
                                std::any::type_name::<R>(),
                            ),
                        )
                        .with_resource_key(key.clone())
                    })?;
                Ok(Some(guard))
            },
            Ok(None) => Ok(None),
            Err(e) => Err(Error::new(ErrorKind::Permanent, e.to_string()).with_resource_key(key)),
        }
    }
}
