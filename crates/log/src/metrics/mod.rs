//! Metrics collection using standard metrics crate

#[cfg(feature = "observability")]
pub use metrics::{
    Counter, Gauge, Histogram, Key, KeyName, Label, Metadata, Recorder, SharedString, Unit,
    counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram,
};

pub mod helpers;

#[cfg(feature = "observability")]
pub use helpers::{TimingGuard, timed_block, timed_block_async};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles() {
        // Just ensure module compiles
    }
}
