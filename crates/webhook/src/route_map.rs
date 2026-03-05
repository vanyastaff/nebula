//! Route mapping for webhook paths

use crate::{Error, Result, WebhookPayload};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Capacity for broadcast channels
const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Thread-safe map of webhook paths to event channels
///
/// Each registered path has a broadcast channel that distributes
/// incoming webhook payloads to all subscribers.
pub(crate) struct RouteMap {
    routes: DashMap<String, broadcast::Sender<WebhookPayload>>,
}

impl RouteMap {
    /// Create a new empty route map
    pub fn new() -> Self {
        Self {
            routes: DashMap::new(),
        }
    }

    /// Register a new webhook path
    ///
    /// Returns a receiver for incoming webhook payloads on this path.
    ///
    /// # Errors
    ///
    /// Returns `Error::RouteConflict` if the path is already registered.
    pub fn register(
        &self,
        path: impl Into<String>,
        capacity: Option<usize>,
    ) -> Result<broadcast::Receiver<WebhookPayload>> {
        let path = path.into();
        let capacity = capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);

        if self.routes.contains_key(&path) {
            return Err(Error::route_conflict(&path));
        }

        let (tx, rx) = broadcast::channel(capacity);
        debug!(path = %path, capacity = %capacity, "Registered webhook route");
        self.routes.insert(path, tx);
        Ok(rx)
    }

    /// Unregister a webhook path
    ///
    /// # Errors
    ///
    /// Returns `Error::RouteNotFound` if the path was not registered.
    pub fn unregister(&self, path: &str) -> Result<()> {
        if self.routes.remove(path).is_some() {
            debug!(path = %path, "Unregistered webhook route");
            Ok(())
        } else {
            Err(Error::route_not_found(path))
        }
    }

    /// Dispatch a webhook payload to the registered handler
    ///
    /// # Errors
    ///
    /// Returns `Error::RouteNotFound` if no handler is registered for this path.
    pub fn dispatch(&self, path: &str, payload: WebhookPayload) -> Result<()> {
        if let Some(sender) = self.routes.get(path) {
            // Broadcast to all receivers
            // We don't care if there are no active receivers - the webhook
            // might arrive before the trigger is fully initialized
            match sender.send(payload) {
                Ok(receiver_count) => {
                    debug!(
                        path = %path,
                        receiver_count = %receiver_count,
                        "Dispatched webhook to receivers"
                    );
                    Ok(())
                }
                Err(_) => {
                    // No receivers, but the route exists
                    warn!(path = %path, "No receivers for webhook");
                    Ok(())
                }
            }
        } else {
            Err(Error::route_not_found(path))
        }
    }

    /// Check if a path is registered
    pub fn contains(&self, path: &str) -> bool {
        self.routes.contains_key(path)
    }

    /// Get the number of registered routes
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Check if the route map is empty
    #[allow(dead_code)] // Used in tests
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Get all registered paths
    pub fn paths(&self) -> Vec<String> {
        self.routes
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clear all routes
    #[allow(dead_code)] // Reserved for cleanup operations
    pub fn clear(&self) {
        self.routes.clear();
        debug!("Cleared all webhook routes");
    }
}

impl Default for RouteMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared route map accessible across threads
pub(crate) type SharedRouteMap = Arc<RouteMap>;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn create_test_payload(path: &str) -> WebhookPayload {
        WebhookPayload::new(
            path.to_string(),
            "POST".to_string(),
            std::collections::HashMap::new(),
            Bytes::from("test"),
        )
    }

    #[test]
    fn test_register_and_unregister() {
        let routes = RouteMap::new();

        // Register a route
        let result = routes.register("/webhooks/test/123", None);
        assert!(result.is_ok());
        assert!(routes.contains("/webhooks/test/123"));

        // Unregister the route
        let result = routes.unregister("/webhooks/test/123");
        assert!(result.is_ok());
        assert!(!routes.contains("/webhooks/test/123"));
    }

    #[test]
    fn test_register_conflict() {
        let routes = RouteMap::new();

        // Register once
        let result = routes.register("/webhooks/test/123", None);
        assert!(result.is_ok());

        // Try to register again - should fail
        let result = routes.register("/webhooks/test/123", None);
        assert!(matches!(result, Err(Error::RouteConflict { .. })));
    }

    #[test]
    fn test_unregister_not_found() {
        let routes = RouteMap::new();

        let result = routes.unregister("/nonexistent");
        assert!(matches!(result, Err(Error::RouteNotFound { .. })));
    }

    #[tokio::test]
    async fn test_dispatch() {
        let routes = RouteMap::new();
        let path = "/webhooks/test/123";

        // Register and get receiver
        let mut rx = routes.register(path, None).unwrap();

        // Dispatch a payload
        let payload = create_test_payload(path);
        let result = routes.dispatch(path, payload.clone());
        assert!(result.is_ok());

        // Receiver should get the payload
        let received = rx.recv().await.unwrap();
        assert_eq!(received.path, payload.path);
    }

    #[test]
    fn test_dispatch_not_found() {
        let routes = RouteMap::new();

        let payload = create_test_payload("/nonexistent");
        let result = routes.dispatch("/nonexistent", payload);
        assert!(matches!(result, Err(Error::RouteNotFound { .. })));
    }

    #[tokio::test]
    async fn test_multiple_receivers() {
        let routes = RouteMap::new();
        let path = "/webhooks/test/123";

        // Register multiple receivers
        let mut rx1 = routes.register(path, None).unwrap();
        let sender = routes.routes.get(path).unwrap().clone();
        let mut rx2 = sender.subscribe();

        // Dispatch a payload
        let payload = create_test_payload(path);
        routes.dispatch(path, payload.clone()).unwrap();

        // Both receivers should get the payload
        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.path, payload.path);
        assert_eq!(received2.path, payload.path);
    }

    #[test]
    fn test_len_and_empty() {
        let routes = RouteMap::new();

        assert_eq!(routes.len(), 0);
        assert!(routes.is_empty());

        routes.register("/path1", None).unwrap();
        assert_eq!(routes.len(), 1);
        assert!(!routes.is_empty());

        routes.register("/path2", None).unwrap();
        assert_eq!(routes.len(), 2);

        routes.clear();
        assert_eq!(routes.len(), 0);
        assert!(routes.is_empty());
    }

    #[test]
    fn test_paths() {
        let routes = RouteMap::new();

        routes.register("/path1", None).unwrap();
        routes.register("/path2", None).unwrap();
        routes.register("/path3", None).unwrap();

        let paths = routes.paths();
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&"/path1".to_string()));
        assert!(paths.contains(&"/path2".to_string()));
        assert!(paths.contains(&"/path3".to_string()));
    }
}
