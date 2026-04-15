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

/// Linux resource limits applied to plugin child processes.
///
/// These values are ignored on non-Linux platforms.
#[derive(Debug, Clone, Copy)]
pub struct LinuxRlimits {
    /// Address-space cap (`RLIMIT_AS`) in bytes.
    pub address_space_bytes: u64,
    /// Open file-descriptor cap (`RLIMIT_NOFILE`).
    pub nofile: u64,
    /// CPU-time cap (`RLIMIT_CPU`) in seconds.
    pub cpu_seconds: u64,
    /// Process-count cap (`RLIMIT_NPROC`).
    pub nproc: u64,
    /// Core-dump size cap (`RLIMIT_CORE`) in bytes.
    pub core_size_bytes: u64,
}

impl Default for LinuxRlimits {
    fn default() -> Self {
        Self {
            address_space_bytes: 512 * 1024 * 1024,
            nofile: 256,
            cpu_seconds: 30,
            nproc: 1,
            core_size_bytes: 0,
        }
    }
}

/// Apply OS-level sandbox restrictions to the current process.
///
/// On Linux: applies Landlock + rlimits.
/// On other platforms: no-op with warning.
pub fn apply_sandbox(
    capabilities: &PluginCapabilities,
    rlimits: &LinuxRlimits,
) -> Result<(), SandboxError> {
    #[cfg(target_os = "linux")]
    {
        linux::apply(capabilities, rlimits)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (capabilities, rlimits);
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

#[cfg(test)]
mod tests {
    use super::LinuxRlimits;

    #[test]
    fn linux_rlimits_default_is_hardened_but_finite() {
        let limits = LinuxRlimits::default();
        assert_eq!(limits.address_space_bytes, 512 * 1024 * 1024);
        assert_eq!(limits.nofile, 256);
        assert_eq!(limits.cpu_seconds, 30);
        assert_eq!(limits.nproc, 1);
        assert_eq!(limits.core_size_bytes, 0);
    }
}

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
    pub fn apply(
        capabilities: &PluginCapabilities,
        rlimits: &LinuxRlimits,
    ) -> Result<(), SandboxError> {
        apply_landlock(capabilities)?;
        apply_rlimits(rlimits)?;
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
    fn apply_rlimits(rlimits: &LinuxRlimits) -> Result<(), SandboxError> {
        use nix::sys::resource::{Resource, setrlimit};

        setrlimit(
            Resource::RLIMIT_AS,
            rlimits.address_space_bytes,
            rlimits.address_space_bytes,
        )
        .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_AS: {e}")))?;

        setrlimit(Resource::RLIMIT_NOFILE, rlimits.nofile, rlimits.nofile)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_NOFILE: {e}")))?;

        setrlimit(
            Resource::RLIMIT_CORE,
            rlimits.core_size_bytes,
            rlimits.core_size_bytes,
        )
        .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_CORE: {e}")))?;

        setrlimit(
            Resource::RLIMIT_CPU,
            rlimits.cpu_seconds,
            rlimits.cpu_seconds,
        )
        .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_CPU: {e}")))?;

        setrlimit(Resource::RLIMIT_NPROC, rlimits.nproc, rlimits.nproc)
            .map_err(|e| SandboxError::Rlimit(format!("RLIMIT_NPROC: {e}")))?;

        tracing::debug!(
            mem = rlimits.address_space_bytes,
            nofile = rlimits.nofile,
            cpu_seconds = rlimits.cpu_seconds,
            nproc = rlimits.nproc,
            core = rlimits.core_size_bytes,
            "rlimits applied"
        );
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
            },
            Capability::FilesystemWrite { paths } => {
                for path in paths {
                    if let Ok(fd) = PathFd::new(path) {
                        ruleset = ruleset
                            .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                            .map_err(|e| SandboxError::Landlock(e.to_string()))?;
                    }
                }
            },
            _ => {},
        }
        Ok(ruleset)
    }
}
