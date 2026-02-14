use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;

/// Metadata about a stream and its current position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamMetadata {
    /// Unique stream identifier.
    pub stream_id: String,
    /// Sequential item index within this stream (0-based).
    pub sequence: u64,
    /// Whether this is the final item in the stream.
    pub is_last: bool,
    /// Total number of items, if known in advance.
    pub total_items: Option<u64>,
}

/// A single item emitted by a streaming action.
#[derive(Debug, Clone)]
pub struct StreamItem<T> {
    /// The item payload.
    pub data: T,
    /// Stream position metadata.
    pub metadata: StreamMetadata,
}

/// Action that produces a continuous stream of items.
///
/// Used for processing large datasets, real-time feeds, log tailing,
/// and any scenario where data arrives incrementally.
///
/// The engine calls `next_item` repeatedly until it returns `None`
/// (stream exhausted) or an error occurs. Backpressure is managed
/// by the engine — it waits for downstream processing before requesting
/// the next item.
///
/// # Type Parameters
///
/// - `Config`: stream configuration (e.g. source URL, filters).
/// - `Item`: the type of each streamed item.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
/// use nebula_action::streaming::*;
/// use async_trait::async_trait;
///
/// struct CsvReader {
///     meta: ActionMetadata,
/// }
///
/// #[async_trait]
/// impl StreamingAction for CsvReader {
///     type Config = serde_json::Value;
///     type Item = serde_json::Value;
///
///     async fn open_stream(
///         &self, _config: &Self::Config, _ctx: &ActionContext,
///     ) -> Result<(), ActionError> {
///         Ok(())
///     }
///
///     async fn next_item(
///         &self, _config: &Self::Config, ctx: &ActionContext,
///     ) -> Result<Option<StreamItem<Self::Item>>, ActionError> {
///         ctx.check_cancelled()?;
///         // Return None when stream is exhausted
///         Ok(None)
///     }
///
///     async fn close_stream(
///         &self, _config: &Self::Config, _ctx: &ActionContext,
///     ) -> Result<(), ActionError> {
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait StreamingAction: Action {
    /// Stream configuration type.
    type Config: Send + Sync + 'static;
    /// Type of each streamed item.
    type Item: Send + Sync + 'static;

    /// Initialize the stream (open connections, seek to position, etc.).
    ///
    /// Called once before the first `next_item` call.
    async fn open_stream(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;

    /// Produce the next item from the stream.
    ///
    /// Returns `Ok(None)` when the stream is exhausted.
    /// The engine manages backpressure — it will not call `next_item`
    /// again until downstream has consumed the previous item.
    async fn next_item(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<Option<StreamItem<Self::Item>>, ActionError>;

    /// Clean up the stream (close connections, release resources).
    ///
    /// Called after the stream ends (either naturally or due to error/cancellation).
    /// Implementations should be idempotent.
    async fn close_stream(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        let _ = (config, ctx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_metadata_construction() {
        let meta = StreamMetadata {
            stream_id: "csv-import-1".into(),
            sequence: 42,
            is_last: false,
            total_items: Some(1000),
        };
        assert_eq!(meta.sequence, 42);
        assert!(!meta.is_last);
        assert_eq!(meta.total_items, Some(1000));
    }

    #[test]
    fn stream_item_construction() {
        let item = StreamItem {
            data: serde_json::json!({"row": 1}),
            metadata: StreamMetadata {
                stream_id: "test".into(),
                sequence: 0,
                is_last: true,
                total_items: Some(1),
            },
        };
        assert!(item.metadata.is_last);
    }

    #[test]
    fn stream_metadata_serialization() {
        let meta = StreamMetadata {
            stream_id: "s1".into(),
            sequence: 5,
            is_last: false,
            total_items: None,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["stream_id"], "s1");
        assert_eq!(json["sequence"], 5);
    }
}
