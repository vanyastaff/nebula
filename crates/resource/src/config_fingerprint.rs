//! A stable fingerprint of a resource configuration's canonical content.
//!
//! [`ResourceConfig::fingerprint`](crate::ResourceConfig::fingerprint) is
//! durable data: the effect journal binds a recorded effect's destination to
//! it, so a journaled slot recorded by one build must resolve under the same
//! configuration in the next build — another toolchain, another platform.
//! [`std::hash::Hash`] cannot carry that: its data is documented as neither
//! portable across platforms nor stable between compiler versions, and
//! `DefaultHasher`'s algorithm may change at any time.
//!
//! A [`ConfigFingerprint`] is instead a pure function of the configuration's
//! serde content. Each fingerprinted field is written as canonical JSON —
//! compact, every object's keys sorted, a key written twice refused — and
//! the fields form one canonical JSON object keyed by field name, so neither
//! the order the fields are declared or added in nor the iteration order of
//! a map inside one changes it. The fingerprint is the first eight bytes,
//! big-endian, of
//!
//! ```text
//! SHA-256( "nebula-resource/config-fingerprint/v1" || 0x00 || canonical object )
//! ```
//!
//! The digest is one-way, so a field that should never leave the process
//! still never does; a credential is no configuration field at all — it
//! comes from the resource's credential slots.

use std::{collections::BTreeMap, fmt};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    Error,
    call::canonical::{self, CanonicalError},
};

/// Domain of the fingerprint digest, versioned: a change of the encoding
/// is a new version, never a silent change of this one.
const DOMAIN: &[u8] = b"nebula-resource/config-fingerprint/v1";

/// Domain of the one fingerprint every unencodable configuration shares.
const UNENCODABLE_DOMAIN: &[u8] = b"nebula-resource/config-fingerprint/v1/unencodable";

/// Builds the stable fingerprint of a resource configuration from its
/// operationally significant fields.
///
/// `#[derive(ResourceConfig)]` emits one for every field not marked
/// `#[config(skip_fingerprint)]`. A hand-written
/// [`ResourceConfig`](crate::ResourceConfig) uses it the same way: add every
/// field that changes what the resource connects to or how, by its name,
/// then [`finish`](Self::finish) in `fingerprint` and refuse an unencodable
/// configuration in `validate` with [`try_finish`](Self::try_finish):
///
/// ```
/// use nebula_resource::{ConfigFingerprint, Error, ResourceConfig, Schema};
///
/// #[derive(Clone, Schema)]
/// struct PgConfig {
///     url: String,
///     max_conns: u32,
/// }
///
/// impl PgConfig {
///     fn fingerprinted(&self) -> ConfigFingerprint {
///         ConfigFingerprint::new()
///             .field("url", &self.url)
///             .field("max_conns", &self.max_conns)
///     }
/// }
///
/// impl ResourceConfig for PgConfig {
///     fn validate(&self) -> Result<(), Error> {
///         self.fingerprinted().try_finish()?;
///         Ok(())
///     }
///
///     fn fingerprint(&self) -> u64 {
///         self.fingerprinted().finish()
///     }
/// }
///
/// let config = PgConfig { url: "postgres://db".to_owned(), max_conns: 8 };
/// let resized = PgConfig { max_conns: 16, ..config.clone() };
/// assert_ne!(config.fingerprint(), resized.fingerprint());
/// ```
///
/// A field is encoded through its `serde::Serialize` impl, so the
/// fingerprint follows the field's serde content, not its Rust type: a
/// change of a field's type that serializes the same keeps it, as does
/// reordering fields. Renaming a field changes it. A non-finite float is
/// written as JSON `null`, as `serde_json` writes it.
#[must_use = "a configuration fingerprint is only computed by `finish` or `try_finish`"]
pub struct ConfigFingerprint {
    /// Each field's name and canonical JSON, sorted by name.
    members: BTreeMap<String, Vec<u8>>,
    /// The first field that could not be encoded.
    refusal: Option<ConfigFingerprintError>,
}

impl ConfigFingerprint {
    /// An empty fingerprint: no field added yet.
    pub fn new() -> Self {
        Self {
            members: BTreeMap::new(),
            refusal: None,
        }
    }

    /// Adds the field `name` with `value`.
    ///
    /// A value that cannot be encoded, or a name added twice, is recorded
    /// and surfaces from [`try_finish`](Self::try_finish).
    pub fn field<T: Serialize + ?Sized>(mut self, name: &str, value: &T) -> Self {
        if self.refusal.is_some() {
            return self;
        }
        match canonical::to_canonical(value) {
            Ok(json) => {
                if self.members.insert(name.to_owned(), json).is_some() {
                    self.refusal = Some(ConfigFingerprintError::DuplicateField);
                }
            },
            Err(error) => self.refusal = Some(refusal_of(error)),
        }
        self
    }

    /// The fingerprint of the fields added.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigFingerprintError`] a field met: such a
    /// configuration has no stable fingerprint, and its `validate` must
    /// refuse it.
    pub fn try_finish(self) -> Result<u64, ConfigFingerprintError> {
        if let Some(refusal) = self.refusal {
            return Err(refusal);
        }
        let mut digest = Sha256::new();
        digest.update(DOMAIN);
        digest.update([0]);
        digest.update(b"{");
        for (index, (name, json)) in self.members.iter().enumerate() {
            if index > 0 {
                digest.update(b",");
            }
            let name =
                serde_json::to_vec(name).map_err(|_| ConfigFingerprintError::Unserializable)?;
            digest.update(&name);
            digest.update(b":");
            digest.update(json);
        }
        digest.update(b"}");
        Ok(head(digest))
    }

