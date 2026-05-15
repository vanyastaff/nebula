//! OS-level child-process hardening.
//!
//! Linux: a fixed-system-path Landlock ruleset (read-only `/lib`, `/usr/lib`,
//! TLS certs, resolver config) plus `setrlimit` resource caps. There is **no
//! per-plugin filesystem capability** — egress/credential/path mediation is
//! the broker's job (ADR-0025), not this module (canon §12.6). ABI < V4 has
//! no kernel network rule by design; network confinement is the broker's, not
//! Landlock's.
//!
//! Non-Linux: no-op (a warning is emitted once, pre-`fork`, in
//! `PreparedSandbox::prepare`).
//!
//! ## Fork safety
//!
//! `PreparedSandbox::prepare` performs **all** allocation and Landlock
//! ruleset construction *before* `fork()`. `PreparedSandbox::apply_in_child`
//! runs between `fork()` and `exec()` and calls only `setrlimit(2)` and
//! `landlock_restrict_self(2)` on the already-built ruleset — no allocation,
//! no `serde`, no `PathFd::new`, no `tracing` on the success path. This
//! defuses the post-`fork` allocator-lock deadlock class (a multi-threaded
//! parent can hold the global allocator lock at `fork`; the child must not
//! call into the allocator).

#[cfg(target_os = "linux")]
use crate::error::SandboxError;

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

/// A sandbox prepared *before* `fork`, applied *in the child* before `exec`.
///
/// Construct with `PreparedSandbox::prepare` on the host thread (allocation
/// allowed). Move it into the `Command::pre_exec` closure and call
/// `PreparedSandbox::apply_in_child` there — that call path performs no heap
/// allocation on success.
///
/// Linux-only: there is no non-Linux variant — other platforms have no OS
/// confinement here (documented honestly via `is_available()`, not a
/// per-spawn log).
#[cfg(target_os = "linux")]
pub struct PreparedSandbox {
    /// Fully-built Landlock ruleset (all `add_rule` allocation already done
    /// pre-`fork`). `Option` so the `FnMut` `pre_exec` closure can move it
    /// out via `take()` for the by-value `restrict_self`.
    ruleset: Option<landlock::RulesetCreated>,
    rlimits: LinuxRlimits,
}

#[cfg(target_os = "linux")]
impl PreparedSandbox {
    /// Build the sandbox on the host thread (pre-`fork`; allocation allowed).
    ///
    /// Constructs the fixed-system-path Landlock ruleset with best-effort
    /// compatibility (degrades cleanly on older kernels) and snapshots the
    /// rlimits.
    pub fn prepare(rlimits: LinuxRlimits) -> Result<Self, SandboxError> {
        let ruleset = linux::build_ruleset()?;
        Ok(Self {
            ruleset: Some(ruleset),
            rlimits,
        })
    }

    /// Apply the sandbox in the forked child, before `exec`.
    ///
    /// **Async-signal-safe on the success path:** only `setrlimit(2)` and
    /// `landlock_restrict_self(2)` are invoked, on data allocated pre-`fork`.
    /// No allocation, no `tracing`. Errors are returned as allocation-free
    /// `io::Error::from(ErrorKind)` / `from_raw_os_error` so even the
    /// failure path does not call into the allocator from this crate.
    ///
    /// Fails closed: a kernel that does not enforce Landlock for an untrusted
    /// child returns `PermissionDenied`, aborting the spawn rather than
    /// running the plugin unconfined.
    pub fn apply_in_child(&mut self) -> std::io::Result<()> {
        linux::set_rlimits(&self.rlimits)?;
        if let Some(ruleset) = self.ruleset.take() {
            linux::restrict_self(ruleset)?;
        }
        Ok(())
    }
}

/// Whether OS-level sandboxing is available on this platform/kernel.
///
/// Diagnostic/maturity helper — not on any hot path.
#[must_use]
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

#[cfg(target_os = "linux")]
mod linux {
    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreated, RulesetCreatedAttr, RulesetStatus,
    };

    use super::{LinuxRlimits, SandboxError};

    /// Highest Landlock ABI this build targets. `BestEffort` compatibility
    /// degrades gracefully on kernels that only support an older ABI.
    const ABI_TARGET: ABI = ABI::V5;

    /// Read-only system paths every plugin needs for dynamic linking and TLS.
    const SYSTEM_PATHS: [&str; 6] = [
        "/lib",
        "/lib64",
        "/usr/lib",
        "/usr/lib64",
        "/etc/ssl",
        "/etc/resolv.conf",
    ];

    pub(super) fn is_landlock_available() -> bool {
        Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(ABI_TARGET))
            .is_ok()
    }

    /// Build the fixed-system-path ruleset. Pre-`fork`; allocation allowed.
    pub(super) fn build_ruleset() -> Result<RulesetCreated, SandboxError> {
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(ABI_TARGET))
            .map_err(|e| SandboxError::Landlock(e.to_string()))?
            .create()
            .map_err(|e| SandboxError::Landlock(e.to_string()))?;

        for path in SYSTEM_PATHS {
            // A missing system path is not an error — it simply isn't added.
            if let Ok(fd) = PathFd::new(path) {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, AccessFs::from_read(ABI_TARGET)))
                    .map_err(|e| SandboxError::Landlock(e.to_string()))?;
            }
        }
        Ok(ruleset)
    }

    /// `setrlimit` the child. Async-signal-safe: no allocation.
    pub(super) fn set_rlimits(r: &LinuxRlimits) -> std::io::Result<()> {
        use nix::sys::resource::{Resource, setrlimit};

        // Stack array of `Copy` tuples — no heap allocation.
        let limits: [(Resource, u64); 5] = [
            (Resource::RLIMIT_AS, r.address_space_bytes),
            (Resource::RLIMIT_NOFILE, r.nofile),
            (Resource::RLIMIT_CORE, r.core_size_bytes),
            (Resource::RLIMIT_CPU, r.cpu_seconds),
            (Resource::RLIMIT_NPROC, r.nproc),
        ];
        for (resource, value) in limits {
            setrlimit(resource, value, value)
                .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
        }
        Ok(())
    }

    /// Apply `landlock_restrict_self`. Fails closed when not enforced.
    ///
    /// Success path is allocation-free. The error path discards the
    /// `landlock` error object without formatting it in this crate and
    /// returns an allocation-free `io::Error` — the rich diagnostic was
    /// already logged pre-`fork` in `build_ruleset`'s caller context.
    pub(super) fn restrict_self(ruleset: RulesetCreated) -> std::io::Result<()> {
        let status = ruleset
            .restrict_self()
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::PermissionDenied))?;
        match status.ruleset {
            RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => Ok(()),
            // Fail closed: never run an untrusted child unconfined.
            RulesetStatus::NotEnforced => {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            },
        }
    }
}

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
