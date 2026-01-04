//! Unit system for numeric parameters.
//!
//! This module provides comprehensive unit definitions for physical quantities,
//! data sizes, time, and other measurable values. Units enable:
//! - Automatic value conversion between units
//! - Proper display formatting
//! - Validation of unit compatibility
//! - Type-safe unit handling
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::core::unit::{Unit, TemperatureUnit};
//!
//! // Create temperature unit
//! let unit = Unit::Temperature(TemperatureUnit::Celsius);
//!
//! // Convert temperature
//! let celsius = 100.0;
//! let fahrenheit = TemperatureUnit::Celsius.convert_to(celsius, TemperatureUnit::Fahrenheit);
//! assert_eq!(fahrenheit, 212.0);
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

// =============================================================================
// Main Unit Enum
// =============================================================================

/// Unit of measurement for numeric values.
///
/// This enum encompasses all supported unit types, providing a unified
/// interface for unit handling across different physical quantities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// No unit (dimensionless value)
    None,

    /// Temperature units
    Temperature(TemperatureUnit),

    /// Distance/length units
    Distance(DistanceUnit),

    /// Weight/mass units
    Weight(WeightUnit),

    /// Volume/capacity units
    Volume(VolumeUnit),

    /// Area units
    Area(AreaUnit),

    /// Speed/velocity units
    Speed(SpeedUnit),

    /// Time duration units
    Duration(DurationUnit),

    /// Data size units
    DataSize(DataSizeUnit),

    /// Frequency units
    Frequency(FrequencyUnit),

    /// Angle units
    Angle(AngleUnit),

    /// Pressure units
    Pressure(PressureUnit),

    /// Energy units
    Energy(EnergyUnit),

    /// Power units
    Power(PowerUnit),

    /// Force units
    Force(ForceUnit),

    /// Acceleration units
    Acceleration(AccelerationUnit),

    /// Currency (ISO 4217 code)
    Currency(CurrencyCode),
}

impl Unit {
    /// Get the unit category name.
    #[must_use]
    pub fn category(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Temperature(_) => "temperature",
            Self::Distance(_) => "distance",
            Self::Weight(_) => "weight",
            Self::Volume(_) => "volume",
            Self::Area(_) => "area",
            Self::Speed(_) => "speed",
            Self::Duration(_) => "duration",
            Self::DataSize(_) => "data_size",
            Self::Frequency(_) => "frequency",
            Self::Angle(_) => "angle",
            Self::Pressure(_) => "pressure",
            Self::Energy(_) => "energy",
            Self::Power(_) => "power",
            Self::Force(_) => "force",
            Self::Acceleration(_) => "acceleration",
            Self::Currency(_) => "currency",
        }
    }

    /// Get the unit symbol for display.
    #[must_use]
    pub fn symbol(&self) -> &str {
        match self {
            Self::None => "",
            Self::Temperature(u) => u.symbol(),
            Self::Distance(u) => u.symbol(),
            Self::Weight(u) => u.symbol(),
            Self::Volume(u) => u.symbol(),
            Self::Area(u) => u.symbol(),
            Self::Speed(u) => u.symbol(),
            Self::Duration(u) => u.symbol(),
            Self::DataSize(u) => u.symbol(),
            Self::Frequency(u) => u.symbol(),
            Self::Angle(u) => u.symbol(),
            Self::Pressure(u) => u.symbol(),
            Self::Energy(u) => u.symbol(),
            Self::Power(u) => u.symbol(),
            Self::Force(u) => u.symbol(),
            Self::Acceleration(u) => u.symbol(),
            Self::Currency(code) => code.as_str(),
        }
    }

    /// Check if this unit is compatible with another unit (same category).
    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Default for Unit {
    fn default() -> Self {
        Self::None
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

// =============================================================================
// Temperature Units
// =============================================================================

/// Temperature measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureUnit {
    /// Celsius (°C)
    Celsius,
    /// Fahrenheit (°F)
    Fahrenheit,
    /// Kelvin (K)
    Kelvin,
}

impl TemperatureUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Fahrenheit => "°F",
            Self::Kelvin => "K",
        }
    }

    /// Convert value from this unit to another temperature unit.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::unit::TemperatureUnit;
    ///
    /// let celsius = 100.0;
    /// let fahrenheit = TemperatureUnit::Celsius.convert_to(celsius, TemperatureUnit::Fahrenheit);
    /// assert_eq!(fahrenheit, 212.0);
    ///
    /// let kelvin = TemperatureUnit::Celsius.convert_to(celsius, TemperatureUnit::Kelvin);
    /// assert_eq!(kelvin, 373.15);
    /// ```
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }

        // Convert to Celsius first
        let celsius = match self {
            Self::Celsius => value,
            Self::Fahrenheit => (value - 32.0) * 5.0 / 9.0,
            Self::Kelvin => value - 273.15,
        };

        // Convert from Celsius to target
        match target {
            Self::Celsius => celsius,
            Self::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
            Self::Kelvin => celsius + 273.15,
        }
    }
}

