//! Typed API with trait-based generic parameters.
//!
//! This module provides the default next-generation parameter system with:
//! - Type-safe subtypes via generics (`Text<Email>` vs `Text<Url>`)
//! - Compile-time subtype validation
//! - Extensible trait system for custom subtypes
//! - Robust serde serialization/deserialization
//! - Auto-validation and auto-constraint application
//!
//! ## Quick Start
//!
//! ```
//! use nebula_parameter::typed::{Text, Number, Email, Port};
//!
//! let email = Text::<Email>::builder("user_email")
//!     .label("Email Address")
//!     .required()
//!     .build();
//!
//! let port = Number::<Port>::builder("server_port")
//!     .label("Server Port")
//!     .default_value(8080)
//!     .build();
//! ```
//!
//! ## Type Aliases for Ergonomics
//!
//! ```
//! use nebula_parameter::typed::{EmailParam, UrlParam, PortParam};
//!
//! let email = EmailParam::builder("email").build();
//! let url = UrlParam::builder("homepage").build();
//! let port = PortParam::builder("port").build();
//! ```

// ── Core generic types ───────────────────────────────────────────────────────
pub mod checkbox;
pub mod code;
pub mod color;
pub mod date;
pub mod datetime;
pub mod expirable;
pub mod group;
pub mod hidden;
pub mod list;
pub mod mode;
pub mod multi_select;
pub mod notice;
pub mod number;
pub mod object;
pub mod prelude;
pub mod secret;
pub mod select;
pub mod text;
pub mod textarea;
pub mod time_picker;

pub use checkbox::{Checkbox, CheckboxBuilder, CheckboxOptions};
pub use code::{Code, CodeBuilder};
pub use color::{Color, ColorBuilder};
pub use date::{Date, DateBuilder};
pub use datetime::{DateTime, DateTimeBuilder};
pub use expirable::{Expirable, ExpirableBuilder};
pub use group::{Group, GroupBuilder};
pub use hidden::{Hidden, HiddenBuilder};
pub use list::{List, ListBuilder};
pub use mode::{Mode, ModeBuilder};
pub use multi_select::{MultiSelect, MultiSelectBuilder};
pub use notice::{Notice, NoticeBuilder};
pub use number::{Number, NumberBuilder, NumberOptions};
pub use object::{Object, ObjectBuilder};
pub use secret::{Secret, SecretBuilder};
pub use select::{Select, SelectBuilder};
pub use text::{Text, TextBuilder, TextOptions};
pub use textarea::{Textarea, TextareaBuilder};
pub use time_picker::{TimePicker, TimePickerBuilder};

// Re-export Options types from types/ module for consistency
pub use crate::types::code::{CodeLanguage, CodeOptions};
pub use crate::types::color::{ColorFormat, ColorOptions};
pub use crate::types::date::DateOptions;
pub use crate::types::datetime::DateTimeOptions;
pub use crate::types::expirable::ExpirableOptions;
pub use crate::types::group::GroupOptions;
pub use crate::types::list::ListOptions;
pub use crate::types::mode::{ModeOptions, ModeSelectorStyle, ModeVariant};
pub use crate::types::multi_select::MultiSelectOptions;
pub use crate::types::notice::NoticeType;
pub use crate::types::object::ObjectOptions;
pub use crate::types::secret::SecretOptions;
pub use crate::types::select::SelectOptions;
pub use crate::types::textarea::TextareaOptions;
pub use crate::types::time::TimeOptions;

// ── Standard subtypes ────────────────────────────────────────────────────────
pub use crate::subtype::std_subtypes::*;

// ── Type aliases for common combinations ─────────────────────────────────────

/// Text parameter with plain (no validation) subtype.
pub type PlainTextParam = Text<Plain>;

/// Text parameter with email validation.
pub type EmailParam = Text<Email>;

/// Text parameter with URL validation.
pub type UrlParam = Text<Url>;

/// Text parameter with password marking (sensitive).
pub type PasswordParam = Text<Password>;

