//! Number parameter subtypes for semantic validation and UI hints.
//!
//! This module defines subtypes that provide semantic meaning to number parameters,
//! enabling appropriate validation, transformation, and UI rendering with proper units.
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::core::subtype::NumberSubtype;
//!
//! // Generic number
//! let subtype = NumberSubtype::Integer;
//!
//! // Temperature with unit
//! let subtype = NumberSubtype::Temperature;
//!
//! // Percentage (0-100)
//! let subtype = NumberSubtype::Percentage;
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic subtype for number parameters.
///
/// Subtypes provide hints for validation, transformation, and UI rendering.
/// They define what the number represents semantically, enabling proper
/// unit handling, range validation, and formatting.
///
/// # Categories
///
/// - **Generic**: Basic numeric types (Integer, Float, Decimal, Percentage)
/// - **Financial**: Money and pricing (Currency, Price, Tax, Discount)
/// - **Physical**: Measurements (Temperature, Distance, Weight, Volume, Speed)
/// - **Time**: Temporal values (Timestamp, Duration, Year, UnixTime)
/// - **Data**: Digital storage (Bytes, Bits, Bandwidth)
/// - **Geo**: Geographic coordinates (Latitude, Longitude, Altitude, Bearing)
/// - **Network**: Network values (Port, IpSegment, Latency)
/// - **Statistics**: Statistical values (Probability, Count, Rating, Score)
/// - **System**: System resources (Identifier, FileSize, MemorySize)
/// - **Custom**: User-defined numeric types (Custom)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberSubtype {
    // =========================================================================
    // Generic Numeric Types (4 variants)
    // =========================================================================
    /// Generic integer (no specific semantic meaning).
    ///
    /// # Use Cases
    /// - Count, index, quantity
    /// - Generic numeric input
    /// - No specific unit
    ///
    /// # UI Hint
    /// Integer input, no decimal places.
    Integer,

    /// Generic floating-point number.
    ///
    /// # Use Cases
    /// - Scientific calculations
    /// - Generic decimal values
    /// - No specific unit
    ///
    /// # UI Hint
    /// Decimal input with configurable precision.
    Float,

    /// High-precision decimal number.
    ///
    /// # Use Cases
    /// - Financial calculations
    /// - High-precision measurements
    /// - Cases where float rounding is unacceptable
    ///
    /// # UI Hint
    /// Decimal input with fixed precision.
    Decimal,

    /// Percentage value (typically 0-100).
    ///
    /// # Use Cases
    /// - Progress indicators
    /// - Discount rates
    /// - Completion status
    ///
    /// # Validation
    /// Typically 0-100, but can be configured.
    ///
    /// # UI Hint
    /// Percentage input with % symbol.
    ///
    /// # Example
    /// ```text
    /// 75.5% → 75.5
    /// ```
    Percentage,

    // =========================================================================
    // Financial (6 variants)
    // =========================================================================
    /// Currency amount (requires currency code in metadata).
    ///
    /// # Use Cases
    /// - Money values
    /// - Prices
    /// - Financial transactions
    ///
    /// # Precision
    /// Fixed decimal precision (typically 2 places).
    ///
    /// # UI Hint
    /// Currency input with currency symbol.
    ///
    /// # Example
    /// ```text
    /// $1,234.56
    /// €999.99
    /// ```
    Currency,

    /// Price value.
    ///
    /// # Use Cases
    /// - Product pricing
    /// - Service costs
    /// - Rate cards
    ///
    /// # UI Hint
    /// Price input with currency formatting.
    Price,

    /// Tax amount or rate.
    ///
    /// # Use Cases
    /// - Sales tax
    /// - VAT
    /// - Tax calculations
    ///
    /// # UI Hint
    /// Percentage or currency depending on context.
    Tax,

    /// Discount amount or percentage.
    ///
    /// # Use Cases
    /// - Sale discounts
    /// - Coupon values
    /// - Price reductions
    ///
    /// # UI Hint
    /// Percentage or currency with discount indicator.
    Discount,

    /// Interest rate (typically percentage).
    ///
    /// # Use Cases
    /// - Loan rates
    /// - Savings rates
    /// - APR/APY
    ///
    /// # UI Hint
    /// Percentage with high precision.
    InterestRate,

    /// Exchange rate between currencies.
    ///
    /// # Use Cases
    /// - Currency conversion
    /// - FX rates
    ///
    /// # UI Hint
    /// Decimal with high precision (4-6 places).
    ExchangeRate,

    // =========================================================================
    // Physical Measurements (14 variants)
    // =========================================================================
    /// Temperature measurement.
    ///
    /// # Use Cases
    /// - Weather data
    /// - Sensor readings
    /// - Climate control
    ///
    /// # Units
    /// Celsius, Fahrenheit, Kelvin (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with temperature unit selector.
    Temperature,

    /// Distance or length measurement.
    ///
    /// # Use Cases
    /// - Physical dimensions
    /// - Travel distance
    /// - Measurements
    ///
    /// # Units
    /// Meters, kilometers, miles, feet, inches (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with distance unit selector.
    Distance,

    /// Weight or mass measurement.
    ///
    /// # Use Cases
    /// - Product weight
    /// - Body weight
    /// - Cargo weight
    ///
    /// # Units
    /// Kilograms, grams, pounds, ounces (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with weight unit selector.
    Weight,

    /// Volume or capacity measurement.
    ///
    /// # Use Cases
    /// - Container capacity
    /// - Liquid volume
    /// - Storage space
    ///
    /// # Units
    /// Liters, milliliters, gallons, cups (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with volume unit selector.
    Volume,

    /// Area measurement.
    ///
    /// # Use Cases
    /// - Land area
    /// - Surface area
    /// - Coverage
    ///
    /// # Units
    /// Square meters, acres, square feet (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with area unit selector.
    Area,

    /// Speed or velocity measurement.
    ///
    /// # Use Cases
    /// - Vehicle speed
    /// - Wind speed
    /// - Data transfer rate
    ///
    /// # Units
    /// km/h, mph, m/s (store in metadata).
    ///
    /// # UI Hint
    /// Numeric input with speed unit selector.
    Speed,

    /// Acceleration measurement.
    ///
    /// # Use Cases
    /// - Physics calculations
    /// - Motion sensors
    ///
    /// # Units
    /// m/s², g-force (store in metadata).
    Acceleration,

    /// Force measurement.
    ///
    /// # Use Cases
    /// - Physics calculations
    /// - Engineering
    ///
    /// # Units
    /// Newtons, pounds-force (store in metadata).
    Force,

    /// Pressure measurement.
    ///
    /// # Use Cases
    /// - Atmospheric pressure
    /// - Tire pressure
    /// - Hydraulics
    ///
    /// # Units
    /// Pascals, PSI, bar, atmospheres (store in metadata).
    Pressure,

    /// Energy measurement.
    ///
    /// # Use Cases
    /// - Power consumption
    /// - Calorie tracking
    /// - Physics
    ///
    /// # Units
    /// Joules, calories, kWh (store in metadata).
    Energy,

    /// Power measurement.
    ///
    /// # Use Cases
    /// - Electrical power
    /// - Engine power
    /// - Device consumption
    ///
    /// # Units
    /// Watts, horsepower (store in metadata).
    Power,

    /// Frequency measurement.
    ///
    /// # Use Cases
    /// - Audio frequency
    /// - Radio frequency
    /// - CPU clock speed
    ///
    /// # Units
    /// Hertz (Hz, kHz, MHz, GHz) (store in metadata).
    Frequency,

    /// Angle measurement.
    ///
    /// # Use Cases
    /// - Rotation
    /// - Orientation
    /// - Geometry
    ///
    /// # Units
    /// Degrees, radians (store in metadata).
    Angle,

    /// Luminosity or brightness measurement.
    ///
    /// # Use Cases
    /// - Light intensity
    /// - Display brightness
    ///
    /// # Units
    /// Lumens, lux, candela (store in metadata).
    Luminosity,

    // =========================================================================
    // Time-related (6 variants)
    // =========================================================================
    /// Unix timestamp (seconds since epoch).
    ///
    /// # Use Cases
    /// - Event timestamps
    /// - Log entries
    /// - System time
    ///
    /// # Format
    /// Integer seconds since 1970-01-01T00:00:00Z.
    ///
    /// # UI Hint
    /// DateTime picker, stored as number.
    UnixTime,

    /// Unix timestamp in milliseconds.
    ///
    /// # Use Cases
    /// - High-precision timestamps
    /// - JavaScript timestamps
    ///
    /// # UI Hint
    /// DateTime picker, millisecond precision.
    UnixTimeMillis,

    /// Duration in seconds.
    ///
    /// # Use Cases
    /// - Timeouts
    /// - Intervals
    /// - Session length
    ///
    /// # UI Hint
    /// Duration input (hours, minutes, seconds).
    DurationSeconds,

    /// Duration in milliseconds.
    ///
    /// # Use Cases
    /// - Precise timing
    /// - Animation duration
    /// - Latency measurements
    ///
    /// # UI Hint
    /// Duration input with millisecond precision.
    DurationMillis,

    /// Year (integer).
    ///
    /// # Use Cases
    /// - Birth year
    /// - Historical dates
    /// - Year selection
    ///
    /// # Validation
    /// Typically 1900-2100 range.
    ///
    /// # UI Hint
    /// Year picker or numeric input.
    Year,

    /// Age (typically in years).
    ///
    /// # Use Cases
    /// - Person age
    /// - Account age
    /// - Product age
    ///
    /// # Validation
    /// Typically 0-150 range.
    ///
    /// # UI Hint
    /// Numeric input with age context.
    Age,

    // =========================================================================
    // Data Size (5 variants)
    // =========================================================================
    /// Data size in bytes.
    ///
    /// # Use Cases
    /// - File size
    /// - Memory size
    /// - Storage capacity
    ///
    /// # UI Hint
    /// Byte size input with auto-formatting (KB, MB, GB).
    Bytes,

    /// File size (alias for Bytes with file context).
    ///
    /// # Use Cases
    /// - Upload limits
    /// - Disk usage
    /// - Download size
    ///
    /// # UI Hint
    /// File size input with human-readable format.
    FileSize,

    /// Memory size.
    ///
    /// # Use Cases
    /// - RAM allocation
    /// - Cache size
    /// - Buffer size
    ///
    /// # UI Hint
    /// Memory size input (MB, GB).
    MemorySize,

    /// Data transfer rate.
    ///
    /// # Use Cases
    /// - Network bandwidth
    /// - Disk I/O
    /// - Transfer speed
    ///
    /// # Units
    /// Bytes per second, Mbps, Gbps (store in metadata).
    ///
    /// # UI Hint
    /// Bandwidth input with unit selector.
    Bandwidth,

    /// Bitrate for media.
    ///
    /// # Use Cases
    /// - Audio bitrate
    /// - Video bitrate
    /// - Streaming quality
    ///
    /// # Units
    /// Bits per second (kbps, Mbps).
    ///
    /// # UI Hint
    /// Bitrate input with quality presets.
    Bitrate,

    // =========================================================================
    // Geographic Coordinates (4 variants)
    // =========================================================================
    /// Latitude coordinate (-90 to +90).
    ///
    /// # Use Cases
    /// - GPS coordinates
    /// - Map locations
    /// - Geospatial data
    ///
    /// # Validation
    /// -90 to +90 degrees.
    ///
    /// # UI Hint
    /// Coordinate input with validation.
    ///
    /// # Example
    /// ```text
    /// 37.7749 (San Francisco)
    /// ```
    Latitude,

    /// Longitude coordinate (-180 to +180).
    ///
    /// # Use Cases
    /// - GPS coordinates
    /// - Map locations
    /// - Geospatial data
    ///
    /// # Validation
    /// -180 to +180 degrees.
    ///
    /// # UI Hint
    /// Coordinate input with validation.
    ///
    /// # Example
    /// ```text
    /// -122.4194 (San Francisco)
    /// ```
    Longitude,

    /// Altitude or elevation.
    ///
    /// # Use Cases
    /// - GPS altitude
    /// - Elevation data
    /// - Flight levels
    ///
    /// # Units
    /// Meters, feet (store in metadata).
    ///
    /// # UI Hint
    /// Altitude input with unit selector.
    Altitude,

    /// Compass bearing (0-360 degrees).
    ///
    /// # Use Cases
    /// - Direction
    /// - Heading
    /// - Navigation
    ///
    /// # Validation
    /// 0-360 degrees.
    ///
    /// # UI Hint
    /// Compass input or numeric 0-360.
    Bearing,

    // =========================================================================
    // Network and System (6 variants)
    // =========================================================================
    /// Network port number (0-65535).
    ///
    /// # Use Cases
    /// - Server configuration
    /// - Network services
    /// - Firewall rules
    ///
    /// # Validation
    /// 0-65535 (16-bit unsigned).
    ///
    /// # UI Hint
    /// Port number input with validation.
    Port,

    /// IPv4 address segment (0-255).
    ///
    /// # Use Cases
    /// - IP address input components
    /// - Subnet masks
    ///
    /// # Validation
    /// 0-255 (8-bit unsigned).
    IpV4Segment,

    /// Network latency in milliseconds.
    ///
    /// # Use Cases
    /// - Ping times
    /// - Network monitoring
    /// - Performance metrics
    ///
    /// # UI Hint
    /// Latency display with ms unit.
    Latency,

    /// Numeric identifier (process ID, thread ID, user ID, etc.).
    ///
    /// # Use Cases
    /// - Process/thread management (PID, TID)
    /// - User identification (UID)
    /// - System monitoring
    /// - Audit logs
    ///
    /// # UI Hint
    /// Integer ID input or display.
    ///
    /// # Example
    /// ```rust
    /// use nebula_parameter::core::subtype::NumberSubtype;
    ///
    /// // For process ID
    /// let pid = NumberSubtype::Identifier;
    ///
    /// // For user ID
    /// let uid = NumberSubtype::Identifier;
    /// ```
    Identifier,

    /// Return code or exit code.
    ///
    /// # Use Cases
    /// - Process exit status
    /// - Command results
    /// - Error codes
    ///
    /// # Validation
    /// Typically 0-255.
    ///
    /// # UI Hint
    /// Exit code display with status indicator.
    ExitCode,

    /// HTTP status code (100-599).
    ///
    /// # Use Cases
    /// - API responses
    /// - Web server logs
    /// - HTTP monitoring
    ///
    /// # Validation
    /// 100-599.
    ///
    /// # UI Hint
    /// Status code input with description.
    HttpStatusCode,

    // =========================================================================
    // Statistics and Counts (7 variants)
    // =========================================================================
    /// Probability value (0.0-1.0).
    ///
    /// # Use Cases
    /// - Statistical calculations
    /// - Machine learning
    /// - Risk assessment
    ///
    /// # Validation
    /// 0.0-1.0 (inclusive).
    ///
    /// # UI Hint
    /// Probability input, displayed as percentage.
    Probability,

    /// Count or quantity.
    ///
    /// # Use Cases
    /// - Item counts
    /// - Population size
    /// - Inventory
    ///
    /// # Validation
    /// Non-negative integer.
    ///
    /// # UI Hint
    /// Integer input, non-negative.
    Count,

    /// Array or list index (0-based).
    ///
    /// # Use Cases
    /// - Array indexing
    /// - Position in sequence
    ///
    /// # Validation
    /// Non-negative integer.
    ///
    /// # UI Hint
    /// Integer input, 0-based.
    Index,

    /// Rating value.
    ///
    /// # Use Cases
    /// - Star ratings
    /// - Review scores
    /// - Quality metrics
    ///
    /// # Validation
    /// Typically 0-5 or 0-10.
    ///
    /// # UI Hint
    /// Star rating or numeric scale.
    Rating,

    /// Score value.
    ///
    /// # Use Cases
    /// - Game scores
    /// - Test scores
    /// - Performance metrics
    ///
    /// # UI Hint
    /// Score display with context.
    Score,

    /// Rank or position (1-based).
    ///
    /// # Use Cases
    /// - Leaderboards
    /// - Search rankings
    /// - Ordering
    ///
    /// # Validation
    /// Positive integer (1-based).
    ///
    /// # UI Hint
    /// Rank display with ordinal formatting (1st, 2nd, 3rd).
    Rank,

    /// Priority level.
    ///
    /// # Use Cases
    /// - Task priority
    /// - Queue priority
    /// - Importance ranking
    ///
    /// # Validation
    /// Typically 0-10 or 1-5.
    ///
    /// # UI Hint
    /// Priority selector (Low/Medium/High).
    Priority,

    // =========================================================================
    // Custom (1 variant)
    // =========================================================================
    /// Custom numeric subtype defined by the user.
    ///
    /// # Use Cases
    /// - Domain-specific numbers
    /// - Application-specific validation
    ///
    /// # Example
    /// ```rust
    /// NumberSubtype::Custom("invoice_line_number".into())
    /// NumberSubtype::Custom("sku_quantity".into())
    /// ```
    Custom(String),
}

