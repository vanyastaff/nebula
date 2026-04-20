//! Capability model for sandboxed plugins.
//!
//! Inspired by iOS/Android permission model. A plugin declares what capabilities
//! it needs, the user grants or denies them.
//!
//! By default, a plugin has **no capabilities** — it can only do pure computation.

use serde::{Deserialize, Serialize};

/// A single capability that a plugin can request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Capability {
    // ── OS level (CLI + Desktop) ─────────────────────────────────────
    /// Network access to specific domains.
    Network {
        /// Allowed domains (e.g., "api.telegram.org", "*.googleapis.com").
        ///
        /// Wildcards are literal suffix matches: `"*.example.com"` allows
        /// `api.example.com` and `example.com`. Be careful with broad suffixes
        /// like `"*.com"` or `"*.co.uk"` — they effectively allow huge parts
        /// of the public internet.
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
/// # Security (#284)
///
/// The pre-#284 implementation fell back to a purely lexical prefix check
/// whenever `canonicalize` failed on either side. That fallback was
/// exploitable: an attacker with write access to a granted directory
/// could drop a symlink (`/allowed/tmp/escape → /etc`) and then request
/// `/allowed/tmp/escape/passwd`. `canonicalize` would fail (target not
/// accessible or not yet a valid path), the lexical fallback would keep
/// the path unchanged, the prefix check against `/allowed/tmp` would
/// succeed, and the subsequent `open()` call in the kernel would follow
/// the symlink and read `/etc/passwd`.
///
/// The current rule defuses that by being strict about the base and
/// only falling back to lexical when the filesystem clearly isn't in
/// play:
///
/// 1. `..` components are banned in either argument. A capability declaration should never contain
///    parent-traversal, and a requested path with `..` can't be resolved safely without a canonical
///    form.
/// 2. Degenerate bases (`""`, `"."` that normalises empty) never match.
/// 3. If the base canonicalizes, the path must either canonicalize under it, or have its **deepest
///    existing ancestor** canonicalize under it. Any other error on `canonicalize` (EACCES, ELOOP,
///    ENOTDIR) fails closed — a path that references something we cannot verify is treated as
///    denied.
/// 4. If the base does **not** canonicalize (abstract test fixtures or a capability pointing at a
///    path that doesn't exist yet), a lexical prefix check is used. There is no real filesystem at
///    the base, so there is no symlink attack surface — and `..` has already been banned in (1).
///
/// On-disk ancestor walk-up treats only `ErrorKind::NotFound` as
/// "keep climbing". Every other error kind is a trust violation and
/// returns `false`. A `NotFound` from `canonicalize` is further cross-checked
/// against `symlink_metadata`: if the component itself exists on disk (i.e.
/// it IS a symlink, but its target is broken or outside the base), the walk
/// stops and we fail closed. Otherwise the component really is absent and
/// we continue climbing to find the deepest existing ancestor. This closes
/// the #284 bypass where a symlink pointing at a not-yet-created outside
/// target would slip past the walk and let the parent base match.
fn path_under(path: &str, base: &str) -> bool {
    use std::{
        io::ErrorKind,
        path::{Component, Path, PathBuf},
    };

    /// `canonicalize(a)` returning `NotFound` is ambiguous:
    /// - the component does not exist at all → safe to climb to parent;
    /// - the component IS a symlink whose target is broken or absent → NOT safe to climb, because
    ///   once the target materialises the kernel will follow it wherever it points.
    ///
    /// Differentiate via `symlink_metadata`, which inspects the link itself
    /// (no traversal). Any success = the component exists on disk, so a
    /// `canonicalize NotFound` must be a broken symlink → fail closed.
    fn ancestor_is_broken_symlink(a: &Path) -> bool {
        std::fs::symlink_metadata(a).is_ok()
    }

    /// Walk up from `start`'s parent until an ancestor canonicalises,
    /// then return whether it sits under `canon_base`. A `NotFound`
    /// whose subject is a broken symlink (see `ancestor_is_broken_symlink`)
    /// fails closed rather than climbing further — this is the #284
    /// bypass fix.
    fn canonical_ancestor_is_under(start: &Path, canon_base: &Path) -> bool {
        let mut ancestor = start.parent();
        while let Some(a) = ancestor {
            match a.canonicalize() {
                Ok(canon_a) => return canon_a.starts_with(canon_base),
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    if ancestor_is_broken_symlink(a) {
                        return false;
                    }
                    ancestor = a.parent();
                },
                // EACCES, ELOOP, ENOTDIR, etc. — the path references a
                // filesystem object we can't verify. Fail closed per #284.
                Err(_) => return false,
            }
        }
        false
    }

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

    fn contains_parent_dir(p: &Path) -> bool {
        p.components().any(|c| matches!(c, Component::ParentDir))
    }

    let path = Path::new(path);
    let base_path = Path::new(base);

    // (1) Reject parent-traversal in either argument.
    if contains_parent_dir(path) || contains_parent_dir(base_path) {
        return false;
    }

    // (2) Degenerate base never matches.
    if base_lex_is_degenerate(base_path) {
        return false;
    }

    match base_path.canonicalize() {
        Ok(canon_base) if !canon_base.as_os_str().is_empty() => {
            // (3) Filesystem-grounded check. Prefer full-path canonicalisation; otherwise walk up
            //     through `NotFound` ancestors until one canonicalises, and check that resolved
            //     point against the canonical base. Every other error kind fails closed.
            match path.canonicalize() {
                Ok(canon_path) => canon_path.starts_with(&canon_base),
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    // Leaf might legitimately not exist yet (common:
                    // write to a new file under a granted dir). But if
                    // the leaf itself IS a broken symlink on disk, fail
                    // closed — otherwise the #284 bypass resurfaces when
                    // the target appears later.
                    if ancestor_is_broken_symlink(path) {
                        return false;
                    }
                    canonical_ancestor_is_under(path, &canon_base)
                },
                // Same "fail closed on unknown error" policy for the
                // top-level canonicalize (e.g., EACCES on a symlink's
                // parent): we can't reason about where the symlink
                // actually points, so deny.
                Err(_) => false,
            }
        },
        // (4) Base is abstract — lexical comparison only. `..` is already banned, so this is safe
        //     against traversal. Symlink attacks are not in scope here because there is no real
        //     filesystem at the base.
        _ => {
            let base_lex = normalize_lex(base_path);
            if base_lex.as_os_str().is_empty() {
                return false;
            }
            match path.canonicalize() {
                Ok(canon_path) => canon_path.starts_with(&base_lex),
                Err(_) => normalize_lex(path).starts_with(&base_lex),
            }
        },
    }
}

