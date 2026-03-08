//! Subtype system for semantic parameter classification.
//!
//! Subtypes provide semantic meaning to parameter types, allowing:
//! - Better UI widget selection (color picker for Color, date picker for Date)
//! - Automatic validation based on subtype
//! - Type-specific formatting and parsing
//! - Improved developer experience with explicit intent
//!
//! ## Trait-Based Extensibility (New!)
//!
//! See [`traits`] and [`macros_typed`] modules for the new trait-based API
//! that allows defining custom subtypes. Read [`PARAMDEF_IMPROVEMENTS.md`]
//! for architectural details.

pub mod macros_typed;
pub mod std_subtypes;
pub mod traits;

use serde::{Deserialize, Serialize};

/// Semantic subtype for checkbox/boolean parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BooleanSubtype {
    /// Generic on/off toggle.
    Toggle,
    /// Feature rollout or kill-switch flag.
    FeatureFlag,
    /// User consent confirmation.
    Consent,
}

impl BooleanSubtype {
    /// Parses a subtype from its canonical string name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "toggle" => Some(Self::Toggle),
            "feature_flag" => Some(Self::FeatureFlag),
            "consent" => Some(Self::Consent),
            _ => None,
        }
    }

    /// Returns a human-readable description of the subtype.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Toggle => "Boolean toggle",
            Self::FeatureFlag => "Feature flag",
            Self::Consent => "Consent confirmation",
        }
    }

    /// Returns the default value commonly used by this subtype.
    #[must_use]
    pub const fn default_value(&self) -> bool {
        false
    }
}

impl Default for BooleanSubtype {
    fn default() -> Self {
        Self::Toggle
    }
}

/// Semantic subtype for text parameters.
///
/// Defines the intended use and format of text data, enabling:
/// - Appropriate UI widgets (email input, URL input, code editor)
/// - Automatic validation patterns
/// - Format-specific parsing and serialization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TextSubtype {
    /// Plain text with no special formatting
    Plain,

    /// Email address (validates RFC 5322 format)
    Email,

    /// HTTP/HTTPS URL
    Url,

    /// Absolute or relative file path
    FilePath,

    /// Directory path
    DirectoryPath,

    /// Hexadecimal color (#RGB, #RRGGBB, #RRGGBBAA)
    ColorHex,

    /// Password or secret (masked in UI)
    Password,

    /// JSON string (validates JSON syntax)
    Json,

    /// YAML string
    Yaml,

    /// XML string
    Xml,

    /// TOML string
    Toml,

    /// Markdown formatted text
    Markdown,

    /// HTML markup
    Html,

    /// CSS stylesheet
    Css,

    /// JavaScript code
    JavaScript,

    /// TypeScript code
    TypeScript,

    /// Python code
    Python,

    /// Rust code
    Rust,

    /// SQL query
    Sql,

    /// Regular expression pattern
    Regex,

    /// Glob pattern (*.rs, **/*.json)
    Glob,

    /// MIME type (text/plain, application/json)
    MimeType,

    /// IPv4 address (192.168.1.1)
    Ipv4,

    /// IPv6 address
    Ipv6,

    /// MAC address
    MacAddress,

    /// UUID (v4, v5, etc)
    Uuid,

    /// Semantic version (1.2.3, 1.0.0-alpha)
    Semver,

    /// Git commit hash
    GitHash,

    /// Date in ISO 8601 format (YYYY-MM-DD)
    IsoDate,

    /// Time in ISO 8601 format (HH:MM:SS)
    IsoTime,

    /// DateTime in ISO 8601 format
    IsoDateTime,

    /// Duration (1h30m, 90s)
    Duration,

    /// Cron expression
    Cron,

    /// Locale code (en-US, ru-RU)
    Locale,

    /// Currency code (USD, EUR, RUB)
    Currency,

    /// Country code (US, GB, RU)
    CountryCode,

    /// Phone number
    Phone,

    /// Credit card number (masked)
    CreditCard,

    /// Base64 encoded data
    Base64,

    /// JWT token
    JwtToken,

    /// API key or token
    ApiKey,

    /// SSH public key
    SshKey,

    /// PEM certificate
    PemCertificate,

    /// Environment variable name
    EnvVar,

    /// Unix username
    Username,

    /// Hostname or domain
    Hostname,

    /// Port number as string
    Port,

    /// Multi-line text (textarea)
    Multiline,

    /// Rich text (with formatting)
    RichText,

    /// Template string (with placeholders)
    Template,

    /// Expression (arithmetic, logical)
    Expression,
}

