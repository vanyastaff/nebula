//! Shared `serde` default/`skip_serializing_if` helpers.
//!
//! `BaseMetadata` (`base.rs`) and `PluginManifest` (`manifest.rs`) both carry
//! a `version: Version` field defaulting to `1.0.0` and a `maturity:
//! MaturityLevel` field defaulting to [`MaturityLevel::default()`], each
//! wired through identical `#[serde(default = "...", skip_serializing_if =
//! "...")]` helper pairs. Written once here instead of twice so the two
//! wire-format decisions (what "default version" and "default maturity"
//! mean) can't drift apart between the two composing types.

use semver::Version;

use crate::maturity::MaturityLevel;

/// The default interface version: `1.0.0`.
pub(crate) fn default_version() -> Version {
    Version::new(1, 0, 0)
}

/// `true` iff `v` is the default interface version.
///
/// Used as `skip_serializing_if` so a manifest/metadata value at the
/// default version omits `version` from the wire format entirely.
pub(crate) fn is_default_version(v: &Version) -> bool {
    v == &default_version()
}

/// `true` iff `m` is the default [`MaturityLevel`].
///
/// Used as `skip_serializing_if` so a manifest/metadata value at the
/// default maturity omits `maturity` from the wire format entirely.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires the &T signature"
)]
pub(crate) fn is_default_maturity(m: &MaturityLevel) -> bool {
    *m == MaturityLevel::default()
}
