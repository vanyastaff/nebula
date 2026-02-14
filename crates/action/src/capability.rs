use std::time::Duration;

/// Capability that an action may request from the runtime.
///
/// Actions declare their required capabilities in [`ActionMetadata`](crate::ActionMetadata).
/// The engine grants or denies capabilities based on isolation policy.
/// A `SandboxedContext` (Phase 3) enforces these at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Network access restricted to specific hosts.
    Network {
        /// Host patterns this action may connect to.
        allowed_hosts: Vec<String>,
    },
    /// Filesystem access restricted to specific paths.
    FileSystem {
        /// Allowed filesystem paths.
        paths: Vec<String>,
        /// If `true`, only read access is permitted.
        read_only: bool,
    },
    /// Access to a managed resource by ID.
    Resource(String),
    /// Access to a credential/secret by ID.
    Credential(String),
    /// Maximum memory the action may consume.
    MaxMemory(usize),
    /// Maximum CPU time for action execution.
    MaxCpuTime(Duration),
    /// Access to specific environment variables.
    Environment {
        /// Environment variable names this action may read.
        keys: Vec<String>,
    },
}

/// Isolation level for action execution.
///
/// Determines how much the runtime trusts an action's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IsolationLevel {
    /// Trusted built-in action — no sandbox overhead.
    ///
    /// Only for signed, first-party actions.
    None,

    /// In-process execution with capability checks.
    ///
    /// API calls are proxied through capability enforcement, but the code
    /// runs in the same process. Does **not** protect against unsafe code,
    /// memory exploits, or side channels.
    #[default]
    CapabilityGated,

    /// Full isolation via WASM or process sandbox.
    ///
    /// **Mandatory** for community/marketplace actions — cannot be
    /// overridden by configuration.
    Isolated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_isolation_is_capability_gated() {
        assert_eq!(IsolationLevel::default(), IsolationLevel::CapabilityGated);
    }

    #[test]
    fn capability_equality() {
        let a = Capability::Network {
            allowed_hosts: vec!["api.example.com".into()],
        };
        let b = Capability::Network {
            allowed_hosts: vec!["api.example.com".into()],
        };
        assert_eq!(a, b);
    }

    #[test]
    fn capability_network() {
        let cap = Capability::Network {
            allowed_hosts: vec!["*.example.com".into(), "api.internal".into()],
        };
        match &cap {
            Capability::Network { allowed_hosts } => {
                assert_eq!(allowed_hosts.len(), 2);
            }
            _ => panic!("expected Network"),
        }
    }

    #[test]
    fn capability_credential() {
        let cap = Capability::Credential("github-token".into());
        assert_eq!(cap, Capability::Credential("github-token".into()));
    }

    #[test]
    fn capability_max_memory() {
        let cap = Capability::MaxMemory(256 * 1024 * 1024); // 256 MB
        match cap {
            Capability::MaxMemory(bytes) => assert_eq!(bytes, 268_435_456),
            _ => panic!("expected MaxMemory"),
        }
    }
}