// =============================================================================
// Distance Units
// =============================================================================

/// Distance/length measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceUnit {
    /// Millimeters (mm)
    Millimeters,
    /// Centimeters (cm)
    Centimeters,
    /// Meters (m)
    Meters,
    /// Kilometers (km)
    Kilometers,
    /// Inches (in)
    Inches,
    /// Feet (ft)
    Feet,
    /// Yards (yd)
    Yards,
    /// Miles (mi)
    Miles,
    /// Nautical miles (NM)
    NauticalMiles,
}

impl DistanceUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Millimeters => "mm",
            Self::Centimeters => "cm",
            Self::Meters => "m",
            Self::Kilometers => "km",
            Self::Inches => "in",
            Self::Feet => "ft",
            Self::Yards => "yd",
            Self::Miles => "mi",
            Self::NauticalMiles => "NM",
        }
    }

    /// Get conversion factor to meters.
    #[must_use]
    fn to_meters(&self) -> f64 {
        match self {
            Self::Millimeters => 0.001,
            Self::Centimeters => 0.01,
            Self::Meters => 1.0,
            Self::Kilometers => 1000.0,
            Self::Inches => 0.0254,
            Self::Feet => 0.3048,
            Self::Yards => 0.9144,
            Self::Miles => 1609.344,
            Self::NauticalMiles => 1852.0,
        }
    }

    /// Convert value from this unit to another distance unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_meters() / target.to_meters()
    }
}

// =============================================================================
// Weight Units
// =============================================================================

/// Weight/mass measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeightUnit {
    /// Milligrams (mg)
    Milligrams,
    /// Grams (g)
    Grams,
    /// Kilograms (kg)
    Kilograms,
    /// Metric tons (t)
    Tonnes,
    /// Ounces (oz)
    Ounces,
    /// Pounds (lb)
    Pounds,
    /// Stones (st)
    Stones,
    /// Short tons (US ton)
    ShortTons,
    /// Long tons (UK ton)
    LongTons,
}

impl WeightUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Milligrams => "mg",
            Self::Grams => "g",
            Self::Kilograms => "kg",
            Self::Tonnes => "t",
            Self::Ounces => "oz",
            Self::Pounds => "lb",
            Self::Stones => "st",
            Self::ShortTons => "ton",
            Self::LongTons => "ton (UK)",
        }
    }

    /// Get conversion factor to kilograms.
    #[must_use]
    fn to_kilograms(&self) -> f64 {
        match self {
            Self::Milligrams => 0.000001,
            Self::Grams => 0.001,
            Self::Kilograms => 1.0,
            Self::Tonnes => 1000.0,
            Self::Ounces => 0.0283495,
            Self::Pounds => 0.453592,
            Self::Stones => 6.35029,
            Self::ShortTons => 907.185,
            Self::LongTons => 1016.05,
        }
    }

    /// Convert value from this unit to another weight unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_kilograms() / target.to_kilograms()
    }
}

