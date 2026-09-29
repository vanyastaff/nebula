//! `plugin.toml` marker parsing.
//!
//! Plugins are registered **in-process** (ADR-0091): there is no plugin binary to
//! spawn and no wire protocol to speak. The `plugin.toml` file that sits next to a
//! plugin crate is a small trust-and-compatibility marker read **without compiling
//! the plugin**, so pre-compile tooling can reject an incompatible or mis-identified
//! plugin cheaply. It declares two things:
//! 1. `[nebula].sdk` — semver `VersionReq` the plugin was built against.
//! 2. `[plugin].id` — optional canonical plugin-id **guard**: when present, tooling
//!    rejects the plugin if its [`PluginManifest`](crate::PluginManifest) key does not
//!    match this id. Both sources must agree.
//!
//! Parsing is not signing, authentication, or capability enforcement (canon §12.6).
//! The runtime source of truth for what a plugin registers remains `impl Plugin`.

use std::path::{Path, PathBuf};

use semver::VersionReq;
use serde::Deserialize;

/// Parsed contents of a `plugin.toml` file.
#[derive(Debug, Clone)]
pub struct PluginTomlManifest {
    /// The `[nebula].sdk` semver version requirement.
    pub sdk: VersionReq,
    /// Optional canonical plugin id **guard**. When present, tooling rejects
    /// the plugin if its `PluginManifest` key does not match this id — both
    /// sources must agree. This is a pre-compile-time pin for the plugin's
    /// identity; use it to prevent drift between the crate's Cargo package
    /// name and the manifest key the plugin registers.
    pub plugin_id: Option<String>,
}

/// Errors from [`parse_plugin_toml`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginTomlError {
    /// The `plugin.toml` file was not found.
    #[error("plugin.toml not found at {path}")]
    Missing {
        /// The path that was checked.
        path: PathBuf,
    },
    /// The file exists (or the path otherwise resolved) but could not be
    /// read — e.g. permission denied, path is a directory, or any other
    /// I/O failure. Distinct from [`PluginTomlError::Missing`] so operators
    /// can tell "plugin has no plugin.toml" apart from "plugin.toml is
    /// unreadable".
    #[error("plugin.toml at {path} could not be read: {source}")]
    Io {
        /// The path that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file was found but is not valid TOML.
    #[error("plugin.toml at {path} is not valid TOML: {source}")]
    InvalidToml {
        /// The path of the file that failed to parse.
        path: PathBuf,
        /// The underlying TOML parse error.
        #[source]
        source: Box<toml::de::Error>,
    },
    /// The `[nebula]` table is present but the required `sdk` key is missing.
    #[error("plugin.toml at {path} is missing required [nebula].sdk")]
    MissingSdkConstraint {
        /// The path of the file missing the `sdk` key.
        path: PathBuf,
    },
    /// The `[nebula].sdk` value was present but not a valid semver `VersionReq`.
    #[error("plugin.toml at {path} has invalid sdk constraint: {source}")]
    InvalidSdkConstraint {
        /// The path of the file with the invalid constraint.
        path: PathBuf,
        /// The underlying semver parse error.
        #[source]
        source: semver::Error,
    },
}

// ── Internal raw deserialize shape ──────────────────────────────────────────

#[derive(Deserialize)]
struct Raw {
    nebula: RawNebula,
    #[serde(default)]
    plugin: Option<RawPlugin>,
}

#[derive(Deserialize)]
struct RawNebula {
    #[serde(default)]
    sdk: Option<String>,
}

#[derive(Deserialize)]
struct RawPlugin {
    #[serde(default)]
    id: Option<String>,
}

// ── Public parser ────────────────────────────────────────────────────────────

/// Parse a `plugin.toml` file at `path`.
///
/// # Errors
///
/// Returns [`PluginTomlError`] for any of the failure modes described on
/// that type's variants.
pub fn parse_plugin_toml(path: &Path) -> Result<PluginTomlManifest, PluginTomlError> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PluginTomlError::Missing {
                path: path.to_path_buf(),
            }
        } else {
            PluginTomlError::Io {
                path: path.to_path_buf(),
                source: e,
            }
        }
    })?;

    let raw: Raw = toml::from_str(&contents).map_err(|source| PluginTomlError::InvalidToml {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;

    let sdk_str = raw
        .nebula
        .sdk
        .ok_or_else(|| PluginTomlError::MissingSdkConstraint {
            path: path.to_path_buf(),
        })?;

    let sdk =
        VersionReq::parse(&sdk_str).map_err(|source| PluginTomlError::InvalidSdkConstraint {
            path: path.to_path_buf(),
            source,
        })?;

    let plugin_id = raw.plugin.and_then(|p| p.id);

    Ok(PluginTomlManifest { sdk, plugin_id })
}
