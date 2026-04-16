//! Semantic versioning for workflow definitions.

use serde::{Deserialize, Serialize};

/// Semantic version for workflow definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Major version number.
    pub major: u32,
    /// Minor version number.
    pub minor: u32,
    /// Patch version number.
    pub patch: u32,
    /// Pre-release identifier (e.g., "alpha", "beta", "rc.1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<String>,
    /// Build metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
}

impl Version {
    /// Create a new version.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
            build: None,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        if let Some(build) = &self.build {
            write!(f, "+{build}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then(match (&self.pre, &other.pre) {
                // Both have no pre-release: equal
                (None, None) => std::cmp::Ordering::Equal,
                // Release (no pre) sorts after pre-release per semver
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                // Both have pre-release: lexicographic
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_release_sorts_before_release() {
        let alpha = Version {
            pre: Some("alpha".into()),
            ..Version::new(1, 0, 0)
        };
        let release = Version::new(1, 0, 0);
        assert!(alpha < release, "1.0.0-alpha must sort before 1.0.0");
    }

    #[test]
    fn pre_release_ordering_is_not_equal() {
        // This was the original bug: Ord ignored `pre`, making these equal.
        let alpha = Version {
            pre: Some("alpha".into()),
            ..Version::new(1, 0, 0)
        };
        let release = Version::new(1, 0, 0);
        assert_ne!(
            alpha.cmp(&release),
            std::cmp::Ordering::Equal,
            "1.0.0-alpha and 1.0.0 must not compare equal in Ord"
        );
    }

    #[test]
    fn pre_release_lexicographic() {
        let alpha = Version {
            pre: Some("alpha".into()),
            ..Version::new(1, 0, 0)
        };
        let beta = Version {
            pre: Some("beta".into()),
            ..Version::new(1, 0, 0)
        };
        assert!(alpha < beta, "1.0.0-alpha must sort before 1.0.0-beta");
    }

    #[test]
    fn partial_eq_considers_pre_and_build() {
        // Derived PartialEq compares all fields including pre and build.
        let a = Version::new(1, 0, 0);
        let b = Version {
            pre: Some("alpha".into()),
            ..Version::new(1, 0, 0)
        };
        assert_ne!(a, b, "PartialEq must distinguish release from pre-release");

        let c = Version {
            build: Some("20240101".into()),
            ..Version::new(1, 0, 0)
        };
        assert_ne!(a, c, "PartialEq must distinguish different build metadata");
    }

    #[test]
    fn display_includes_pre_and_build() {
        let v = Version {
            pre: Some("rc.1".into()),
            build: Some("sha.abc123".into()),
            ..Version::new(2, 1, 0)
        };
        assert_eq!(v.to_string(), "2.1.0-rc.1+sha.abc123");
    }
}