// =============================================================================
// Volume Units
// =============================================================================

/// Volume/capacity measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeUnit {
    /// Milliliters (ml)
    Milliliters,
    /// Liters (L)
    Liters,
    /// Cubic meters (m³)
    CubicMeters,
    /// Teaspoons (tsp)
    Teaspoons,
    /// Tablespoons (tbsp)
    Tablespoons,
    /// Fluid ounces (fl oz)
    FluidOunces,
    /// Cups
    Cups,
    /// Pints (pt)
    Pints,
    /// Quarts (qt)
    Quarts,
    /// Gallons (gal)
    Gallons,
}

impl VolumeUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Milliliters => "ml",
            Self::Liters => "L",
            Self::CubicMeters => "m³",
            Self::Teaspoons => "tsp",
            Self::Tablespoons => "tbsp",
            Self::FluidOunces => "fl oz",
            Self::Cups => "cup",
            Self::Pints => "pt",
            Self::Quarts => "qt",
            Self::Gallons => "gal",
        }
    }

    /// Get conversion factor to liters.
    #[must_use]
    fn to_liters(&self) -> f64 {
        match self {
            Self::Milliliters => 0.001,
            Self::Liters => 1.0,
            Self::CubicMeters => 1000.0,
            Self::Teaspoons => 0.00492892,
            Self::Tablespoons => 0.0147868,
            Self::FluidOunces => 0.0295735,
            Self::Cups => 0.24,
            Self::Pints => 0.473176,
            Self::Quarts => 0.946353,
            Self::Gallons => 3.78541,
        }
    }

    /// Convert value from this unit to another volume unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_liters() / target.to_liters()
    }
}

// =============================================================================
// Area Units
// =============================================================================

/// Area measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AreaUnit {
    /// Square millimeters (mm²)
    SquareMillimeters,
    /// Square centimeters (cm²)
    SquareCentimeters,
    /// Square meters (m²)
    SquareMeters,
    /// Hectares (ha)
    Hectares,
    /// Square kilometers (km²)
    SquareKilometers,
    /// Square inches (in²)
    SquareInches,
    /// Square feet (ft²)
    SquareFeet,
    /// Square yards (yd²)
    SquareYards,
    /// Acres (ac)
    Acres,
    /// Square miles (mi²)
    SquareMiles,
}

impl AreaUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::SquareMillimeters => "mm²",
            Self::SquareCentimeters => "cm²",
            Self::SquareMeters => "m²",
            Self::Hectares => "ha",
            Self::SquareKilometers => "km²",
            Self::SquareInches => "in²",
            Self::SquareFeet => "ft²",
            Self::SquareYards => "yd²",
            Self::Acres => "ac",
            Self::SquareMiles => "mi²",
        }
    }

    /// Get conversion factor to square meters.
    #[must_use]
    fn to_square_meters(&self) -> f64 {
        match self {
            Self::SquareMillimeters => 0.000001,
            Self::SquareCentimeters => 0.0001,
            Self::SquareMeters => 1.0,
            Self::Hectares => 10000.0,
            Self::SquareKilometers => 1_000_000.0,
            Self::SquareInches => 0.00064516,
            Self::SquareFeet => 0.092903,
            Self::SquareYards => 0.836127,
            Self::Acres => 4046.86,
            Self::SquareMiles => 2_589_988.0,
        }
    }

    /// Convert value from this unit to another area unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_square_meters() / target.to_square_meters()
    }
}

// =============================================================================
// Speed Units
// =============================================================================

/// Speed/velocity measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeedUnit {
    /// Meters per second (m/s)
    MetersPerSecond,
    /// Kilometers per hour (km/h)
    KilometersPerHour,
    /// Miles per hour (mph)
    MilesPerHour,
    /// Feet per second (ft/s)
    FeetPerSecond,
    /// Knots (kn)
    Knots,
}