    /// The fingerprint of the fields added, or — when a field could not be
    /// encoded — the one fingerprint every unencodable configuration
    /// shares. A `validate` that calls [`try_finish`](Self::try_finish)
    /// keeps such a configuration from ever being registered or reloaded.
    pub fn finish(self) -> u64 {
        self.try_finish().unwrap_or_else(|_| {
            let mut digest = Sha256::new();
            digest.update(UNENCODABLE_DOMAIN);
            head(digest)
        })
    }
}

impl Default for ConfigFingerprint {
    fn default() -> Self {
        Self::new()
    }
}

/// Field values are configuration content: never printed.
impl fmt::Debug for ConfigFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigFingerprint")
            .field("fields", &self.members.len())
            .field("refusal", &self.refusal)
            .finish()
    }
}

/// The first eight bytes of `digest`, big-endian.
fn head(digest: Sha256) -> u64 {
    let bytes: [u8; 32] = digest.finalize().into();
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = bytes;
    u64::from_be_bytes([b0, b1, b2, b3, b4, b5, b6, b7])
}

/// Why a configuration has no stable fingerprint.
///
/// No variant carries configuration data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigFingerprintError {
    /// A field does not serialize to JSON: its `Serialize` impl failed, or
    /// it is a map keyed by something other than a string or a scalar.
    Unserializable,
    /// An object inside a field writes one key twice.
    DuplicateKey,
    /// A field's canonical JSON is over the 1 MiB cap.
    TooLarge,
    /// One field name was added twice.
    DuplicateField,
}

impl fmt::Display for ConfigFingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unserializable => "a configuration field does not serialize to JSON",
            Self::DuplicateKey => "an object of a configuration field has a duplicate key",
            Self::TooLarge => "a configuration field's canonical form is over its 1 MiB cap",
            Self::DuplicateField => "a configuration field was fingerprinted twice",
        })
    }
}

impl std::error::Error for ConfigFingerprintError {}

/// Why a field has no canonical form.
fn refusal_of(error: CanonicalError) -> ConfigFingerprintError {
    match error {
        CanonicalError::Unserializable => ConfigFingerprintError::Unserializable,
        CanonicalError::DuplicateKey => ConfigFingerprintError::DuplicateKey,
        CanonicalError::TooLarge => ConfigFingerprintError::TooLarge,
    }
}

/// A configuration without a stable fingerprint is permanently invalid.
impl From<ConfigFingerprintError> for Error {
    fn from(error: ConfigFingerprintError) -> Self {
        Self::permanent(format!(
            "resource configuration has no stable fingerprint: {error}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use serde::Serialize;

    use super::{ConfigFingerprint, ConfigFingerprintError};

    /// The encoding is pinned: changing it fails here, and must instead be
    /// a new domain version.
    #[test]
    fn the_encoding_matches_its_golden_vector() {
        let fingerprint = ConfigFingerprint::new()
            .field("url", "postgres://db")
            .field("max_conns", &8_u32)
            .try_finish()
            .expect("plain fields encode");
        // SHA-256("nebula-resource/config-fingerprint/v1\0{\"max_conns\":8,\"url\":\"postgres://db\"}")
        assert_eq!(fingerprint, GOLDEN_PG);
        assert_eq!(ConfigFingerprint::new().try_finish(), Ok(GOLDEN_EMPTY));
    }

    /// Computed independently, e.g. `printf '...' | sha256sum`.
    const GOLDEN_PG: u64 = 0xef89_6f97_85cb_04dd;
    /// `SHA-256("nebula-resource/config-fingerprint/v1\0{}")`.
    const GOLDEN_EMPTY: u64 = 0x2198_2e61_3096_7557;

    #[test]
    fn field_and_map_order_do_not_change_it() {
        let forward = ConfigFingerprint::new()
            .field("a", &1_u8)
            .field("b", "two")
            .finish();
        let backward = ConfigFingerprint::new()
            .field("b", "two")
            .field("a", &1_u8)
            .finish();
        assert_eq!(forward, backward);

        let ordered = BTreeMap::from([("x", 1), ("y", 2), ("z", 3)]);
        let mut hashed = HashMap::new();
        for key in ["z", "x", "y"] {
            hashed.insert(key, ordered[key]);
        }
        assert_eq!(
            ConfigFingerprint::new().field("m", &ordered).finish(),
            ConfigFingerprint::new().field("m", &hashed).finish(),
        );
    }

    #[test]
    fn a_significant_change_changes_it() {
        let base = ConfigFingerprint::new().field("a", &1_u8).finish();
        assert_ne!(base, ConfigFingerprint::new().field("a", &2_u8).finish());
        assert_ne!(base, ConfigFingerprint::new().field("b", &1_u8).finish());
        assert_ne!(
            base,
            ConfigFingerprint::new()
                .field("a", &1_u8)
                .field("c", &None::<u8>)
                .finish()
        );
    }

    #[test]
    fn an_unencodable_field_is_refused() {
        struct Failing;
        impl Serialize for Failing {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("no"))
            }
        }
        assert_eq!(
            ConfigFingerprint::new().field("f", &Failing).try_finish(),
            Err(ConfigFingerprintError::Unserializable)
        );
        assert_eq!(
            ConfigFingerprint::new()
                .field("a", &1_u8)
                .field("a", &1_u8)
                .try_finish(),
            Err(ConfigFingerprintError::DuplicateField)
        );
        let refused = ConfigFingerprint::new().field("f", &Failing).finish();
        assert_eq!(
            refused,
            ConfigFingerprint::new()
                .field("a", &1_u8)
                .field("a", &2_u8)
                .finish(),
            "every unencodable configuration shares one fingerprint"
        );
        let error: crate::Error = ConfigFingerprintError::TooLarge.into();
        assert_eq!(error.kind(), &crate::ErrorKind::Permanent);
    }
}
