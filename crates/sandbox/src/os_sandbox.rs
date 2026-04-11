//! OS-level sandbox enforcement for child processes.
//!
//! Uses Linux kernel security features:
//! - **Landlock** — filesystem access control (kernel 5.13+)
//! - **rlimits** — resource limits (memory, file descriptors)
//!
//! On non-Linux platforms, sandbox is advisory-only (logs a warning).

#[cfg(target_os = "linux")]
use crate::capabilities::Capability;
use crate::capabilities::PluginCapabilities;

/// Apply OS-level sandbox restrictions to the current process.
///
/// On Linux: applies Landlock + rlimits.
/// On other platforms: no-op with warning.
pub fn apply_sandbox(capabilities: &PluginCapabilities) -> Result<(), SandboxError> {
    #[cfg(target_os = "linux")]
    {
        linux::apply(capabilities)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = capabilities;
        tracing::warn!("OS-level sandboxing not available on this platform");
        Ok(())
    }
}

/// Check if OS-level sandboxing is available on this platform.
pub fn is_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_landlock_available()
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Sandbox enforcement error.
#[derive(Debug)]
pub enum SandboxError {
    /// Landlock setup failed.
    Landlock(String),
    /// Resource limit setup failed.
    Rlimit(String),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Landlock(msg) => write!(f, "landlock: {msg}"),
            Self::Rlimit(msg) => write!(f, "rlimit: {msg}"),
        }
    }
}

impl std::error::Error for SandboxError {}

#[cfg(target_os = "linux")]
mod linux {
    use landlock::{
        ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreated,
        RulesetCreatedAttr, RulesetStatus,
    };

    use super::*;

    /// Check if Landlock is supported by the running kernel.
    pub fn is_landlock_available() -> bool {
        Ruleset::default()
            .handle_access(AccessFs::from_all(ABI::V1))
            .is_ok()
    }

    /// Apply all sandbox layers.
    pub fn apply(capabilities: &PluginCapabilities) -> Result<(), SandboxError> {
        apply_landlock(capabilities)?;
        apply_rlimits()?;
        Ok(())
    }

    /// Apply Landlock filesystem restrictions.
    fn apply_landlock(capabilities: &PluginCapabilities) -> Result<(), SandboxError> {
        let abi = ABI::V1;

        let mut ruleset = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .map_err(|e| SandboxError::Landlock(e.to_string()))?
            .create()
            .map_err(|e| SandboxError::Landlock(e.to_string()))?;

        // Allow read access to standard system paths (dynamic linking, TLS certs).
        let system_paths = [
            "/lib",
            "/lib64",
            "/usr/lib",
            "/usr/lib64",
            "/etc/ssl",
            "/etc/resolv.conf",
        ];
        for path in &system_paths {
            if let Ok(fd) = PathFd::new(path) {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))
                    .map_err(|e| SandboxError::Landlock(e.to_string()))?;
            }
        }

        // Add paths from capabilities.
        for cap in capabilities.list() {
            ruleset = add_capability_rules(ruleset, cap, abi)?;
        }

        let status = ruleset
            .restrict_self()
            .map_err(|e| SandboxError::Landlock(e.to_string()))?;

        match status.ruleset {
            RulesetStatus::FullyEnforced => tracing::debug!("landlock fully enforced"),
            RulesetStatus::PartiallyEnforced => tracing::warn!("landlock partially enforced"),
            RulesetStatus::NotEnforced => tracing::warn!("landlock not enforced (kernel too old?)"),
        }

        Ok(())
    }

    /// Apply resource limits.
    fn apply_rlimits() -> Result<(), SandboxError> {
        use nix::sys::resource::{Resource, setrlimit};

        // Limit address space to 512MB.
        setrlimit(Resource::RLIMIT_AS, 512 * 1024 * 1024, 512 * 1024 * 1024)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_AS: {e}")))?;

        // Limit open file descriptors to 256.
        setrlimit(Resource::RLIMIT_NOFILE, 256, 256)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_NOFILE: {e}")))?;

        // No core dumps.
        setrlimit(Resource::RLIMIT_CORE, 0, 0)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_CORE: {e}")))?;

        // CPU time limit (30 seconds hard cap).
        setrlimit(Resource::RLIMIT_CPU, 30, 30)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_CPU: {e}")))?;

        // Prevent fork bombs — plugin cannot spawn child processes.
        setrlimit(Resource::RLIMIT_NPROC, 1, 1)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_NPROC: {e}")))?;

        tracing::debug!("rlimits applied: 512MB mem, 256 fds, 30s CPU, no fork, no core dumps");
        Ok(())
    }

    /// Add landlock rules for a single capability.
    fn add_capability_rules(
        mut ruleset: RulesetCreated,
        cap: &Capability,
        abi: ABI,
    ) -> Result<RulesetCreated, SandboxError> {
        match cap {
            Capability::FilesystemRead { paths } => {
                for path in paths {
                    if let Ok(fd) = PathFd::new(path) {
                        ruleset = ruleset
                            .add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))
                            .map_err(|e| SandboxError::Landlock(e.to_string()))?;
                    }
                }
            }
            Capability::FilesystemWrite { paths } => {
                for path in paths {
                    if let Ok(fd) = PathFd::new(path) {
                        ruleset = ruleset
                            .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                            .map_err(|e| SandboxError::Landlock(e.to_string()))?;
                    }
                }
            }
            _ => {}
        }
        Ok(ruleset)
    }
}
