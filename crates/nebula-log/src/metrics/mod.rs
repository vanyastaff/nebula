//! Metrics collection using standard metrics crate

#[cfg(feature = "observability")]
pub use metrics::{
    counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram, Counter,
    Gauge, Histogram, Key, KeyName, Label, Metadata, Recorder, SharedString, Unit,
};

pub mod helpers;

#[cfg(feature = "observability")]
pub use helpers::{timed_block, timed_block_async, TimingGuard};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles() {
        // Just ensure module compiles
    }
}
