use core::fmt::{Display, Formatter};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ValueKind {
    String,
    Integer,
    Float,
    Boolean,
    Binary,
    Array,
    Object,
    Date,
    Time,
    DateTime,
    Duration,
    Regex,
}

impl Display for ValueKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ValueKind::String => write!(f, "string"),
            ValueKind::Integer => write!(f, "integer"),
            ValueKind::Float => write!(f, "float"),
            ValueKind::Boolean => write!(f, "boolean"),
            ValueKind::Binary => write!(f, "binary"),
            ValueKind::Array => write!(f, "array"),
            ValueKind::Object => write!(f, "object"),
            ValueKind::Date => write!(f, "date"),
            ValueKind::Time => write!(f, "time"),
            ValueKind::DateTime => write!(f, "datetime"),
            ValueKind::Duration => write!(f, "duration"),
            ValueKind::Regex => write!(f, "regex"),
        }
    }
}