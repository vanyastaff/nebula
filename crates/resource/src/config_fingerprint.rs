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
/// reordering fields. Renaming a field changes it. A NaN or infinite float
/// — which JSON would write as `null` — is refused
/// ([`ConfigFingerprintError::NonFiniteFloat`]).
///
/// # Determinism
///
/// Object keys are sorted, so a `HashMap` field is fine; **array order is
/// kept**, because it is content for a `Vec`. A field must therefore
/// serialize the same sequence for equal values in every process: a
/// `HashSet` (or any set or collection iterated in hash or insertion order)
/// does not — its order changes with the per-process hash seed, so equal
/// configurations would get different fingerprints, and a journaled effect
/// would be refused as a mismatch after a restart. Use a `BTreeSet`, a
/// sorted `Vec`, or leave such a field out. `#[derive(ResourceConfig)]`
/// refuses a field whose type names `HashSet` at compile time.
#[must_use = "a configuration fingerprint is only computed by `finish` or `try_finish`"]
pub struct ConfigFingerprint {
    /// Each field's name and canonical JSON, sorted by name.
    members: BTreeMap<String, Vec<u8>>,
    /// The first field that could not be encoded, and why.
    refusal: Option<(String, ConfigFingerprintError)>,
}

impl ConfigFingerprint {
    /// An empty fingerprint: no field added yet.
    pub fn new() -> Self {
        Self {
            members: BTreeMap::new(),
            refusal: None,
        }
    }

    /// Adds the field `name` with `value`. See the type's determinism
    /// requirement: `value` must serialize identically for equal values in
    /// every process.
    ///
    /// A value that cannot be encoded, or a name added twice, is recorded
    /// and surfaces from [`try_finish`](Self::try_finish).
    pub fn field<T: Serialize + ?Sized>(mut self, name: &str, value: &T) -> Self {
        if self.refusal.is_some() {
            return self;
        }
        match canonical::to_canonical_finite(value) {
            Ok(json) => {
                if self.members.insert(name.to_owned(), json).is_some() {
                    self.refusal = Some((name.to_owned(), ConfigFingerprintError::DuplicateField));
                }
            },
            Err(error) => self.refusal = Some((name.to_owned(), refusal_of(error))),
        }
        self
    }

    /// The fingerprint of the fields added.
    ///
    /// A refusal is traced as a `warn` event carrying the field's name and
    /// the failed invariant ([`ConfigFingerprintError::as_str`]) — never
    /// its value — within the caller's span: the manager validates under a
    /// span naming the resource key.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigFingerprintError`] a field met: such a
    /// configuration has no stable fingerprint, and its `validate` must
    /// refuse it.
    pub fn try_finish(self) -> Result<u64, ConfigFingerprintError> {
        if let Some((field, refusal)) = &self.refusal {
            tracing::warn!(
                config.field = %field,
                refusal = refusal.as_str(),
                "resource configuration has no stable fingerprint"
            );
        }
        self.digest()
    }

    /// The fingerprint of the fields added, or — when a field could not be
    /// encoded — the one fingerprint every unencodable configuration
    /// shares. A `validate` that calls [`try_finish`](Self::try_finish)
    /// keeps such a configuration from ever being registered or reloaded.
    pub fn finish(self) -> u64 {
        self.digest().unwrap_or_else(|_| {
            let mut digest = Sha256::new();
            digest.update(UNENCODABLE_DOMAIN);
            head(digest)
        })
    }

    /// The digest of the fields, or the first refusal, untraced.
    fn digest(self) -> Result<u64, ConfigFingerprintError> {
        if let Some((_, refusal)) = self.refusal {
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
            .field(
                "refusal",
                &self.refusal.as_ref().map(|(_, refusal)| refusal),
            )
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
    /// A float inside a field is NaN or infinite: JSON would write it
    /// `null`, so it could not be told apart from a real `null`.
    NonFiniteFloat,
}

impl ConfigFingerprintError {
    /// A stable code for the failed invariant, for logs and metrics.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unserializable => "unserializable",
            Self::DuplicateKey => "duplicate_key",
            Self::TooLarge => "too_large",
            Self::DuplicateField => "duplicate_field",
            Self::NonFiniteFloat => "non_finite_float",
        }
    }
}