impl TextSubtype {
    /// Parses a subtype from its canonical string name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "plain" => Some(Self::Plain),
            "email" => Some(Self::Email),
            "url" => Some(Self::Url),
            "password" => Some(Self::Password),
            "json" => Some(Self::Json),
            "uuid" => Some(Self::Uuid),
            "file_path" => Some(Self::FilePath),
            "port" => Some(Self::Port),
            _ => None,
        }
    }

    /// Returns a human-readable description of the subtype.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Plain => "Plain text",
            Self::Email => "Email address",
            Self::Url => "URL address",
            Self::FilePath => "File path",
            Self::DirectoryPath => "Directory path",
            Self::ColorHex => "Hexadecimal color",
            Self::Password => "Password or secret",
            Self::Json => "JSON string",
            Self::Yaml => "YAML string",
            Self::Xml => "XML string",
            Self::Toml => "TOML string",
            Self::Markdown => "Markdown text",
            Self::Html => "HTML markup",
            Self::Css => "CSS stylesheet",
            Self::JavaScript => "JavaScript code",
            Self::TypeScript => "TypeScript code",
            Self::Python => "Python code",
            Self::Rust => "Rust code",
            Self::Sql => "SQL query",
            Self::Regex => "Regular expression",
            Self::Glob => "Glob pattern",
            Self::MimeType => "MIME type",
            Self::Ipv4 => "IPv4 address",
            Self::Ipv6 => "IPv6 address",
            Self::MacAddress => "MAC address",
            Self::Uuid => "UUID",
            Self::Semver => "Semantic version",
            Self::GitHash => "Git commit hash",
            Self::IsoDate => "ISO date",
            Self::IsoTime => "ISO time",
            Self::IsoDateTime => "ISO date-time",
            Self::Duration => "Duration",
            Self::Cron => "Cron expression",
            Self::Locale => "Locale code",
            Self::Currency => "Currency code",
            Self::CountryCode => "Country code",
            Self::Phone => "Phone number",
            Self::CreditCard => "Credit card number",
            Self::Base64 => "Base64 data",
            Self::JwtToken => "JWT token",
            Self::ApiKey => "API key",
            Self::SshKey => "SSH public key",
            Self::PemCertificate => "PEM certificate",
            Self::EnvVar => "Environment variable",
            Self::Username => "Username",
            Self::Hostname => "Hostname",
            Self::Port => "Port number",
            Self::Multiline => "Multi-line text",
            Self::RichText => "Rich text",
            Self::Template => "Template string",
            Self::Expression => "Expression",
        }
    }

    /// Returns whether this subtype should be masked in UI (for sensitive data).
    #[must_use]
    pub const fn is_sensitive(&self) -> bool {
        matches!(
            self,
            Self::Password
                | Self::ApiKey
                | Self::JwtToken
                | Self::CreditCard
                | Self::SshKey
                | Self::PemCertificate
        )
    }

    /// Returns whether this subtype represents code that should use a code editor.
    #[must_use]
    pub const fn is_code(&self) -> bool {
        matches!(
            self,
            Self::Json
                | Self::Yaml
                | Self::Xml
                | Self::Toml
                | Self::Markdown
                | Self::Html
                | Self::Css
                | Self::JavaScript
                | Self::TypeScript
                | Self::Python
                | Self::Rust
                | Self::Sql
        )
    }

    /// Returns suggested validation pattern for this subtype.
    #[must_use]
    pub fn validation_pattern(&self) -> Option<&'static str> {
        match self {
            Self::Email => Some(r"^[^\s@]+@[^\s@]+\.[^\s@]+$"),
            Self::Url => Some(r"^https?://[^\s/$.?#].[^\s]*$"),
            Self::Ipv4 => Some(r"^(\d{1,3}\.){3}\d{1,3}$"),
            Self::Uuid => Some(
                r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
            ),
            Self::Semver => Some(r"^\d+\.\d+\.\d+(-[0-9A-Za-z-]+)?(\+[0-9A-Za-z-]+)?$"),
            Self::ColorHex => Some(r"^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$"),
            Self::MacAddress => Some(r"^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$"),
            _ => None,
        }
    }
}

