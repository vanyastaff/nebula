//! Resource versioning and compatibility checking
//!
//! This module provides semantic versioning support for resources, including:
//! - Version parsing and comparison
//! - Compatibility checking between versions
//! - Migration path validation

use crate::core::error::{ResourceError, ResourceResult};
use std::fmt;
use std::str::FromStr;

/// Semantic version for resources
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Version {
    /// Major version (breaking changes)
    pub major: u32,
    /// Minor version (backwards-compatible features)
    pub minor: u32,
    /// Patch version (backwards-compatible bug fixes)
    pub patch: u32,
    /// Pre-release identifier (e.g., "alpha", "beta.1")
    pub pre_release: Option<String>,
    /// Build metadata
    pub build: Option<String>,
}

impl Version {
    /// Create a new version
    #[must_use] 
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre_release: None,
            build: None,
        }
    }

    /// Create a version with pre-release identifier
    pub fn with_pre_release(mut self, pre_release: impl Into<String>) -> Self {
        self.pre_release = Some(pre_release.into());
        self
    }

    /// Create a version with build metadata
    pub fn with_build(mut self, build: impl Into<String>) -> Self {
        self.build = Some(build.into());
        self
    }

    /// Check if this version is compatible with another version
    ///
    /// Compatibility rules (following semver):
    /// - Major version must match
    /// - This version's minor must be >= required minor
    /// - Patch version doesn't matter for compatibility
    #[must_use] 
    pub fn is_compatible_with(&self, required: &Version) -> bool {
        if self.major != required.major {
            return false;
        }

        if self.major == 0 {
            // In 0.x.x, minor version changes can be breaking
            self.minor == required.minor
        } else {
            // In 1.x.x+, minor version can be greater
            self.minor >= required.minor
        }
    }

    /// Check if this version can migrate to another version
    ///
    /// Migration is possible if:
    /// - Major versions differ by at most 1
    /// - If same major, any minor/patch is acceptable
    #[must_use] 
    pub fn can_migrate_to(&self, target: &Version) -> bool {
        if self.major == target.major {
            // Same major version - always possible
            true
        } else if target.major == self.major + 1 {
            // Upgrading to next major version - possible
            true
        } else if self.major == target.major + 1 {
            // Downgrading to previous major version - risky but possible
            true
        } else {
            // Too many major versions apart
            false
        }
    }

    /// Get the precedence order for version comparison
    fn precedence(&self) -> VersionPrecedence {
        VersionPrecedence {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            has_pre_release: self.pre_release.is_some(),
        }
    }
}

/// Helper struct for version comparison
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VersionPrecedence {
    major: u32,
    minor: u32,
    patch: u32,
    has_pre_release: bool,
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.precedence().cmp(&other.precedence())
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;

        if let Some(ref pre) = self.pre_release {
            write!(f, "-{pre}")?;
        }

        if let Some(ref build) = self.build {
            write!(f, "+{build}")?;
        }

        Ok(())
    }
}

impl FromStr for Version {
    type Err = ResourceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split on '+' for build metadata
        let (version_part, build) = if let Some(pos) = s.find('+') {
            (&s[..pos], Some(s[pos + 1..].to_string()))
        } else {
            (s, None)
        };

        // Split on '-' for pre-release
        let (core_part, pre_release) = if let Some(pos) = version_part.find('-') {
            (
                &version_part[..pos],
                Some(version_part[pos + 1..].to_string()),
            )
        } else {
            (version_part, None)
        };

        // Parse major.minor.patch
        let parts: Vec<&str> = core_part.split('.').collect();
        if parts.len() != 3 {
            return Err(ResourceError::configuration(format!(
                "Invalid version format: expected 'major.minor.patch', got '{s}'"
            )));
        }

        let major = parts[0].parse::<u32>().map_err(|_| {
            ResourceError::configuration(format!("Invalid major version: {}", parts[0]))
        })?;

        let minor = parts[1].parse::<u32>().map_err(|_| {
            ResourceError::configuration(format!("Invalid minor version: {}", parts[1]))
        })?;

        let patch = parts[2].parse::<u32>().map_err(|_| {
            ResourceError::configuration(format!("Invalid patch version: {}", parts[2]))
        })?;

        Ok(Version {
            major,
            minor,
            patch,
            pre_release,
            build,
        })
    }
}

/// Version compatibility checker for resources
#[derive(Debug, Clone)]
pub struct VersionChecker {
    /// Minimum supported version
    pub min_version: Version,
    /// Maximum supported version (if any)
    pub max_version: Option<Version>,
    /// Deprecated versions
    pub deprecated: Vec<Version>,
}

impl VersionChecker {
    /// Create a new version checker
    #[must_use] 
    pub fn new(min_version: Version) -> Self {
        Self {
            min_version,
            max_version: None,
            deprecated: Vec::new(),
        }
    }