impl SpeedUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::MetersPerSecond => "m/s",
            Self::KilometersPerHour => "km/h",
            Self::MilesPerHour => "mph",
            Self::FeetPerSecond => "ft/s",
            Self::Knots => "kn",
        }
    }

    /// Get conversion factor to meters per second.
    #[must_use]
    fn to_meters_per_second(&self) -> f64 {
        match self {
            Self::MetersPerSecond => 1.0,
            Self::KilometersPerHour => 0.277778,
            Self::MilesPerHour => 0.44704,
            Self::FeetPerSecond => 0.3048,
            Self::Knots => 0.514444,
        }
    }

    /// Convert value from this unit to another speed unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_meters_per_second() / target.to_meters_per_second()
    }
}

// =============================================================================
// Duration Units
// =============================================================================

/// Time duration units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurationUnit {
    /// Nanoseconds (ns)
    Nanoseconds,
    /// Microseconds (μs)
    Microseconds,
    /// Milliseconds (ms)
    Milliseconds,
    /// Seconds (s)
    Seconds,
    /// Minutes (min)
    Minutes,
    /// Hours (h)
    Hours,
    /// Days (d)
    Days,
    /// Weeks (w)
    Weeks,
}

impl DurationUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Nanoseconds => "ns",
            Self::Microseconds => "μs",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Minutes => "min",
            Self::Hours => "h",
            Self::Days => "d",
            Self::Weeks => "w",
        }
    }

    /// Get conversion factor to seconds.
    #[must_use]
    fn to_seconds(&self) -> f64 {
        match self {
            Self::Nanoseconds => 0.000_000_001,
            Self::Microseconds => 0.000_001,
            Self::Milliseconds => 0.001,
            Self::Seconds => 1.0,
            Self::Minutes => 60.0,
            Self::Hours => 3600.0,
            Self::Days => 86400.0,
            Self::Weeks => 604800.0,
        }
    }

    /// Convert value from this unit to another duration unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_seconds() / target.to_seconds()
    }
}

// =============================================================================
// Data Size Units
// =============================================================================

/// Data size units (binary, base-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataSizeUnit {
    /// Bytes (B)
    Bytes,
    /// Kilobytes (KB) - 1024 bytes
    Kilobytes,
    /// Megabytes (MB) - 1024 KB
    Megabytes,
    /// Gigabytes (GB) - 1024 MB
    Gigabytes,
    /// Terabytes (TB) - 1024 GB
    Terabytes,
    /// Petabytes (PB) - 1024 TB
    Petabytes,
}

impl DataSizeUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Bytes => "B",
            Self::Kilobytes => "KB",
            Self::Megabytes => "MB",
            Self::Gigabytes => "GB",
            Self::Terabytes => "TB",
            Self::Petabytes => "PB",
        }
    }

    /// Get conversion factor to bytes (base-2).
    #[must_use]
    fn to_bytes(&self) -> f64 {
        match self {
            Self::Bytes => 1.0,
            Self::Kilobytes => 1024.0,
            Self::Megabytes => 1024.0_f64.powi(2),
            Self::Gigabytes => 1024.0_f64.powi(3),
            Self::Terabytes => 1024.0_f64.powi(4),
            Self::Petabytes => 1024.0_f64.powi(5),
        }
    }

    /// Convert value from this unit to another data size unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_bytes() / target.to_bytes()
    }

    /// Format byte count with appropriate unit.
    ///
    /// Automatically selects the best unit for display.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::unit::DataSizeUnit;
    ///
    /// assert_eq!(DataSizeUnit::format_bytes(1024.0), "1.00 KB");
    /// assert_eq!(DataSizeUnit::format_bytes(1_048_576.0), "1.00 MB");
    /// ```
    #[must_use]
    pub fn format_bytes(bytes: f64) -> String {
        const UNITS: [DataSizeUnit; 6] = [
            DataSizeUnit::Petabytes,
            DataSizeUnit::Terabytes,
            DataSizeUnit::Gigabytes,
            DataSizeUnit::Megabytes,
            DataSizeUnit::Kilobytes,
            DataSizeUnit::Bytes,
        ];

        for unit in UNITS {
            let threshold = unit.to_bytes();
            if bytes >= threshold {
                let value = bytes / threshold;
                return format!("{:.2} {}", value, unit.symbol());
            }
        }

        format!("{:.2} B", bytes)
    }
}

