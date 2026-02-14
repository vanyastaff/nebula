use async_trait::async_trait;

use crate::capability::Capability;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::metadata::ActionMetadata;

/// Execution context wrapped with capability enforcement.
///
/// The engine wraps an [`ActionContext`] in a `SandboxedContext` before
/// passing it to untrusted or capability-gated actions. Every resource
/// access is checked against the granted capabilities.
///
/// Built-in trusted actions (with `IsolationLevel::None`) receive
/// a plain `ActionContext` instead.
pub struct SandboxedContext {
    inner: ActionContext,
    granted: Vec<Capability>,
}

impl SandboxedContext {
    /// Wrap an existing context with a set of granted capabilities.
    pub fn new(inner: ActionContext, granted: Vec<Capability>) -> Self {
        Self { inner, granted }
    }

    /// Access the underlying context (always available).
    pub fn inner(&self) -> &ActionContext {
        &self.inner
    }

    /// Check whether a specific capability has been granted.
    ///
    /// Returns `Ok(())` if granted, or `Err(ActionError::SandboxViolation)`.
    pub fn check_capability(&self, required: &Capability) -> Result<(), ActionError> {
        let granted = self.granted.iter().any(|g| capabilities_match(g, required));
        if granted {
            Ok(())
        } else {
            Err(ActionError::SandboxViolation {
                capability: format!("{required:?}"),
                action_id: self.inner.node_id.to_string(),
            })
        }
    }

    /// Check whether a credential is accessible.
    pub fn check_credential(&self, credential_id: &str) -> Result<(), ActionError> {
        self.check_capability(&Capability::Credential(credential_id.to_owned()))
    }

    /// Check whether network access to a host is allowed.
    pub fn check_network(&self, host: &str) -> Result<(), ActionError> {
        let granted = self.granted.iter().any(|g| match g {
            Capability::Network { allowed_hosts } => {
                allowed_hosts.iter().any(|pattern| host_matches(pattern, host))
            }
            _ => false,
        });
        if granted {
            Ok(())
        } else {
            Err(ActionError::SandboxViolation {
                capability: format!("Network({})", host),
                action_id: self.inner.node_id.to_string(),
            })
        }
    }

    /// Delegate cancellation check to inner context.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        self.inner.check_cancelled()
    }
}

/// Port trait for executing actions within an isolation boundary.
///
/// Implemented by drivers:
/// - `sandbox-inprocess`: runs in the same process with capability checks
/// - `sandbox-wasm`: runs in a WASM sandbox (full isolation)
///
/// The engine calls this instead of invoking `Action::execute` directly.
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Execute an action within the sandbox.
    ///
    /// The runner:
    /// 1. Verifies capabilities from `metadata` against granted set
    /// 2. Enforces resource limits (memory, CPU, wall time)
    /// 3. Invokes the action
    /// 4. Validates output size
    async fn execute(
        &self,
        context: SandboxedContext,
        metadata: &ActionMetadata,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError>;
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Check if a granted capability satisfies a required capability.
fn capabilities_match(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        (Capability::Credential(g), Capability::Credential(r)) => g == r,
        (Capability::Resource(g), Capability::Resource(r)) => g == r,
        (Capability::MaxMemory(g), Capability::MaxMemory(r)) => g >= r,
        (Capability::MaxCpuTime(g), Capability::MaxCpuTime(r)) => g >= r,
        (
            Capability::Environment { keys: g },
            Capability::Environment { keys: r },
        ) => r.iter().all(|rk| g.contains(rk)),
        (
            Capability::Network { allowed_hosts: g },
            Capability::Network { allowed_hosts: r },
        ) => r.iter().all(|rh| g.iter().any(|gh| host_matches(gh, rh))),
        (
            Capability::FileSystem {
                paths: g_paths,
                read_only: g_ro,
            },
            Capability::FileSystem {
                paths: r_paths,
                read_only: r_ro,
            },
        ) => {
            // If granted is read_only but required needs write → deny
            if *g_ro && !r_ro {
                return false;
            }
            r_paths.iter().all(|rp| g_paths.iter().any(|gp| rp.starts_with(gp)))
        }
        _ => false,
    }
}

