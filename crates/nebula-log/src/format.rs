//! Format utilities (time)

use tracing_subscriber::fmt::time::SystemTime;

/// Create timer based on optional format string.
/// For now, we'll use [`SystemTime`] as the base timer since custom timers cause type issues.
///
/// TODO(feature): Implement custom formatting when tracing-subscriber API allows it.
/// The current limitation is due to type system constraints in tracing-subscriber.
pub fn make_timer(_format: Option<&str>) -> SystemTime {
    SystemTime
}