// =============================================================================
// Frequency Units
// =============================================================================

/// Frequency measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrequencyUnit {
    /// Hertz (Hz)
    Hertz,
    /// Kilohertz (kHz)
    Kilohertz,
    /// Megahertz (MHz)
    Megahertz,
    /// Gigahertz (GHz)
    Gigahertz,
}

impl FrequencyUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Hertz => "Hz",
            Self::Kilohertz => "kHz",
            Self::Megahertz => "MHz",
            Self::Gigahertz => "GHz",
        }
    }

    /// Get conversion factor to Hertz.
    #[must_use]
    fn to_hertz(&self) -> f64 {
        match self {
            Self::Hertz => 1.0,
            Self::Kilohertz => 1_000.0,
            Self::Megahertz => 1_000_000.0,
            Self::Gigahertz => 1_000_000_000.0,
        }
    }

    /// Convert value from this unit to another frequency unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_hertz() / target.to_hertz()
    }
}

// =============================================================================
// Angle Units
// =============================================================================

/// Angle measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AngleUnit {
    /// Degrees (°)
    Degrees,
    /// Radians (rad)
    Radians,
    /// Gradians (grad)
    Gradians,
}

impl AngleUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Degrees => "°",
            Self::Radians => "rad",
            Self::Gradians => "grad",
        }
    }

    /// Convert value from this unit to another angle unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }

        // Convert to degrees first
        let degrees = match self {
            Self::Degrees => value,
            Self::Radians => value * 180.0 / std::f64::consts::PI,
            Self::Gradians => value * 0.9,
        };

        // Convert from degrees to target
        match target {
            Self::Degrees => degrees,
            Self::Radians => degrees * std::f64::consts::PI / 180.0,
            Self::Gradians => degrees / 0.9,
        }
    }
}

// =============================================================================
// Pressure Units
// =============================================================================

/// Pressure measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PressureUnit {
    /// Pascals (Pa)
    Pascals,
    /// Kilopascals (kPa)
    Kilopascals,
    /// Megapascals (MPa)
    Megapascals,
    /// Bar (bar)
    Bar,
    /// Pounds per square inch (PSI)
    Psi,
    /// Atmospheres (atm)
    Atmospheres,
    /// Millimeters of mercury (mmHg)
    MillimetersOfMercury,
}

impl PressureUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Pascals => "Pa",
            Self::Kilopascals => "kPa",
            Self::Megapascals => "MPa",
            Self::Bar => "bar",
            Self::Psi => "PSI",
            Self::Atmospheres => "atm",
            Self::MillimetersOfMercury => "mmHg",
        }
    }

    /// Get conversion factor to Pascals.
    #[must_use]
    fn to_pascals(&self) -> f64 {
        match self {
            Self::Pascals => 1.0,
            Self::Kilopascals => 1_000.0,
            Self::Megapascals => 1_000_000.0,
            Self::Bar => 100_000.0,
            Self::Psi => 6894.76,
            Self::Atmospheres => 101_325.0,
            Self::MillimetersOfMercury => 133.322,
        }
    }

    /// Convert value from this unit to another pressure unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_pascals() / target.to_pascals()
    }
}

// =============================================================================
// Energy Units
// =============================================================================

