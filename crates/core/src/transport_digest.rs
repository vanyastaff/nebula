//! Fixed-width transport identifiers shared across crate boundaries.
//!
//! This module owns type separation and the canonical wire representation only:
//! 32 opaque bytes encoded as exactly 64 lowercase hexadecimal characters.
//! Canonicalization, hashing, manifest construction, and capability interpretation
//! remain the responsibility of the crates that derive and consume these values.

use std::{borrow::Cow, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Error returned when a transport digest is not strict lowercase 64-character hexadecimal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TransportDigestParseError {
    /// The encoded digest is not exactly 64 bytes long.
    #[error("digest must contain exactly 64 lowercase hexadecimal characters, found {actual}")]
    InvalidLength {
        /// Actual encoded byte length.
        actual: usize,
    },
    /// The encoded digest contains a byte outside lowercase hexadecimal.
    #[error("digest contains invalid lowercase hexadecimal at byte {index}")]
    InvalidHex {
        /// Index of the invalid byte.
        index: usize,
    },
}

fn decode_hex(value: &str) -> Result<[u8; 32], TransportDigestParseError> {
    let encoded = value.as_bytes();
    if encoded.len() != 64 {
        return Err(TransportDigestParseError::InvalidLength {
            actual: encoded.len(),
        });
    }

    let mut bytes = [0_u8; 32];
    for (index, pair) in encoded.chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0], index * 2)?;
        let low = hex_nibble(pair[1], index * 2 + 1)?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8, index: usize) -> Result<u8, TransportDigestParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(TransportDigestParseError::InvalidHex { index }),
    }
}

fn encode_hex(bytes: &[u8; 32], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

macro_rules! transport_digest {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Constructs an identifier from its opaque digest bytes.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Borrows the opaque digest bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                encode_hex(&self.0, formatter)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({self})", stringify!($name))
            }
        }

        impl FromStr for $name {
            type Err = TransportDigestParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                decode_hex(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let encoded = Cow::<'de, str>::deserialize(deserializer)?;
                encoded.parse().map_err(de::Error::custom)
            }
        }
    };
}

transport_digest!(
    PluginSetId,
    "Identity of a canonical registered plugin set. It is not a capability proof."
);
transport_digest!(
    WorkerFlavorRevisionId,
    "Identity of an immutable worker-flavor revision."
);
transport_digest!(
    ArtifactSetDigest,
    "Digest of the logical artifact-set provenance used to derive a worker flavor."
);