impl Default for TextSubtype {
    fn default() -> Self {
        Self::Plain
    }
}

/// Semantic subtype for number parameters.
///
/// Defines the physical or logical meaning of numeric values, enabling:
/// - Unit conversion (meters <-> feet)
/// - Appropriate formatting (currency, percentages)
/// - Context-aware validation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NumberSubtype {
    /// Plain number with no special meaning
    None,

    /// Linear distance (meters, feet, etc)
    Distance,

    /// Angular measurement (degrees, radians)
    Angle,

    /// Time duration (seconds, milliseconds)
    Time,

    /// Temperature (celsius, fahrenheit)
    Temperature,

    /// Speed or velocity (m/s, mph)
    Speed,

    /// Acceleration (m/s²)
    Acceleration,

    /// Mass or weight (kg, pounds)
    Mass,

    /// Volume (liters, gallons)
    Volume,

    /// Area (square meters, acres)
    Area,

    /// Energy (joules, calories)
    Energy,

    /// Power (watts)
    Power,

    /// Pressure (pascals, psi)
    Pressure,

    /// Force (newtons)
    Force,

    /// Frequency (hertz)
    Frequency,

    /// Percentage (0-100 or 0-1)
    Percentage,

    /// Factor or multiplier
    Factor,

    /// Pixel measurement
    Pixel,

    /// Frame index or count
    Frame,

    /// Byte size (bytes, KB, MB)
    ByteSize,

    /// Bit rate (bps, Mbps)
    BitRate,

    /// Currency amount
    Currency,

    /// Index in array/list (0-based)
    Index,

    /// Count or quantity
    Count,

    /// Port number (1-65535)
    Port,

    /// Opacity or transparency (0-1)
    Opacity,

    /// Brightness level
    Brightness,

    /// Ratio (aspect ratio, etc)
    Ratio,

    /// Level or tier
    Level,

    /// Priority value
    Priority,

    /// Score or rating
    Score,

    /// Timeout duration
    Timeout,

    /// Delay duration
    Delay,

    /// Timestamp (Unix epoch)
    Timestamp,
}