/// Energy measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnergyUnit {
    /// Joules (J)
    Joules,
    /// Kilojoules (kJ)
    Kilojoules,
    /// Calories (cal)
    Calories,
    /// Kilocalories (kcal)
    Kilocalories,
    /// Watt-hours (Wh)
    WattHours,
    /// Kilowatt-hours (kWh)
    KilowattHours,
}

impl EnergyUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Joules => "J",
            Self::Kilojoules => "kJ",
            Self::Calories => "cal",
            Self::Kilocalories => "kcal",
            Self::WattHours => "Wh",
            Self::KilowattHours => "kWh",
        }
    }

    /// Get conversion factor to Joules.
    #[must_use]
    fn to_joules(&self) -> f64 {
        match self {
            Self::Joules => 1.0,
            Self::Kilojoules => 1_000.0,
            Self::Calories => 4.184,
            Self::Kilocalories => 4_184.0,
            Self::WattHours => 3_600.0,
            Self::KilowattHours => 3_600_000.0,
        }
    }

    /// Convert value from this unit to another energy unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_joules() / target.to_joules()
    }
}

// =============================================================================
// Power Units
// =============================================================================

/// Power measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerUnit {
    /// Watts (W)
    Watts,
    /// Kilowatts (kW)
    Kilowatts,
    /// Megawatts (MW)
    Megawatts,
    /// Horsepower (hp)
    Horsepower,
}

impl PowerUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Watts => "W",
            Self::Kilowatts => "kW",
            Self::Megawatts => "MW",
            Self::Horsepower => "hp",
        }
    }

    /// Get conversion factor to Watts.
    #[must_use]
    fn to_watts(&self) -> f64 {
        match self {
            Self::Watts => 1.0,
            Self::Kilowatts => 1_000.0,
            Self::Megawatts => 1_000_000.0,
            Self::Horsepower => 745.7,
        }
    }

    /// Convert value from this unit to another power unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_watts() / target.to_watts()
    }
}

// =============================================================================
// Force Units
// =============================================================================

/// Force measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForceUnit {
    /// Newtons (N)
    Newtons,
    /// Kilonewtons (kN)
    Kilonewtons,
    /// Pounds-force (lbf)
    PoundsForce,
}

impl ForceUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Newtons => "N",
            Self::Kilonewtons => "kN",
            Self::PoundsForce => "lbf",
        }
    }

    /// Get conversion factor to Newtons.
    #[must_use]
    fn to_newtons(&self) -> f64 {
        match self {
            Self::Newtons => 1.0,
            Self::Kilonewtons => 1_000.0,
            Self::PoundsForce => 4.44822,
        }
    }

    /// Convert value from this unit to another force unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_newtons() / target.to_newtons()
    }
}

// =============================================================================
// Acceleration Units
// =============================================================================

/// Acceleration measurement units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccelerationUnit {
    /// Meters per second squared (m/s²)
    MetersPerSecondSquared,
    /// Feet per second squared (ft/s²)
    FeetPerSecondSquared,
    /// Standard gravity (g)
    StandardGravity,
}

impl AccelerationUnit {
    /// Get the unit symbol.
    #[must_use]
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::MetersPerSecondSquared => "m/s²",
            Self::FeetPerSecondSquared => "ft/s²",
            Self::StandardGravity => "g",
        }
    }

    /// Get conversion factor to m/s².
    #[must_use]
    fn to_meters_per_second_squared(&self) -> f64 {
        match self {
            Self::MetersPerSecondSquared => 1.0,
            Self::FeetPerSecondSquared => 0.3048,
            Self::StandardGravity => 9.80665,
        }
    }

    /// Convert value from this unit to another acceleration unit.
    #[must_use]
    pub fn convert_to(&self, value: f64, target: Self) -> f64 {
        if self == &target {
            return value;
        }
        value * self.to_meters_per_second_squared() / target.to_meters_per_second_squared()
    }
}