impl NumberSubtype {
    /// Create a Custom subtype.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::subtype::NumberSubtype;
    ///
    /// let subtype = NumberSubtype::custom("invoice_number");
    /// ```
    #[must_use]
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }

    /// Check if this subtype represents currency or money.
    #[must_use]
    pub fn is_financial(&self) -> bool {
        matches!(
            self,
            Self::Currency
                | Self::Price
                | Self::Tax
                | Self::Discount
                | Self::InterestRate
                | Self::ExchangeRate
        )
    }

    /// Check if this subtype represents a physical measurement.
    #[must_use]
    pub fn is_physical(&self) -> bool {
        matches!(
            self,
            Self::Temperature
                | Self::Distance
                | Self::Weight
                | Self::Volume
                | Self::Area
                | Self::Speed
                | Self::Acceleration
                | Self::Force
                | Self::Pressure
                | Self::Energy
                | Self::Power
                | Self::Frequency
                | Self::Angle
                | Self::Luminosity
        )
    }

    /// Check if this subtype represents a time-related value.
    #[must_use]
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::UnixTime
                | Self::UnixTimeMillis
                | Self::DurationSeconds
                | Self::DurationMillis
                | Self::Year
                | Self::Age
        )
    }

    /// Check if this subtype represents data size or bandwidth.
    #[must_use]
    pub fn is_data_size(&self) -> bool {
        matches!(
            self,
            Self::Bytes | Self::FileSize | Self::MemorySize | Self::Bandwidth | Self::Bitrate
        )
    }

    /// Check if this subtype represents a geographic coordinate.
    #[must_use]
    pub fn is_geographic(&self) -> bool {
        matches!(
            self,
            Self::Latitude | Self::Longitude | Self::Altitude | Self::Bearing
        )
    }

    /// Check if this subtype has a constrained range.
    ///
    /// Returns `true` for subtypes with well-defined min/max values.
    #[must_use]
    pub fn has_constrained_range(&self) -> bool {
        matches!(
            self,
            Self::Percentage
                | Self::Latitude
                | Self::Longitude
                | Self::Bearing
                | Self::Port
                | Self::IpV4Segment
                | Self::Probability
                | Self::HttpStatusCode
        )
    }

    /// Get the typical range for this subtype, if applicable.
    ///
    /// Returns (min, max) as floating-point values.
    #[must_use]
    pub fn typical_range(&self) -> Option<(f64, f64)> {
        match self {
            Self::Percentage => Some((0.0, 100.0)),
            Self::Latitude => Some((-90.0, 90.0)),
            Self::Longitude => Some((-180.0, 180.0)),
            Self::Bearing => Some((0.0, 360.0)),
            Self::Port => Some((0.0, 65535.0)),
            Self::IpV4Segment => Some((0.0, 255.0)),
            Self::Probability => Some((0.0, 1.0)),
            Self::HttpStatusCode => Some((100.0, 599.0)),
            Self::ExitCode => Some((0.0, 255.0)),
            Self::Rating => Some((0.0, 5.0)),    // Common default
            Self::Priority => Some((0.0, 10.0)), // Common default
            _ => None,
        }
    }

    /// Check if this subtype should typically be an integer.
    #[must_use]
    pub fn prefers_integer(&self) -> bool {
        matches!(
            self,
            Self::Integer
                | Self::Port
                | Self::IpV4Segment
                | Self::Identifier
                | Self::ExitCode
                | Self::HttpStatusCode
                | Self::Count
                | Self::Index
                | Self::Rank
                | Self::Priority
                | Self::Year
                | Self::Age
                | Self::UnixTime
                | Self::UnixTimeMillis
                | Self::DurationSeconds
                | Self::DurationMillis
        )
    }

    /// Check if this subtype should display with a unit suffix.
    #[must_use]
    pub fn has_display_unit(&self) -> bool {
        self.is_physical()
            || self.is_data_size()
            || matches!(
                self,
                Self::Percentage | Self::DurationSeconds | Self::DurationMillis
            )
    }

    /// Get common display unit for this subtype (if standardized).
    ///
    /// For subtypes with multiple possible units, returns the most common default.
    #[must_use]
    pub fn default_display_unit(&self) -> Option<&'static str> {
        match self {
            Self::Percentage => Some("%"),
            Self::Temperature => Some("°C"),
            Self::Distance => Some("m"),
            Self::Weight => Some("kg"),
            Self::Volume => Some("L"),
            Self::Area => Some("m²"),
            Self::Speed => Some("m/s"),
            Self::Acceleration => Some("m/s²"),
            Self::Force => Some("N"),
            Self::Pressure => Some("Pa"),
            Self::Energy => Some("J"),
            Self::Power => Some("W"),
            Self::Frequency => Some("Hz"),
            Self::Angle => Some("°"),
            Self::Luminosity => Some("lm"),
            Self::Bytes | Self::FileSize | Self::MemorySize => Some("bytes"),
            Self::Bandwidth => Some("Bps"),
            Self::Bitrate => Some("bps"),
            Self::DurationSeconds => Some("s"),
            Self::DurationMillis => Some("ms"),
            Self::Latency => Some("ms"),
            _ => None,
        }
    }
}