impl NumberSubtype {
    /// Parses a subtype from its canonical string name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "number" | "none" => Some(Self::None),
            "distance" => Some(Self::Distance),
            "percentage" => Some(Self::Percentage),
            "factor" => Some(Self::Factor),
            "port" => Some(Self::Port),
            "timestamp" => Some(Self::Timestamp),
            _ => None,
        }
    }

    /// Returns a human-readable description of the subtype.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::None => "Plain number",
            Self::Distance => "Distance measurement",
            Self::Angle => "Angular measurement",
            Self::Time => "Time duration",
            Self::Temperature => "Temperature",
            Self::Speed => "Speed or velocity",
            Self::Acceleration => "Acceleration",
            Self::Mass => "Mass or weight",
            Self::Volume => "Volume",
            Self::Area => "Area",
            Self::Energy => "Energy",
            Self::Power => "Power",
            Self::Pressure => "Pressure",
            Self::Force => "Force",
            Self::Frequency => "Frequency",
            Self::Percentage => "Percentage",
            Self::Factor => "Factor or multiplier",
            Self::Pixel => "Pixel measurement",
            Self::Frame => "Frame index",
            Self::ByteSize => "Byte size",
            Self::BitRate => "Bit rate",
            Self::Currency => "Currency amount",
            Self::Index => "Array index",
            Self::Count => "Count or quantity",
            Self::Port => "Port number",
            Self::Opacity => "Opacity level",
            Self::Brightness => "Brightness level",
            Self::Ratio => "Ratio value",
            Self::Level => "Level or tier",
            Self::Priority => "Priority value",
            Self::Score => "Score or rating",
            Self::Timeout => "Timeout duration",
            Self::Delay => "Delay duration",
            Self::Timestamp => "Unix timestamp (seconds)",
        }
    }

    /// Returns whether this subtype represents a percentage (needs % display).
    #[must_use]
    pub const fn is_percentage(&self) -> bool {
        matches!(self, Self::Percentage | Self::Opacity)
    }

    /// Returns whether this subtype represents an integer value.
    #[must_use]
    pub const fn is_integer_only(&self) -> bool {
        matches!(
            self,
            Self::Pixel
                | Self::Frame
                | Self::Index
                | Self::Count
                | Self::Port
                | Self::Level
                | Self::Priority
                | Self::Timestamp
        )
    }

    /// Returns suggested constraints for this subtype.
    #[must_use]
    pub const fn default_constraints(&self) -> Option<(f64, f64)> {
        match self {
            Self::Percentage | Self::Opacity => Some((0.0, 100.0)),
            Self::Port => Some((1.0, 65535.0)),
            Self::Index | Self::Count | Self::Frame => Some((0.0, f64::MAX)),
            Self::Brightness => Some((0.0, 255.0)),
            Self::Factor => Some((0.0, f64::MAX)),
            _ => None,
        }
    }
}

impl Default for NumberSubtype {
    fn default() -> Self {
        Self::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_subtype_serde() {
        let subtype = BooleanSubtype::FeatureFlag;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, r#""feature_flag""#);

        let deserialized: BooleanSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, BooleanSubtype::FeatureFlag);
    }

    #[test]
    fn text_subtype_serde() {
        let subtype = TextSubtype::Email;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, r#""email""#);

        let deserialized: TextSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TextSubtype::Email);
    }

    #[test]
    fn number_subtype_serde() {
        let subtype = NumberSubtype::Temperature;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, r#""temperature""#);

        let deserialized: NumberSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, NumberSubtype::Temperature);
    }

    #[test]
    fn text_subtype_sensitive_detection() {
        assert!(TextSubtype::Password.is_sensitive());
        assert!(TextSubtype::ApiKey.is_sensitive());
        assert!(!TextSubtype::Email.is_sensitive());
    }

    #[test]
    fn text_subtype_code_detection() {
        assert!(TextSubtype::Json.is_code());
        assert!(TextSubtype::Python.is_code());
        assert!(!TextSubtype::Email.is_code());
    }

    #[test]
    fn number_subtype_percentage_detection() {
        assert!(NumberSubtype::Percentage.is_percentage());
        assert!(NumberSubtype::Opacity.is_percentage());
        assert!(!NumberSubtype::Distance.is_percentage());
    }

    #[test]
    fn number_subtype_integer_detection() {
        assert!(NumberSubtype::Port.is_integer_only());
        assert!(NumberSubtype::Index.is_integer_only());
        assert!(!NumberSubtype::Distance.is_integer_only());
    }

    #[test]
    fn text_validation_patterns() {
        assert!(TextSubtype::Email.validation_pattern().is_some());
        assert!(TextSubtype::Uuid.validation_pattern().is_some());
        assert!(TextSubtype::Plain.validation_pattern().is_none());
    }

    #[test]
    fn number_default_constraints() {
        assert_eq!(
            NumberSubtype::Port.default_constraints(),
            Some((1.0, 65535.0))
        );
        assert_eq!(
            NumberSubtype::Percentage.default_constraints(),
            Some((0.0, 100.0))
        );
        assert_eq!(NumberSubtype::Distance.default_constraints(), None);
    }
}
