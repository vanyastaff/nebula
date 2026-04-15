//! Capability model for sandboxed plugins.
//!
//! Inspired by iOS/Android permission model. A plugin declares what capabilities
//! it needs, the user grants or denies them.
//!
//! By default, a plugin has **no capabilities** — it can only do pure computation.

use serde::{Deserialize, Serialize};

/// A single capability that a plugin can request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Capability {
    // ── OS level (CLI + Desktop) ─────────────────────────────────────
    /// Network access to specific domains.
    Network {
        /// Allowed domains (e.g., "api.telegram.org", "*.googleapis.com").
        domains: Vec<String>,
    },

    /// Unrestricted network access (dangerous — for trusted plugins only).
    NetworkAll,

    /// Filesystem read access to specific paths.
    FilesystemRead {
        /// Allowed paths.
        paths: Vec<String>,
    },

    /// Filesystem read-write access to specific paths.
    FilesystemWrite {
        /// Allowed paths.
        paths: Vec<String>,
    },

    /// Access to specific environment variables.
    Env {
        /// Allowed variable names.
        keys: Vec<String>,
    },

    /// Spawn child processes.
    ProcessSpawn,

    /// Read system info (hostname, OS, CPU).
    SystemInfo,

    // ── Desktop only (Tauri) ─────────────────────────────────────────
    /// Microphone access.
    Microphone,

    /// Camera access.
    Camera,

    /// Clipboard read access.
    ClipboardRead,

    /// Clipboard write access.
    ClipboardWrite,

    /// System notifications.
    Notifications,

    /// GPU compute access.
    Gpu,
}

/// Set of capabilities granted to a plugin.
///
/// Default = empty (no capabilities).
///
/// # Examples
///
/// ```
/// use nebula_sandbox::capabilities::{Capability, PluginCapabilities};
///
/// let caps = PluginCapabilities::none();
/// assert!(!caps.has_network_access());
///
/// let caps = PluginCapabilities::new(vec![Capability::Network {
///     domains: vec!["api.telegram.org".into()],
/// }]);
/// assert!(caps.has_network_access());
/// assert!(caps.check_domain("api.telegram.org"));
/// assert!(!caps.check_domain("evil.com"));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginCapabilities {
    capabilities: Vec<Capability>,
}

impl PluginCapabilities {
    /// No capabilities — pure computation only.
    #[must_use]
    pub fn none() -> Self {
        Self {
            capabilities: Vec::new(),
        }
    }

    /// Create with a list of capabilities.
    #[must_use]
    pub fn new(capabilities: Vec<Capability>) -> Self {
        Self { capabilities }
    }

    /// All capabilities — for trusted/official plugins.
    #[must_use]
    pub fn trusted() -> Self {
        Self {
            capabilities: vec![Capability::NetworkAll],
        }
    }

    /// Add a capability.
    #[must_use]
    pub fn with(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }

    /// Check if any network access is granted.
    pub fn has_network_access(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, Capability::Network { .. } | Capability::NetworkAll))
    }

    /// Check if a specific domain is allowed.
    pub fn check_domain(&self, domain: &str) -> bool {
        self.capabilities.iter().any(|c| match c {
            Capability::NetworkAll => true,
            Capability::Network { domains } => domains.iter().any(|d| match_domain(domain, d)),
            _ => false,
        })
    }

    /// Check if a filesystem path is readable.
    pub fn check_fs_read(&self, path: &str) -> bool {
        self.capabilities.iter().any(|c| match c {
            Capability::FilesystemRead { paths } | Capability::FilesystemWrite { paths } => {
                paths.iter().any(|p| path_under(path, p))
            },
            _ => false,
        })
    }

    /// Check if a filesystem path is writable.
    pub fn check_fs_write(&self, path: &str) -> bool {
        self.capabilities.iter().any(|c| match c {
            Capability::FilesystemWrite { paths } => paths.iter().any(|p| path_under(path, p)),
            _ => false,
        })
    }

    /// Check if a specific env var is accessible.
    pub fn check_env(&self, key: &str) -> bool {
        self.capabilities.iter().any(|c| match c {
            Capability::Env { keys } => keys.iter().any(|k| k == key),
            _ => false,
        })
    }

    /// Check if a desktop capability is granted.
    pub fn has_capability(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }

    /// List all capabilities.
    pub fn list(&self) -> &[Capability] {
        &self.capabilities
    }
}