impl Default for NumberSubtype {
    fn default() -> Self {
        Self::Float
    }
}

impl fmt::Display for NumberSubtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer => write!(f, "integer"),
            Self::Float => write!(f, "float"),
            Self::Decimal => write!(f, "decimal"),
            Self::Percentage => write!(f, "percentage"),
            Self::Currency => write!(f, "currency"),
            Self::Price => write!(f, "price"),
            Self::Tax => write!(f, "tax"),
            Self::Discount => write!(f, "discount"),
            Self::InterestRate => write!(f, "interest_rate"),
            Self::ExchangeRate => write!(f, "exchange_rate"),
            Self::Temperature => write!(f, "temperature"),
            Self::Distance => write!(f, "distance"),
            Self::Weight => write!(f, "weight"),
            Self::Volume => write!(f, "volume"),
            Self::Area => write!(f, "area"),
            Self::Speed => write!(f, "speed"),
            Self::Acceleration => write!(f, "acceleration"),
            Self::Force => write!(f, "force"),
            Self::Pressure => write!(f, "pressure"),
            Self::Energy => write!(f, "energy"),
            Self::Power => write!(f, "power"),
            Self::Frequency => write!(f, "frequency"),
            Self::Angle => write!(f, "angle"),
            Self::Luminosity => write!(f, "luminosity"),
            Self::UnixTime => write!(f, "unix_time"),
            Self::UnixTimeMillis => write!(f, "unix_time_millis"),
            Self::DurationSeconds => write!(f, "duration_seconds"),
            Self::DurationMillis => write!(f, "duration_millis"),
            Self::Year => write!(f, "year"),
            Self::Age => write!(f, "age"),
            Self::Bytes => write!(f, "bytes"),
            Self::FileSize => write!(f, "file_size"),
            Self::MemorySize => write!(f, "memory_size"),
            Self::Bandwidth => write!(f, "bandwidth"),
            Self::Bitrate => write!(f, "bitrate"),
            Self::Latitude => write!(f, "latitude"),
            Self::Longitude => write!(f, "longitude"),
            Self::Altitude => write!(f, "altitude"),
            Self::Bearing => write!(f, "bearing"),
            Self::Port => write!(f, "port"),
            Self::IpV4Segment => write!(f, "ipv4_segment"),
            Self::Latency => write!(f, "latency"),
            Self::Identifier => write!(f, "identifier"),
            Self::ExitCode => write!(f, "exit_code"),
            Self::HttpStatusCode => write!(f, "http_status"),
            Self::Probability => write!(f, "probability"),
            Self::Count => write!(f, "count"),
            Self::Index => write!(f, "index"),
            Self::Rating => write!(f, "rating"),
            Self::Score => write!(f, "score"),
            Self::Rank => write!(f, "rank"),
            Self::Priority => write!(f, "priority"),
            Self::Custom(name) => write!(f, "custom({})", name),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        assert_eq!(NumberSubtype::default(), NumberSubtype::Float);
    }

    #[test]
    fn test_is_financial() {
        assert!(NumberSubtype::Currency.is_financial());
        assert!(NumberSubtype::Price.is_financial());
        assert!(NumberSubtype::Tax.is_financial());
        assert!(!NumberSubtype::Temperature.is_financial());
        assert!(!NumberSubtype::Integer.is_financial());
    }

    #[test]
    fn test_is_physical() {
        assert!(NumberSubtype::Temperature.is_physical());
        assert!(NumberSubtype::Distance.is_physical());
        assert!(NumberSubtype::Weight.is_physical());
        assert!(!NumberSubtype::Currency.is_physical());
        assert!(!NumberSubtype::Port.is_physical());
    }

    #[test]
    fn test_is_temporal() {
        assert!(NumberSubtype::UnixTime.is_temporal());
        assert!(NumberSubtype::DurationSeconds.is_temporal());
        assert!(NumberSubtype::Year.is_temporal());
        assert!(!NumberSubtype::Temperature.is_temporal());
    }

    #[test]
    fn test_is_data_size() {
        assert!(NumberSubtype::Bytes.is_data_size());
        assert!(NumberSubtype::FileSize.is_data_size());
        assert!(NumberSubtype::Bandwidth.is_data_size());
        assert!(!NumberSubtype::Temperature.is_data_size());
    }

    #[test]
    fn test_is_geographic() {
        assert!(NumberSubtype::Latitude.is_geographic());
        assert!(NumberSubtype::Longitude.is_geographic());
        assert!(NumberSubtype::Altitude.is_geographic());
        assert!(!NumberSubtype::Temperature.is_geographic());
    }

    #[test]
    fn test_has_constrained_range() {
        assert!(NumberSubtype::Percentage.has_constrained_range());
        assert!(NumberSubtype::Latitude.has_constrained_range());
        assert!(NumberSubtype::Port.has_constrained_range());
        assert!(!NumberSubtype::Temperature.has_constrained_range());
    }

    #[test]
    fn test_typical_range() {
        assert_eq!(
            NumberSubtype::Percentage.typical_range(),
            Some((0.0, 100.0))
        );
        assert_eq!(NumberSubtype::Latitude.typical_range(), Some((-90.0, 90.0)));
        assert_eq!(
            NumberSubtype::Longitude.typical_range(),
            Some((-180.0, 180.0))
        );
        assert_eq!(NumberSubtype::Port.typical_range(), Some((0.0, 65535.0)));
        assert_eq!(NumberSubtype::Temperature.typical_range(), None);
    }

    #[test]
    fn test_prefers_integer() {
        assert!(NumberSubtype::Integer.prefers_integer());
        assert!(NumberSubtype::Port.prefers_integer());
        assert!(NumberSubtype::Count.prefers_integer());
        assert!(!NumberSubtype::Float.prefers_integer());
        assert!(!NumberSubtype::Temperature.prefers_integer());
    }

    #[test]
    fn test_has_display_unit() {
        assert!(NumberSubtype::Percentage.has_display_unit());
        assert!(NumberSubtype::Temperature.has_display_unit());
        assert!(NumberSubtype::Bytes.has_display_unit());
        assert!(!NumberSubtype::Integer.has_display_unit());
    }

    #[test]
    fn test_default_display_unit() {
        assert_eq!(NumberSubtype::Percentage.default_display_unit(), Some("%"));
        assert_eq!(
            NumberSubtype::Temperature.default_display_unit(),
            Some("°C")
        );
        assert_eq!(NumberSubtype::Distance.default_display_unit(), Some("m"));
        assert_eq!(NumberSubtype::Integer.default_display_unit(), None);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", NumberSubtype::Integer), "integer");
        assert_eq!(format!("{}", NumberSubtype::Temperature), "temperature");
        assert_eq!(format!("{}", NumberSubtype::Currency), "currency");
        assert_eq!(
            format!("{}", NumberSubtype::Custom("sku".into())),
            "custom(sku)"
        );
    }

    #[test]
    fn test_serialization() {
        let subtype = NumberSubtype::Temperature;
        let json = serde_json::to_string(&subtype).unwrap();
        assert_eq!(json, "\"temperature\"");

        let deserialized: NumberSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, NumberSubtype::Temperature);
    }

    #[test]
    fn test_custom_serialization() {
        let subtype = NumberSubtype::Custom("invoice_number".into());
        let json = serde_json::to_string(&subtype).unwrap();
        let deserialized: NumberSubtype = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, subtype);
    }

    #[test]
    fn test_clone() {
        let subtype = NumberSubtype::Temperature;
        let cloned = subtype.clone();
        assert_eq!(subtype, cloned);
    }

    #[test]
    fn test_eq() {
        assert_eq!(NumberSubtype::Temperature, NumberSubtype::Temperature);
        assert_ne!(NumberSubtype::Temperature, NumberSubtype::Distance);
    }
}