// =============================================================================
// Currency Code (ISO 4217)
// =============================================================================

/// Currency code (ISO 4217).
///
/// For comprehensive list, see: https://en.wikipedia.org/wiki/ISO_4217
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CurrencyCode([u8; 3]);

impl CurrencyCode {
    /// Create a new currency code.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::core::unit::CurrencyCode;
    ///
    /// let usd = CurrencyCode::new("USD");
    /// let eur = CurrencyCode::new("EUR");
    /// ```
    #[must_use]
    pub fn new(code: &str) -> Self {
        let bytes = code.as_bytes();
        let mut arr = [0u8; 3];
        let len = bytes.len().min(3);
        arr[..len].copy_from_slice(&bytes[..len]);
        Self(arr)
    }

    /// Get the currency code as string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("???")
    }

    /// Common currency codes
    pub const USD: Self = Self([b'U', b'S', b'D']);
    pub const EUR: Self = Self([b'E', b'U', b'R']);
    pub const GBP: Self = Self([b'G', b'B', b'P']);
    pub const JPY: Self = Self([b'J', b'P', b'Y']);
    pub const CNY: Self = Self([b'C', b'N', b'Y']);
    pub const CHF: Self = Self([b'C', b'H', b'F']);
    pub const AUD: Self = Self([b'A', b'U', b'D']);
    pub const CAD: Self = Self([b'C', b'A', b'D']);
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temperature_conversion() {
        let celsius = 100.0;
        let fahrenheit = TemperatureUnit::Celsius.convert_to(celsius, TemperatureUnit::Fahrenheit);
        assert!((fahrenheit - 212.0).abs() < 0.01);

        let kelvin = TemperatureUnit::Celsius.convert_to(celsius, TemperatureUnit::Kelvin);
        assert!((kelvin - 373.15).abs() < 0.01);
    }

    #[test]
    fn test_distance_conversion() {
        let meters = 1000.0;
        let km = DistanceUnit::Meters.convert_to(meters, DistanceUnit::Kilometers);
        assert!((km - 1.0).abs() < 0.01);

        let miles = DistanceUnit::Kilometers.convert_to(1.0, DistanceUnit::Miles);
        assert!((miles - 0.621371).abs() < 0.001);
    }

    #[test]
    fn test_data_size_conversion() {
        let kb = 1.0;
        let bytes = DataSizeUnit::Kilobytes.convert_to(kb, DataSizeUnit::Bytes);
        assert_eq!(bytes, 1024.0);

        let mb = DataSizeUnit::Kilobytes.convert_to(1024.0, DataSizeUnit::Megabytes);
        assert_eq!(mb, 1.0);
    }

    #[test]
    fn test_data_size_formatting() {
        assert_eq!(DataSizeUnit::format_bytes(1024.0), "1.00 KB");
        assert_eq!(DataSizeUnit::format_bytes(1_048_576.0), "1.00 MB");
        assert_eq!(DataSizeUnit::format_bytes(500.0), "500.00 B");
    }

    #[test]
    fn test_unit_symbol() {
        assert_eq!(TemperatureUnit::Celsius.symbol(), "°C");
        assert_eq!(DistanceUnit::Meters.symbol(), "m");
        assert_eq!(WeightUnit::Kilograms.symbol(), "kg");
    }

    #[test]
    fn test_unit_compatibility() {
        let temp = Unit::Temperature(TemperatureUnit::Celsius);
        let temp2 = Unit::Temperature(TemperatureUnit::Fahrenheit);
        let dist = Unit::Distance(DistanceUnit::Meters);

        assert!(temp.is_compatible_with(&temp2));
        assert!(!temp.is_compatible_with(&dist));
    }

    #[test]
    fn test_currency_code() {
        let usd = CurrencyCode::new("USD");
        assert_eq!(usd.as_str(), "USD");
        assert_eq!(CurrencyCode::USD.as_str(), "USD");
    }
}
