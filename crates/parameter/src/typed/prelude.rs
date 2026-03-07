//! Prelude for the generic typed parameter API.

// ── Core generic types ───────────────────────────────────────────────────────
pub use crate::typed::checkbox::{Checkbox, CheckboxBuilder, CheckboxOptions};
pub use crate::typed::code::{Code, CodeBuilder};
pub use crate::typed::color::{Color, ColorBuilder};
pub use crate::typed::date::{Date, DateBuilder};
pub use crate::typed::datetime::{DateTime, DateTimeBuilder};
pub use crate::typed::expirable::{Expirable, ExpirableBuilder};
pub use crate::typed::group::{Group, GroupBuilder};
pub use crate::typed::hidden::{Hidden, HiddenBuilder};
pub use crate::typed::list::{List, ListBuilder};
pub use crate::typed::mode::{Mode, ModeBuilder};
pub use crate::typed::multi_select::{MultiSelect, MultiSelectBuilder};
pub use crate::typed::notice::{Notice, NoticeBuilder};
pub use crate::typed::number::{Number, NumberBuilder, NumberOptions};
pub use crate::typed::object::{Object, ObjectBuilder};
pub use crate::typed::secret::{Secret, SecretBuilder};
pub use crate::typed::select::{Select, SelectBuilder};
pub use crate::typed::text::{Text, TextBuilder, TextOptions};
pub use crate::typed::textarea::{Textarea, TextareaBuilder};
pub use crate::typed::time_picker::{TimePicker, TimePickerBuilder};

// Re-export Options types and enums
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

// ── Standard text subtypes ───────────────────────────────────────────────────
pub use crate::subtype::std_subtypes::{Email, Json, Password, Plain, Url, Uuid};

// ── Code subtypes (text-based) ───────────────────────────────────────────────
pub use crate::subtype::std_subtypes::{JavaScript, Markdown, Python, Rust, Shell, Sql, Yaml};

// ── Color subtypes (text-based) ──────────────────────────────────────────────
pub use crate::subtype::std_subtypes::{HexColor, HslColor, RgbColor};

// ── Date/Time subtypes (text-based) ──────────────────────────────────────────
pub use crate::subtype::std_subtypes::{Birthday, ExpiryDate, IsoDate, IsoDateTime, Time};

// ── Number subtypes ──────────────────────────────────────────────────────────
pub use crate::subtype::std_subtypes::{
    Distance, Factor, GenericNumber, Percentage, Port, Timestamp,
};

// ── Boolean subtypes ─────────────────────────────────────────────────────────
pub use crate::subtype::std_subtypes::{Consent, FeatureFlag, Toggle};

// ── Type aliases for common combinations ─────────────────────────────────────
pub use crate::typed::{
    // Text variants
    BirthdayParam,
    // Checkbox variants
    CheckboxParam,
    // Non-generic wrapper aliases
    CodeParam,
    ColorParam,
    ConsentParam,
    DateParam,
    DateTimeParam,
    // Number variants
    DistanceParam,
    EmailParam,
    ExpirableParam,
    ExpiryDateParam,
    FactorParam,
    FeatureFlagParam,
    GenericNumberParam,
    GroupParam,
    HexColorParam,
    HiddenParam,
    HslColorParam,
    IsoDateParam,
    IsoDateTimeParam,
    JavaScriptParam,
    JsonParam,
    ListParam,
    MarkdownParam,
    ModeParam,
    MultiSelectParam,
    NoticeParam,
    ObjectParam,
    PasswordParam,
    PercentageParam,
    PlainTextParam,
    PortParam,
    PythonParam,
    RgbColorParam,
    RustParam,
    SecretParam,
    SelectParam,
    ShellParam,
    SqlParam,
    TextareaParam,
    TimeParam,
    TimePickerParam,
    TimestampParam,
    UrlParam,
    UuidParam,
    YamlParam,
};
