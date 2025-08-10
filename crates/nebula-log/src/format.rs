//! Format utilities (time)

use tracing_subscriber::fmt::time::SystemTime;

/// Create timer based on optional format string.
/// For now, we'll use SystemTime as the base timer since custom timers cause type issues.
/// TODO: Implement custom formatting when tracing-subscriber API allows it.
pub fn make_timer(_format: Option<&str>) -> SystemTime {
    SystemTime
}