/// Simple host pattern matching.
///
/// Supports `*` wildcard prefix (e.g. `"*.example.com"` matches `"api.example.com"`).
fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        pattern == host
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use nebula_core::scope::ScopeLevel;
    use std::time::Duration;

    fn test_sandboxed(caps: Vec<Capability>) -> SandboxedContext {
        let ctx = ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        );
        SandboxedContext::new(ctx, caps)
    }

    #[test]
    fn credential_check_granted() {
        let ctx = test_sandboxed(vec![Capability::Credential("github-token".into())]);
        assert!(ctx.check_credential("github-token").is_ok());
    }

    #[test]
    fn credential_check_denied() {
        let ctx = test_sandboxed(vec![Capability::Credential("github-token".into())]);
        let err = ctx.check_credential("aws-secret").unwrap_err();
        assert!(matches!(err, ActionError::SandboxViolation { .. }));
    }

    #[test]
    fn network_wildcard_match() {
        let ctx = test_sandboxed(vec![Capability::Network {
            allowed_hosts: vec!["*.example.com".into()],
        }]);
        assert!(ctx.check_network("api.example.com").is_ok());
        assert!(ctx.check_network("example.com").is_ok());
        assert!(ctx.check_network("evil.com").is_err());
    }

    #[test]
    fn network_exact_match() {
        let ctx = test_sandboxed(vec![Capability::Network {
            allowed_hosts: vec!["api.github.com".into()],
        }]);
        assert!(ctx.check_network("api.github.com").is_ok());
        assert!(ctx.check_network("github.com").is_err());
    }

    #[test]
    fn network_star_allows_all() {
        let ctx = test_sandboxed(vec![Capability::Network {
            allowed_hosts: vec!["*".into()],
        }]);
        assert!(ctx.check_network("anything.at.all").is_ok());
    }

    #[test]
    fn filesystem_read_only_blocks_write() {
        let granted = Capability::FileSystem {
            paths: vec!["/data".into()],
            read_only: true,
        };
        let required = Capability::FileSystem {
            paths: vec!["/data/file.txt".into()],
            read_only: false, // wants write
        };
        assert!(!capabilities_match(&granted, &required));
    }

    #[test]
    fn filesystem_path_prefix() {
        let granted = Capability::FileSystem {
            paths: vec!["/data".into()],
            read_only: false,
        };
        let required = Capability::FileSystem {
            paths: vec!["/data/subdir/file.txt".into()],
            read_only: false,
        };
        assert!(capabilities_match(&granted, &required));
    }

    #[test]
    fn filesystem_path_outside() {
        let granted = Capability::FileSystem {
            paths: vec!["/data".into()],
            read_only: false,
        };
        let required = Capability::FileSystem {
            paths: vec!["/etc/passwd".into()],
            read_only: true,
        };
        assert!(!capabilities_match(&granted, &required));
    }

    #[test]
    fn max_memory_sufficient() {
        let granted = Capability::MaxMemory(512 * 1024 * 1024);
        let required = Capability::MaxMemory(256 * 1024 * 1024);
        assert!(capabilities_match(&granted, &required));
    }

    #[test]
    fn max_memory_insufficient() {
        let granted = Capability::MaxMemory(128 * 1024 * 1024);
        let required = Capability::MaxMemory(256 * 1024 * 1024);
        assert!(!capabilities_match(&granted, &required));
    }

    #[test]
    fn environment_keys_subset() {
        let granted = Capability::Environment {
            keys: vec!["HOME".into(), "PATH".into(), "USER".into()],
        };
        let required = Capability::Environment {
            keys: vec!["HOME".into(), "USER".into()],
        };
        assert!(capabilities_match(&granted, &required));
    }

    #[test]
    fn environment_keys_not_subset() {
        let granted = Capability::Environment {
            keys: vec!["HOME".into()],
        };
        let required = Capability::Environment {
            keys: vec!["HOME".into(), "SECRET".into()],
        };
        assert!(!capabilities_match(&granted, &required));
    }

    #[test]
    fn max_cpu_time_sufficient() {
        let granted = Capability::MaxCpuTime(Duration::from_secs(60));
        let required = Capability::MaxCpuTime(Duration::from_secs(30));
        assert!(capabilities_match(&granted, &required));
    }

    #[test]
    fn different_capability_types_dont_match() {
        let granted = Capability::Credential("x".into());
        let required = Capability::Resource("x".into());
        assert!(!capabilities_match(&granted, &required));
    }

    #[test]
    fn cancellation_delegates_to_inner() {
        let ctx = test_sandboxed(vec![]);
        assert!(ctx.check_cancelled().is_ok());
        ctx.inner().cancellation.cancel();
        assert!(ctx.check_cancelled().is_err());
    }
}
