//! Maturity level for a catalog entity.
//!
//! Mirrors the vocabulary used in `docs/MATURITY.md` — the per-crate table
//! that tracks which surfaces are frontier-experimental vs. stable. Every
//! metadata entry now declares its own maturity in code, not only in docs.

use serde::{Deserialize, Serialize};

/// Declared stability level of a catalog entity.
///
/// Default is [`MaturityLevel::Stable`] so authors that never touch the
/// field don't accidentally ship experimental code as frontier — if a
/// surface is experimental, the author must state it explicitly.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaturityLevel {
    /// Actively iterated, public surface may break without notice.
    Experimental,
    /// Stabilizing — breaking changes require a deprecation cycle.
    Beta,
    /// Stable surface — breaking changes require a major version bump.
    #[default]
    Stable,
    /// Scheduled for removal — use [`DeprecationNotice`](crate::DeprecationNotice)
    /// to describe when and what replaces it.
    Deprecated,
}

impl MaturityLevel {
    /// Returns `true` for [`Self::Experimental`] and [`Self::Beta`] — the
    /// levels where breaking changes can land without a major bump.
    #[must_use]
    pub fn is_unstable(self) -> bool {
        matches!(self, Self::Experimental | Self::Beta)
    }

    /// Returns `true` for [`Self::Deprecated`].
    #[must_use]
    pub fn is_deprecated(self) -> bool {
        matches!(self, Self::Deprecated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_stable() {
        assert_eq!(MaturityLevel::default(), MaturityLevel::Stable);
        assert!(!MaturityLevel::default().is_unstable());
    }

    #[test]
    fn serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&MaturityLevel::Experimental).unwrap(),
            r#""experimental""#
        );
        assert_eq!(
            serde_json::from_str::<MaturityLevel>(r#""deprecated""#).unwrap(),
            MaturityLevel::Deprecated
        );
    }
}