    /// Set maximum supported version
    #[must_use] 
    pub fn with_max_version(mut self, max_version: Version) -> Self {
        self.max_version = Some(max_version);
        self
    }

    /// Add a deprecated version
    #[must_use] 
    pub fn with_deprecated(mut self, version: Version) -> Self {
        self.deprecated.push(version);
        self
    }

    /// Check if a version is supported
    #[must_use] 
    pub fn is_supported(&self, version: &Version) -> bool {
        if version < &self.min_version {
            return false;
        }

        if let Some(ref max) = self.max_version
            && version > max
        {
            return false;
        }

        true
    }

    /// Check if a version is deprecated
    #[must_use] 
    pub fn is_deprecated(&self, version: &Version) -> bool {
        self.deprecated.contains(version)
    }

    /// Validate a version and return warnings if deprecated
    pub fn validate(&self, version: &Version) -> ResourceResult<Option<String>> {
        if !self.is_supported(version) {
            return Err(ResourceError::configuration(format!(
                "Version {} is not supported (min: {}, max: {:?})",
                version, self.min_version, self.max_version
            )));
        }

        if self.is_deprecated(version) {
            Ok(Some(format!(
                "Version {} is deprecated, consider upgrading to {}",
                version, self.min_version
            )))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        let v = "1.2.3".parse::<Version>().unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.pre_release, None);
        assert_eq!(v.build, None);

        let v = "2.0.0-alpha.1".parse::<Version>().unwrap();
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
        assert_eq!(v.pre_release, Some("alpha.1".to_string()));

        let v = "1.5.0+build.123".parse::<Version>().unwrap();
        assert_eq!(v.build, Some("build.123".to_string()));

        let v = "1.0.0-rc.1+build.456".parse::<Version>().unwrap();
        assert_eq!(v.pre_release, Some("rc.1".to_string()));
        assert_eq!(v.build, Some("build.456".to_string()));
    }

    #[test]
    fn test_version_display() {
        let v = Version::new(1, 2, 3);
        assert_eq!(v.to_string(), "1.2.3");

        let v = v.with_pre_release("beta.2");
        assert_eq!(v.to_string(), "1.2.3-beta.2");

        let v = v.with_build("build.789");
        assert_eq!(v.to_string(), "1.2.3-beta.2+build.789");
    }

    #[test]
    fn test_version_comparison() {
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(1, 1, 0);
        let v3 = Version::new(2, 0, 0);

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v1 < v3);
    }

    #[test]
    fn test_version_compatibility() {
        let v1_0_0 = Version::new(1, 0, 0);
        let v1_1_0 = Version::new(1, 1, 0);
        let v1_2_0 = Version::new(1, 2, 0);
        let v2_0_0 = Version::new(2, 0, 0);

        // 1.2.0 is compatible with requirements for 1.0.0 and 1.1.0
        assert!(v1_2_0.is_compatible_with(&v1_0_0));
        assert!(v1_2_0.is_compatible_with(&v1_1_0));

        // 1.0.0 is not compatible with requirements for 1.1.0
        assert!(!v1_0_0.is_compatible_with(&v1_1_0));

        // Major version mismatch - not compatible
        assert!(!v1_2_0.is_compatible_with(&v2_0_0));
        assert!(!v2_0_0.is_compatible_with(&v1_0_0));
    }

    #[test]
    fn test_version_compatibility_zero_major() {
        let v0_1_0 = Version::new(0, 1, 0);
        let v0_2_0 = Version::new(0, 2, 0);

        // In 0.x.x, minor version changes are breaking
        assert!(!v0_2_0.is_compatible_with(&v0_1_0));
        assert!(!v0_1_0.is_compatible_with(&v0_2_0));
    }

    #[test]
    fn test_version_migration() {
        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(2, 0, 0);
        let v3 = Version::new(3, 0, 0);

        // Can migrate to next major version
        assert!(v1.can_migrate_to(&v2));

        // Can't skip major versions
        assert!(!v1.can_migrate_to(&v3));

        // Can migrate within same major
        assert!(v1.can_migrate_to(&Version::new(1, 5, 0)));
    }

    #[test]
    fn test_version_checker() {
        let checker = VersionChecker::new(Version::new(1, 0, 0))
            .with_max_version(Version::new(2, 0, 0))
            .with_deprecated(Version::new(1, 0, 0));

        assert!(checker.is_supported(&Version::new(1, 5, 0)));
        assert!(!checker.is_supported(&Version::new(0, 9, 0)));
        assert!(!checker.is_supported(&Version::new(3, 0, 0)));

        assert!(checker.is_deprecated(&Version::new(1, 0, 0)));
        assert!(!checker.is_deprecated(&Version::new(1, 5, 0)));
    }
}