/// Text parameter with JSON validation.
pub type JsonParam = Text<Json>;

/// Text parameter with UUID validation.
pub type UuidParam = Text<Uuid>;

/// Generic number parameter.
pub type GenericNumberParam = Number<GenericNumber>;

/// Number parameter constrained to port range (1-65535).
pub type PortParam = Number<Port>;

/// Number parameter constrained to percentage range (0-100).
pub type PercentageParam = Number<Percentage>;

/// Number parameter constrained to factor range (0.0-1.0).
pub type FactorParam = Number<Factor>;

/// Number parameter for Unix timestamps.
pub type TimestampParam = Number<Timestamp>;

/// Number parameter for distance values.
pub type DistanceParam = Number<Distance>;

/// Generic checkbox parameter.
pub type CheckboxParam = Checkbox<Toggle>;

/// Checkbox parameter for feature-flag use cases.
pub type FeatureFlagParam = Checkbox<FeatureFlag>;

/// Checkbox parameter for consent use cases.
pub type ConsentParam = Checkbox<Consent>;

// ── Non-generic typed wrappers ──────────────────────────────────────────────

/// Secret input parameter.
pub type SecretParam = Secret;

/// Multiline text input parameter.
pub type TextareaParam = Textarea;

/// Code editor parameter.
pub type CodeParam = Code;

/// Color picker parameter.
pub type ColorParam = Color;

/// Date picker parameter.
pub type DateParam = Date;

/// Date-time picker parameter.
pub type DateTimeParam = DateTime;

/// Time picker parameter.
pub type TimePickerParam = TimePicker;

/// Hidden value parameter.
pub type HiddenParam = Hidden;

/// Mutually-exclusive mode selector parameter.
pub type ModeParam = Mode;

/// Expirable wrapper parameter.
pub type ExpirableParam = Expirable;

/// UI-only group parameter.
pub type GroupParam = Group;

/// Repeatable list parameter.
pub type ListParam = List;

/// Display-only notice parameter.
pub type NoticeParam = Notice;

/// Object/grouped-fields parameter.
pub type ObjectParam = Object;

/// Single-select parameter.
pub type SelectParam = Select;

/// Multi-select parameter.
pub type MultiSelectParam = MultiSelect;

// ── Code subtypes (as Text) ──────────────────────────────────────────────────

/// Text parameter for JavaScript code.
pub type JavaScriptParam = Text<JavaScript>;

/// Text parameter for Python code.
pub type PythonParam = Text<Python>;

/// Text parameter for Rust code.
pub type RustParam = Text<Rust>;

/// Text parameter for SQL queries.
pub type SqlParam = Text<Sql>;

/// Text parameter for YAML configuration.
pub type YamlParam = Text<Yaml>;

/// Text parameter for shell scripts.
pub type ShellParam = Text<Shell>;

/// Text parameter for Markdown text.
pub type MarkdownParam = Text<Markdown>;

// ── Color subtypes (as Text) ─────────────────────────────────────────────────

/// Text parameter for hex color codes (#RRGGBB).
pub type HexColorParam = Text<HexColor>;

/// Text parameter for RGB color values.
pub type RgbColorParam = Text<RgbColor>;

/// Text parameter for HSL color values.
pub type HslColorParam = Text<HslColor>;

// ── Date/Time subtypes (as Text) ─────────────────────────────────────────────

/// Text parameter for ISO 8601 dates (YYYY-MM-DD).
pub type IsoDateParam = Text<IsoDate>;

/// Text parameter for ISO 8601 datetimes.
pub type IsoDateTimeParam = Text<IsoDateTime>;

/// Text parameter for time values (HH:MM:SS).
pub type TimeParam = Text<Time>;

/// Text parameter for birthday dates.
pub type BirthdayParam = Text<Birthday>;

/// Text parameter for expiry dates (MM/YY).
pub type ExpiryDateParam = Text<ExpiryDate>;