/// Check if `path` is under `base` directory.
///
/// "/tmp/foo" is under "/tmp", but "/tmp_evil" is NOT under "/tmp" —
/// comparison is component-wise via [`Path::starts_with`], not substring.
///
/// # Security
///
/// Traversal attacks like `/tmp/../etc/passwd` are defused in two layers:
///
/// 1. **Preferred path** — if both `path` and `base` canonicalize successfully (both exist on
///    disk), the canonical forms are used, so `..` is resolved by the OS before comparison.
/// 2. **Fallback** — if either path does not exist, both are run through a lexical normalisation
///    pass that drops `.` components and resolves `..` by popping the previous component. This
///    keeps the function usable in tests and sandboxed environments where paths are abstract,
///    without opening a traversal hole.
///
/// The fallback is component-wise: `"/tmp/file.txt".starts_with("/tmp")`
/// returns `true`, `"/tmp_evil".starts_with("/tmp")` returns `false`.
/// This is sep-agnostic — works the same on POSIX and Windows paths.
///
/// A **degenerate base** (`""`, `"."`, or any path that lexically normalises to an empty
/// relative path) never matches: otherwise `Path::starts_with` would treat the empty prefix as
/// matching every path, which would bypass filesystem capability checks.
fn path_under(path: &str, base: &str) -> bool {
    use std::path::{Component, Path, PathBuf};

    fn normalize_lex(p: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                Component::CurDir => {},
                Component::ParentDir => {
                    out.pop();
                },
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    fn base_lex_is_degenerate(base_path: &Path) -> bool {
        normalize_lex(base_path).as_os_str().is_empty()
    }

    let path = Path::new(path);
    let base_path = Path::new(base);

    match (path.canonicalize(), base_path.canonicalize()) {
        (Ok(p), Ok(b)) => {
            if b.as_os_str().is_empty() {
                return false;
            }
            p.starts_with(&b)
        },
        // Path missing on disk but base resolved — use lexical prefix when the base is not
        // degenerate; otherwise compare against the canonical base (for example `"."` → cwd).
        (Err(_), Ok(b)) => {
            if b.as_os_str().is_empty() {
                return false;
            }
            let path_lex = normalize_lex(path);
            let base_lex = normalize_lex(base_path);
            if base_lex.as_os_str().is_empty() {
                path_lex.starts_with(&b)
            } else {
                path_lex.starts_with(&base_lex)
            }
        },
        (Ok(p), Err(_)) => {
            if base_lex_is_degenerate(base_path) {
                return false;
            }
            let base_lex = normalize_lex(base_path);
            p.starts_with(&base_lex)
        },
        (Err(_), Err(_)) => {
            if base_lex_is_degenerate(base_path) {
                return false;
            }
            let base_lex = normalize_lex(base_path);
            normalize_lex(path).starts_with(&base_lex)
        },
    }
}

/// Match a host against a domain pattern.
/// Supports wildcard prefix: "*.example.com" matches "api.example.com".
fn match_domain(host: &str, pattern: &str) -> bool {
    let host = host.to_lowercase();
    let pattern = pattern.to_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_denies_all() {
        let caps = PluginCapabilities::none();
        assert!(!caps.has_network_access());
        assert!(!caps.check_domain("example.com"));
        assert!(!caps.check_fs_read("/tmp"));
        assert!(!caps.check_env("HOME"));
    }

    #[test]
    fn network_domain_allowlist() {
        let caps = PluginCapabilities::new(vec![Capability::Network {
            domains: vec!["api.telegram.org".into(), "*.googleapis.com".into()],
        }]);

        assert!(caps.has_network_access());
        assert!(caps.check_domain("api.telegram.org"));
        assert!(caps.check_domain("storage.googleapis.com"));
        assert!(!caps.check_domain("evil.com"));
    }

    #[test]
    fn network_all() {
        let caps = PluginCapabilities::trusted();
        assert!(caps.check_domain("anything.com"));
    }

    #[test]
    fn filesystem_read_only() {
        let caps = PluginCapabilities::new(vec![Capability::FilesystemRead {
            paths: vec!["/tmp".into()],
        }]);

        assert!(caps.check_fs_read("/tmp/file.txt"));
        assert!(!caps.check_fs_write("/tmp/file.txt"));
        assert!(!caps.check_fs_read("/etc/passwd"));
    }

    #[test]
    fn filesystem_write_implies_read() {
        let caps = PluginCapabilities::new(vec![Capability::FilesystemWrite {
            paths: vec!["/tmp".into()],
        }]);

        assert!(caps.check_fs_read("/tmp/file.txt"));
        assert!(caps.check_fs_write("/tmp/file.txt"));
    }

    #[test]
    fn env_allowlist() {
        let caps = PluginCapabilities::new(vec![Capability::Env {
            keys: vec!["API_KEY".into()],
        }]);

        assert!(caps.check_env("API_KEY"));
        assert!(!caps.check_env("SECRET"));
    }

    #[test]
    fn desktop_capabilities() {
        let caps = PluginCapabilities::new(vec![Capability::Microphone, Capability::Camera]);

        assert!(caps.has_capability(&Capability::Microphone));
        assert!(caps.has_capability(&Capability::Camera));
        assert!(!caps.has_capability(&Capability::Gpu));
    }

    #[test]
    fn builder_pattern() {
        let caps = PluginCapabilities::none()
            .with(Capability::Network {
                domains: vec!["api.telegram.org".into()],
            })
            .with(Capability::Notifications);

        assert!(caps.check_domain("api.telegram.org"));
        assert!(caps.has_capability(&Capability::Notifications));
        assert_eq!(caps.list().len(), 2);
    }

    #[test]
    fn path_under_rejects_empty_base() {
        assert!(!path_under("/etc/passwd", ""));
        assert!(!path_under("/tmp/foo", ""));
    }

    #[test]
    fn path_under_rejects_dot_base_for_nonexistent_path() {
        let p = concat!("/nonexistent-nebula-path-under-test-", line!());
        assert!(!path_under(p, "."));
    }

    #[test]
    fn filesystem_empty_path_entry_does_not_grant_all_paths() {
        let caps = PluginCapabilities::new(vec![Capability::FilesystemRead {
            paths: vec!["".into()],
        }]);
        assert!(!caps.check_fs_read("/tmp/file.txt"));
    }
}