impl fmt::Display for ConfigFingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unserializable => "a configuration field does not serialize to JSON",
            Self::DuplicateKey => "an object of a configuration field has a duplicate key",
            Self::TooLarge => "a configuration field's canonical form is over its 1 MiB cap",
            Self::DuplicateField => "a configuration field was fingerprinted twice",
            Self::NonFiniteFloat => "a configuration field holds a NaN or infinite float",
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
        CanonicalError::NonFiniteFloat => ConfigFingerprintError::NonFiniteFloat,
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

    #[test]
    fn non_finite_floats_are_refused_not_aliased_to_null() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                ConfigFingerprint::new().field("x", &value).try_finish(),
                Err(ConfigFingerprintError::NonFiniteFloat),
                "f64 {value}"
            );
            assert_eq!(
                ConfigFingerprint::new()
                    .field("x", &Some(vec![1.0, value]))
                    .try_finish(),
                Err(ConfigFingerprintError::NonFiniteFloat),
                "nested f64 {value}"
            );
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                ConfigFingerprint::new().field("x", &value).try_finish(),
                Err(ConfigFingerprintError::NonFiniteFloat),
                "f32 {value}"
            );
        }
        // As a map key too: serde_json would stringify it.
        let mut keyed = HashMap::new();
        keyed.insert(KeyF64(f64::NAN), 1);
        assert_eq!(
            ConfigFingerprint::new().field("m", &keyed).try_finish(),
            Err(ConfigFingerprintError::NonFiniteFloat)
        );
    }

    /// An `f64` serialized as a map key.
    #[derive(PartialEq)]
    struct KeyF64(f64);

    impl Eq for KeyF64 {}

    impl std::hash::Hash for KeyF64 {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.to_bits().hash(state);
        }
    }

    impl Serialize for KeyF64 {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_f64(self.0)
        }
    }

    #[test]
    fn finite_floats_match_their_golden_vector() {
        // SHA-256("nebula-resource/config-fingerprint/v1\0{\"ratio\":0.5,\"scale\":1.5}"),
        // the `f32` widened as `serde_json` widens it.
        let fingerprint = ConfigFingerprint::new()
            .field("ratio", &0.5_f64)
            .field("scale", &1.5_f32)
            .try_finish();
        assert_eq!(fingerprint, Ok(0x706b_8c12_e1ab_6998));
    }

    /// A refusal is traced with the field name and the invariant, never the
    /// value.
    #[test]
    fn a_refusal_is_traced_without_the_value() {
        let events = capture::Events::default();
        let refused = tracing::subscriber::with_default(events.clone(), || {
            ConfigFingerprint::new()
                .field("secretish_ratio", &f64::NAN)
                .field("endpoint", "https://do-not-log.example")
                .try_finish()
        });
        assert_eq!(refused, Err(ConfigFingerprintError::NonFiniteFloat));
        let recorded = events.take();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        let event = &recorded[0];
        assert!(event.contains("config.field=secretish_ratio"), "{event}");
        assert!(event.contains("refusal=non_finite_float"), "{event}");
        assert!(!event.contains("do-not-log"), "{event}");

        // `finish` (the `fingerprint` path) never traces.
        let quiet = capture::Events::default();
        tracing::subscriber::with_default(quiet.clone(), || {
            let _ = ConfigFingerprint::new().field("x", &f64::NAN).finish();
        });
        assert!(quiet.take().is_empty());
    }

    /// A minimal subscriber recording each event's fields as text.
    mod capture {
        use std::{
            fmt,
            sync::{Arc, Mutex},
        };

        use tracing::{
            Event, Metadata, Subscriber,
            field::{Field, Visit},
            span,
        };

        #[derive(Clone, Default)]
        pub(super) struct Events(Arc<Mutex<Vec<String>>>);

        impl Events {
            pub(super) fn take(&self) -> Vec<String> {
                std::mem::take(&mut *self.0.lock().expect("events"))
            }
        }

        struct Text(String);

        impl Visit for Text {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                self.0.push_str(&format!("{}={value:?} ", field.name()));
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.push_str(&format!("{}={value} ", field.name()));
            }
        }

        impl Subscriber for Events {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }

            fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
                span::Id::from_u64(1)
            }

            fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

            fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

            fn event(&self, event: &Event<'_>) {
                let mut text = Text(String::new());
                event.record(&mut text);
                self.0.lock().expect("events").push(text.0);
            }

            fn enter(&self, _: &span::Id) {}

            fn exit(&self, _: &span::Id) {}
        }
    }
}
