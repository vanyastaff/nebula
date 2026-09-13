//! Admission budgets and allocation-conscious serde helpers.

use std::{
    fmt,
    io::{self, Write},
    marker::PhantomData,
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, SeqAccess, Visitor},
};

use crate::{MetadataError, MetadataField};

pub(crate) const SHARED_BYTES: usize = 32 * 1024;
pub(crate) const SCHEMA_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MANIFEST_BYTES: usize = 64 * 1024;
pub(crate) const DESCRIPTION_BYTES: usize = 8 * 1024;
pub(crate) const CATEGORY_COUNT: usize = 16;
pub(crate) const TAG_COUNT: usize = 32;
pub(crate) const TAG_BYTES: usize = 64;
pub(crate) const LINK_COUNT: usize = 16;
pub(crate) const RAW_ENTRIES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingCollection<T> {
    entries: Vec<T>,
    exceeded: bool,
}

impl<T> Default for PendingCollection<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            exceeded: false,
        }
    }
}

impl<T> PendingCollection<T> {
    pub(crate) fn collect(entries: impl IntoIterator<Item = T>) -> Self {
        let mut result = Self::default();
        // The extra entry detects overflow without exhausting an untrusted iterator.
        for entry in entries.into_iter().take(RAW_ENTRIES + 1) {
            result.push(entry);
        }
        result
    }

    pub(crate) fn push(&mut self, entry: T) {
        if self.entries.len() == RAW_ENTRIES {
            self.exceeded = true;
        } else {
            self.entries.push(entry);
        }
    }

    pub(crate) fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.entries.retain(keep);
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        &self.entries
    }

    pub(crate) fn take_checked(&mut self, field: MetadataField) -> Result<Vec<T>, MetadataError> {
        if self.exceeded {
            return Err(MetadataError::TooManyRawEntries(field));
        }
        Ok(std::mem::take(&mut self.entries))
    }

    pub(crate) fn set_canonical(&mut self, entries: Vec<T>) {
        self.entries = entries;
        self.exceeded = false;
    }
}

pub(crate) fn check_bytes(
    value: &str,
    limit: usize,
    field: MetadataField,
) -> Result<(), MetadataError> {
    if value.len() > limit {
        Err(MetadataError::FieldTooLarge(field))
    } else {
        Ok(())
    }
}

pub(crate) fn canonical_tags(mut tags: Vec<String>) -> Result<Vec<String>, MetadataError> {
    for tag in &mut tags {
        check_bytes(tag, SHARED_BYTES, MetadataField::Tags)?;
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(MetadataError::BlankTag);
        }
        check_bytes(trimmed, TAG_BYTES, MetadataField::Tags)?;
        if trimmed.len() != tag.len() {
            *tag = trimmed.to_owned();
        }
    }
    tags.sort_unstable();
    tags.dedup();
    if tags.len() > TAG_COUNT {
        return Err(MetadataError::TooManyEntries(MetadataField::Tags));
    }
    Ok(tags)
}

struct CountingWriter {
    remaining: usize,
    exceeded: bool,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            self.exceeded = true;
            return Err(io::Error::other("metadata byte budget exceeded"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tracing::instrument(name = "metadata.check_serialized_budget", skip_all, err)]
pub(crate) fn check_serialized<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
    overflow: MetadataError,
) -> Result<(), MetadataError> {
    let mut writer = CountingWriter {
        remaining: limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        Err(overflow)
    } else {
        result.map_err(|_| MetadataError::SerializationFailed)
    }
}

struct CaptureWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("metadata byte budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tracing::instrument(name = "metadata.capture_serialized", skip_all, err)]
pub(crate) fn capture_serialized<T: Serialize + ?Sized>(
    value: &T,
    limit: usize,
    overflow: MetadataError,
) -> Result<Vec<u8>, MetadataError> {
    let mut writer = CaptureWriter {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(overflow);
    }
    result.map_err(|_| MetadataError::SerializationFailed)?;
    Ok(writer.bytes)
}

pub(crate) fn string<'de, D: Deserializer<'de>, const N: usize>(
    deserializer: D,
) -> Result<String, D::Error> {
    struct BoundedString<const N: usize>;
    impl<const N: usize> Visitor<'_> for BoundedString<N> {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a bounded metadata string")
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
            if value.len() > N {
                return Err(E::custom(MetadataError::InvalidWire));
            }
            Ok(value.to_owned())
        }
        fn visit_string<E: serde::de::Error>(self, value: String) -> Result<String, E> {
            if value.len() > N {
                return Err(E::custom(MetadataError::InvalidWire));
            }
            Ok(value)
        }
    }
    deserializer.deserialize_string(BoundedString::<N>)
}

pub(crate) fn sequence<'de, D, T, const N: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct BoundedSequence<T, const N: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for BoundedSequence<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a bounded metadata sequence")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
            let mut entries = Vec::new();
            while let Some(entry) = sequence.next_element()? {
                if entries.len() == N {
                    return Err(A::Error::custom(MetadataError::InvalidWire));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(BoundedSequence::<T, N>(PhantomData))
}

pub(crate) fn version<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<semver::Version, D::Error> {
    string::<D, SHARED_BYTES>(deserializer)?
        .parse()
        .map_err(|_| D::Error::custom(MetadataError::InvalidWire))
}

pub(crate) fn version_requirement<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<semver::VersionReq, D::Error> {
    string::<D, SHARED_BYTES>(deserializer)?
        .parse()
        .map_err(|_| D::Error::custom(MetadataError::InvalidWire))
}

pub(crate) fn optional_version<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<semver::Version>, D::Error> {
    optional_string::<D, SHARED_BYTES>(deserializer)?
        .map(|value| {
            value
                .parse()
                .map_err(|_| D::Error::custom(MetadataError::InvalidWire))
        })
        .transpose()
}

#[derive(Deserialize)]
struct Tag(#[serde(deserialize_with = "string::<_, SHARED_BYTES>")] String);

pub(crate) fn optional_string<'de, D: Deserializer<'de>, const N: usize>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    struct Text<const N: usize>(#[serde(deserialize_with = "string::<_, N>")] String);
    Ok(Option::<Text<N>>::deserialize(deserializer)?.map(|text| text.0))
}

pub(crate) fn tags<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(sequence::<D, Tag, RAW_ENTRIES>(deserializer)?
        .into_iter()
        .map(|tag| tag.0)
        .collect())
}
