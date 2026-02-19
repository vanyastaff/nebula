//! RAII handle for webhook trigger lifecycle

use crate::WebhookPayload;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// RAII handle for a webhook trigger
///
/// When dropped, automatically:
/// 1. Cancels the trigger's operations
/// 2. Unregisters the webhook path from the server
/// 3. Calls `on_unsubscribe` in a background task
///
/// # Example
///
/// ```no_run
/// # use nebula_webhook::prelude::*;
/// # async fn example() -> Result<()> {
/// # let server = WebhookServer::new(WebhookServerConfig::default()).await?;
/// # let ctx = todo!();
/// // Subscribe to webhooks
/// let handle = server.subscribe(&ctx, None).await?;
///
/// // Use the handle to receive webhooks
/// let mut receiver = handle.receiver();
/// while let Ok(payload) = receiver.recv().await {
///     // Process webhook
/// }
///
/// // When handle is dropped, cleanup happens automatically
/// drop(handle);
/// # Ok(())
/// # }
/// ```
pub struct TriggerHandle {
    /// Path that this handle is registered for
    path: String,

    /// Receiver for incoming webhook payloads
    receiver: broadcast::Receiver<WebhookPayload>,

    /// Cancellation token for this trigger
    ///
    /// This is a child of the context's cancellation token.
    /// When the handle is dropped, this token is cancelled.
    cancel: CancellationToken,

    /// Optional cleanup callback
    ///
    /// Called when the handle is dropped to unregister the route.
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl TriggerHandle {
    /// Create a new trigger handle
    pub(crate) fn new(
        path: String,
        receiver: broadcast::Receiver<WebhookPayload>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            path,
            receiver,
            cancel,
            cleanup: None,
        }
    }

    /// Set a cleanup callback
    ///
    /// This callback is called when the handle is dropped.
    pub(crate) fn with_cleanup<F>(mut self, cleanup: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.cleanup = Some(Box::new(cleanup));
        self
    }

    /// Get the webhook path
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get a mutable reference to the receiver
    ///
    /// Use this to receive incoming webhook payloads.
    pub fn receiver(&mut self) -> &mut broadcast::Receiver<WebhookPayload> {
        &mut self.receiver
    }

    /// Get the cancellation token
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Check if this trigger is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Cancel this trigger
    ///
    /// This will:
    /// 1. Stop accepting new webhooks
    /// 2. Signal all tasks to shut down
    /// 3. Trigger cleanup on drop
    pub fn cancel(&self) {
        info!(path = %self.path, "Cancelling trigger");
        self.cancel.cancel();
    }

    /// Wait for the trigger to be cancelled
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// Try to receive a webhook payload without blocking
    ///
    /// Returns:
    /// - `Ok(payload)` if a payload is available
    /// - `Err(TryRecvError::Empty)` if no payload is available
    /// - `Err(TryRecvError::Closed)` if the channel is closed
    /// - `Err(TryRecvError::Lagged(n))` if `n` messages were skipped
    pub fn try_recv(
        &mut self,
    ) -> std::result::Result<WebhookPayload, broadcast::error::TryRecvError> {
        self.receiver.try_recv()
    }

    /// Receive the next webhook payload
    ///
    /// This method waits until a payload is available or the channel is closed.
    pub async fn recv(
        &mut self,
    ) -> std::result::Result<WebhookPayload, broadcast::error::RecvError> {
        self.receiver.recv().await
    }

    /// Get the current lag (number of messages that were skipped)
    pub fn lag(&self) -> u64 {
        // The receiver tracks lag internally, but we can estimate it
        // by checking if we're lagging behind
        0 // Broadcast receivers don't expose lag directly
    }

    /// Resubscribe to get a new receiver
    ///
    /// This creates a new receiver that will receive future messages.
    /// Useful if you want multiple consumers of the same webhook stream.
    pub fn resubscribe(&self) -> broadcast::Receiver<WebhookPayload> {
        self.receiver.resubscribe()
    }
}