/// Match a host against a domain pattern.
/// Supports wildcard prefix: "*.example.com" matches "api.example.com".
///
/// Wildcards apply to the literal suffix after `"*."`; this function does
/// not consult the Public Suffix List.
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
    fn wildcard_suffix_is_literal_and_can_be_broad() {
        let caps = PluginCapabilities::new(vec![Capability::Network {
            domains: vec!["*.com".into()],
        }]);
        assert!(caps.check_domain("example.com"));
        assert!(caps.check_domain("evil.com"));
        assert!(!caps.check_domain("example.org"));
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
            paths: vec![String::new()],
        }]);
        assert!(!caps.check_fs_read("/tmp/file.txt"));
    }

    // ---- #284 symlink-bypass regression tests ------------------------

    #[test]
    fn path_under_rejects_parent_dir_in_path() {
        // Lexical escape via `..` must be rejected even when the base
        // doesn't canonicalize (the attack surface the pre-#284 lexical
        // fallback otherwise kept open).
        assert!(!path_under("/allowed/../etc/passwd", "/allowed"));
        assert!(!path_under("/allowed/nested/../../etc/passwd", "/allowed"));
    }

    #[test]
    fn path_under_rejects_parent_dir_in_base() {
        assert!(!path_under("/etc/passwd", "/allowed/.."));
    }

    #[cfg(unix)]
    #[test]
    fn path_under_rejects_symlink_escape_to_existing_target() {
        // Classic symlink escape: the symlink resolves to a real path
        // OUTSIDE the granted base. canonicalize sees this, starts_with
        // against the base fails, request is rejected.
        let base = tempfile::tempdir().expect("base tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_file = outside.path().join("secret");
        std::fs::write(&outside_file, b"secret").expect("write secret");
        let escape = base.path().join("escape");
        std::os::unix::fs::symlink(&outside_file, &escape).expect("create symlink");

        let base_str = base.path().to_str().expect("utf8 tempdir path");
        let escape_str = escape.to_str().expect("utf8 symlink path");
        assert!(
            !path_under(escape_str, base_str),
            "symlink escape to {outside_file:?} must be rejected under base {base:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_under_rejects_symlink_to_nonexistent_target_outside_base() {
        // Core #284 bypass: symlink's target does not exist yet, so
        // canonicalize fails — the old lexical fallback happily
        // accepted the path as being under the granted base. With the
        // new rule, the deepest existing ancestor (the symlink itself)
        // canonicalizes to the target's PARENT outside the base and is
        // rejected by starts_with.
        let base = tempfile::tempdir().expect("base tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        // Note: the target does NOT exist — only `outside` exists.
        let outside_target = outside.path().join("not-yet");
        let escape = base.path().join("escape");
        std::os::unix::fs::symlink(&outside_target, &escape).expect("create symlink");

        let requested = escape.join("passwd");
        let base_str = base.path().to_str().expect("utf8 base");
        let requested_str = requested.to_str().expect("utf8 requested");
        assert!(
            !path_under(requested_str, base_str),
            "symlink to nonexistent target outside base must be rejected, \
             base={base:?} requested={requested:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_under_rejects_broken_symlink_mid_path() {
        // Defense-in-depth for the #284 regression the previous commit
        // left open: the symlink is in the MIDDLE of the requested
        // path (`base/escape/passwd`), its target is a not-yet-existing
        // file outside the base. canonicalize(base/escape/passwd)
        // returns NotFound, and canonicalize(base/escape) ALSO returns
        // NotFound because the symlink target is broken — but
        // symlink_metadata succeeds, so we must NOT climb to `base`.
        let base = tempfile::tempdir().expect("base tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_target = outside.path().join("not-yet");
        let escape = base.path().join("escape");
        std::os::unix::fs::symlink(&outside_target, &escape).expect("create symlink");

        let requested = escape.join("passwd");
        let base_str = base.path().to_str().expect("utf8 base");
        let requested_str = requested.to_str().expect("utf8 requested");
        assert!(
            !path_under(requested_str, base_str),
            "broken symlink in the middle of the path must fail closed, \
             base={base:?} requested={requested:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_under_rejects_broken_symlink_as_leaf() {
        // Complementary case: the symlink IS the leaf, target missing,
        // target outside base. Without the symlink_metadata check the
        // walk would climb to `base` and accept.
        let base = tempfile::tempdir().expect("base tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_target = outside.path().join("not-yet");
        let escape = base.path().join("escape");
        std::os::unix::fs::symlink(&outside_target, &escape).expect("create symlink");

        let base_str = base.path().to_str().expect("utf8 base");
        let escape_str = escape.to_str().expect("utf8 escape");
        assert!(
            !path_under(escape_str, base_str),
            "broken symlink as leaf must fail closed, base={base:?} escape={escape:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_under_accepts_symlink_resolving_inside_base() {
        // Sanity: if the symlink resolves to a location INSIDE the
        // granted base, the request IS legitimate (`real/` lives under
        // the same tempdir as the symlink).
        let base = tempfile::tempdir().expect("base tempdir");
        let real_dir = base.path().join("real");
        std::fs::create_dir(&real_dir).expect("create real dir");
        let real_file = real_dir.join("data");
        std::fs::write(&real_file, b"data").expect("write data");
        let link = base.path().join("alias");
        std::os::unix::fs::symlink(&real_file, &link).expect("create symlink");

        let base_str = base.path().to_str().expect("utf8 base");
        let link_str = link.to_str().expect("utf8 link");
        assert!(
            path_under(link_str, base_str),
            "symlink resolving inside base must be accepted, base={base:?} link={link:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_under_accepts_nonexistent_leaf_under_real_base() {
        // Common legitimate case: the base exists, the caller wants to
        // write to a not-yet-existing file inside it. Walking up from
        // the requested path finds the base itself as the deepest
        // canonicalizable ancestor, and the prefix check passes.
        let base = tempfile::tempdir().expect("base tempdir");
        let requested = base.path().join("does-not-exist-yet.txt");
        let base_str = base.path().to_str().expect("utf8 base");
        let requested_str = requested.to_str().expect("utf8 requested");
        assert!(
            path_under(requested_str, base_str),
            "nonexistent leaf under a real base must be accepted",
        );
    }
}
