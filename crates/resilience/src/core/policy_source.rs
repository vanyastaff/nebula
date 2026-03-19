//! Extension point for adaptive policy configuration.

/// A source that provides the current configuration for a resilience pattern.
///
/// Static configs implement this automatically via the blanket impl below.
/// Adaptive sources compute the config at call-time based on runtime signals.
pub trait PolicySource<C: Clone>: Send + Sync {
    /// Returns the current configuration.
    fn current(&self) -> C;
}

/// Blanket impl: any `Clone + Send + Sync` value is a static policy source.
impl<C: Clone + Send + Sync> PolicySource<C> for C {
    fn current(&self) -> C {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct Config {
        value: u32,
    }

    #[test]
    fn static_config_is_policy_source() {
        let cfg = Config { value: 42 };
        // blanket impl: any Clone is a PolicySource
        assert_eq!(cfg.current(), Config { value: 42 });
    }

    #[test]
    fn static_config_returns_clone_each_time() {
        let cfg = Config { value: 7 };
        assert_eq!(cfg.current(), cfg.current());
    }
}