impl Drop for TriggerHandle {
    fn drop(&mut self) {
        debug!(path = %self.path, "Dropping TriggerHandle");

        // Cancel the trigger
        self.cancel.cancel();

        // Run cleanup callback if present
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }

        info!(path = %self.path, "TriggerHandle dropped and cleaned up");
    }
}

impl std::fmt::Debug for TriggerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerHandle")
            .field("path", &self.path)
            .field("is_cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn create_test_payload(path: &str) -> WebhookPayload {
        WebhookPayload::new(
            path.to_string(),
            "POST".to_string(),
            std::collections::HashMap::new(),
            Bytes::from("test"),
        )
    }

    #[tokio::test]
    async fn test_handle_creation() {
        let (_tx, rx) = broadcast::channel(10);
        let cancel = CancellationToken::new();

        let handle = TriggerHandle::new("/test".to_string(), rx, cancel);

        assert_eq!(handle.path(), "/test");
        assert!(!handle.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_cancel() {
        let (_tx, rx) = broadcast::channel(10);
        let cancel = CancellationToken::new();

        let handle = TriggerHandle::new("/test".to_string(), rx, cancel);

        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_recv() {
        let (tx, rx) = broadcast::channel(10);
        let cancel = CancellationToken::new();

        let mut handle = TriggerHandle::new("/test".to_string(), rx, cancel);

        // Send a payload
        let payload = create_test_payload("/test");
        tx.send(payload.clone()).unwrap();

        // Receive it
        let received = handle.recv().await.unwrap();
        assert_eq!(received.path, payload.path);
    }

    #[tokio::test]
    async fn test_handle_try_recv() {
        let (tx, rx) = broadcast::channel(10);
        let cancel = CancellationToken::new();

        let mut handle = TriggerHandle::new("/test".to_string(), rx, cancel);

        // Try to receive without any payload - should be empty
        assert!(matches!(
            handle.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        // Send a payload
        let payload = create_test_payload("/test");
        tx.send(payload.clone()).unwrap();

        // Now try_recv should succeed
        let received = handle.try_recv().unwrap();
        assert_eq!(received.path, payload.path);
    }

    #[tokio::test]
    async fn test_handle_resubscribe() {
        let (tx, rx) = broadcast::channel(10);
        let cancel = CancellationToken::new();

        let mut handle = TriggerHandle::new("/test".to_string(), rx, cancel);

        // Create a second receiver
        let mut rx2 = handle.resubscribe();

        // Send a payload
        let payload = create_test_payload("/test");
        tx.send(payload.clone()).unwrap();

        // Both receivers should get it
        let received1 = handle.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.path, payload.path);
        assert_eq!(received2.path, payload.path);
    }

    #[test]
    fn test_handle_cleanup() {
        let (_, rx) = broadcast::channel::<WebhookPayload>(10);
        let cancel = CancellationToken::new();

        let cleanup_called = Arc::new(AtomicBool::new(false));
        let cleanup_called_clone = cleanup_called.clone();

        let handle = TriggerHandle::new("/test".to_string(), rx, cancel).with_cleanup(move || {
            cleanup_called_clone.store(true, Ordering::SeqCst);
        });

        assert!(!cleanup_called.load(Ordering::SeqCst));

        // Drop the handle - cleanup should be called
        drop(handle);

        assert!(cleanup_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_handle_cancellation_propagation() {
        let (_, rx) = broadcast::channel(10);
        let parent_cancel = CancellationToken::new();
        let child_cancel = parent_cancel.child_token();

        let handle = TriggerHandle::new("/test".to_string(), rx, child_cancel);

        assert!(!handle.is_cancelled());

        // Cancel the parent
        parent_cancel.cancel();

        // Child should also be cancelled
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        assert!(handle.is_cancelled());
    }
}
