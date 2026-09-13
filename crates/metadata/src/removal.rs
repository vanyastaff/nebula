//! Typed removal guidance; a schedule is intent, not an execution policy.

use std::{fmt, str::FromStr};

use chrono::{Datelike, NaiveDate};
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::catalog_error::{
    CatalogValueError, deserialize_catalog_object, deserialize_catalog_string,
};

const MAX_REMOVAL_MILESTONE_BYTES: usize = 256;

/// A Gregorian calendar date in the inclusive year range 1 through 9999.
///
/// Text parsing accepts exactly `YYYY-MM-DD`; serialization uses the same
/// canonical representation. Dates carry no time, timezone, or clock policy.
///
/// ```
/// use nebula_metadata::RemovalDate;
/// let date: RemovalDate = "2028-02-29".parse()?;
/// assert_eq!(date, RemovalDate::new(2028, 2, 29)?);
/// assert_eq!((date.year(), date.month(), date.day()), (2028, 2, 29));
/// assert_eq!(date.to_string(), "2028-02-29");
/// assert!("2027-02-29".parse::<RemovalDate>().is_err());
/// # Ok::<(), nebula_metadata::CatalogValueError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RemovalDate(NaiveDate);

impl RemovalDate {
    /// Construct a Gregorian date from its year, one-based month, and day.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogValueError::InvalidRemovalDate`] when the year is
    /// outside `1..=9999` or the components do not form a real calendar date.
    #[tracing::instrument(name = "metadata.validate_removal_date", skip_all, err)]
    pub fn new(year: i32, month: u32, day: u32) -> Result<Self, CatalogValueError> {
        if !(1..=9999).contains(&year) {
            return Err(CatalogValueError::InvalidRemovalDate);
        }
        NaiveDate::from_ymd_opt(year, month, day)
            .map(Self)
            .ok_or(CatalogValueError::InvalidRemovalDate)
    }

    /// Return the year, in the inclusive range `1..=9999`.
    #[must_use]
    pub fn year(&self) -> i32 {
        self.0.year()
    }

    /// Return the month, in the inclusive range `1..=12`.
    #[must_use]
    pub fn month(&self) -> u32 {
        self.0.month()
    }

    /// Return the day of the month, starting at one.
    #[must_use]
    pub fn day(&self) -> u32 {
        self.0.day()
    }
}

impl TryFrom<&str> for RemovalDate {
    type Error = CatalogValueError;

    #[tracing::instrument(name = "metadata.parse_removal_date", skip_all, err)]
    fn try_from(date: &str) -> Result<Self, Self::Error> {
        // Chrono permits non-padded fields. Check the wire grammar first, then
        // let its calendar parser own month lengths and Gregorian leap years.
        if date.len() != 10
            || !date.bytes().enumerate().all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                _ => byte.is_ascii_digit(),
            })
        {
            return Err(CatalogValueError::InvalidRemovalDate);
        }
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| CatalogValueError::InvalidRemovalDate)?;
        Self::new(parsed.year(), parsed.month(), parsed.day())
    }
}

impl TryFrom<String> for RemovalDate {
    type Error = CatalogValueError;

    fn try_from(date: String) -> Result<Self, Self::Error> {
        Self::try_from(date.as_str())
    }
}

impl FromStr for RemovalDate {
    type Err = CatalogValueError;

    fn from_str(date: &str) -> Result<Self, Self::Err> {
        Self::try_from(date)
    }
}

impl fmt::Display for RemovalDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for RemovalDate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RemovalDate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidRemovalDate,
            Self::from_str,
        )
    }
}

/// A trimmed, nonblank removal milestone of at most 256 UTF-8 bytes.
///
/// Milestones are plain author text. Their meaning and completion are not
/// interpreted by catalog admission or runtime execution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RemovalMilestone(String);

impl RemovalMilestone {
    /// Borrow the canonical, trimmed milestone text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RemovalMilestone {
    type Error = CatalogValueError;

    #[tracing::instrument(name = "metadata.parse_removal_milestone", skip_all, err)]
    fn try_from(milestone: &str) -> Result<Self, Self::Error> {
        let trimmed = milestone.trim();
        if trimmed.is_empty() {
            return Err(CatalogValueError::BlankRemovalMilestone);
        }
        if trimmed.len() > MAX_REMOVAL_MILESTONE_BYTES {
            return Err(CatalogValueError::RemovalMilestoneTooLong);
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl TryFrom<String> for RemovalMilestone {
    type Error = CatalogValueError;

    fn try_from(milestone: String) -> Result<Self, Self::Error> {
        Self::try_from(milestone.as_str())
    }
}

impl FromStr for RemovalMilestone {
    type Err = CatalogValueError;

    fn from_str(milestone: &str) -> Result<Self, Self::Err> {
        Self::try_from(milestone)
    }
}

impl fmt::Display for RemovalMilestone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RemovalMilestone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidRemovalMilestone,
            Self::from_str,
        )
    }
}

/// Announced removal intent, distinguished by calendar date, version, or milestone.
///
/// A passed schedule remains valid evidence and does not delete or disable an
/// entity. `AtVersion` names the source entity's interface version, or the source
/// plugin's bundle version. Deprecation admission owns chronology relative to
/// the notice and current definition; this primitive only checks syntax.
///
/// The wire form is `{"kind":"on_date","value":"2028-02-29"}`, or the closed
/// `at_version` / `milestone` alternatives with a string `value`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RemovalSchedule {
    /// Removal is planned on a checked calendar date.
    OnDate(RemovalDate),
    /// Removal is planned at a specific source version.
    AtVersion(Version),
    /// Removal is planned at a named milestone.
    Milestone(RemovalMilestone),
}

enum RemovalKind {
    OnDate,
    AtVersion,
    Milestone,
}

impl<'de> Deserialize<'de> for RemovalKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidRemovalSchedule,
            |kind| match kind {
                "on_date" => Ok(Self::OnDate),
                "at_version" => Ok(Self::AtVersion),
                "milestone" => Ok(Self::Milestone),
                _ => Err(CatalogValueError::InvalidRemovalSchedule),
            },
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovalFields {
    kind: RemovalKind,
    value: String,
}

impl<'de> Deserialize<'de> for RemovalSchedule {
    #[tracing::instrument(name = "metadata.deserialize_removal_schedule", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields =
            deserialize_catalog_object(deserializer, CatalogValueError::InvalidRemovalSchedule)?;
        parse_removal_schedule(fields).map_err(D::Error::custom)
    }
}

#[tracing::instrument(name = "metadata.parse_removal_schedule", skip_all, err)]
fn parse_removal_schedule(fields: RemovalFields) -> Result<RemovalSchedule, CatalogValueError> {
    match fields.kind {
        RemovalKind::OnDate => fields.value.parse().map(RemovalSchedule::OnDate),
        RemovalKind::AtVersion => fields
            .value
            .parse()
            .map(RemovalSchedule::AtVersion)
            .map_err(|_| CatalogValueError::InvalidRemovalVersion),
        RemovalKind::Milestone => fields.value.parse().map(RemovalSchedule::Milestone),
    }
}
