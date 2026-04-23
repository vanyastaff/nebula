use std::fmt;

use serde::{Deserialize, Serialize};

/// The kind of entity a slug identifies, which determines length constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlugKind {
    Org,            // 3–39 chars
    Workspace,      // 1–50 chars
    Workflow,       // 1–63 chars
    Credential,     // 1–63 chars
    Resource,       // 1–63 chars
    ServiceAccount, // 3–63 chars
    Trigger,        // 1–63 chars
}

impl SlugKind {
    #[must_use]
    pub const fn min_len(self) -> usize {
        match self {
            Self::Org | Self::ServiceAccount => 3,
            Self::Workspace
            | Self::Workflow
            | Self::Credential
            | Self::Resource
            | Self::Trigger => 1,
        }
    }

    #[must_use]
    pub const fn max_len(self) -> usize {
        match self {
            Self::Org => 39,
            Self::Workspace => 50,
            Self::Workflow
            | Self::Credential
            | Self::Resource
            | Self::ServiceAccount
            | Self::Trigger => 63,
        }
    }
}

/// A validated slug string. Immutable after construction.
///
/// Validation rules:
/// - Character set: `[a-z0-9-]`
/// - Must start and end with `[a-z0-9]`
/// - No consecutive hyphens
/// - Length per `SlugKind`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Slug(String);

impl Slug {
    /// Validate and create a slug for the given entity kind.
    pub fn new(value: &str, kind: SlugKind) -> Result<Self, SlugError> {
        validate_slug(value, kind)?;
        Ok(Self(value.to_owned()))
    }

    /// Create without validation (for trusted internal use only).
    #[must_use]
    pub fn new_unchecked(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Slug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Error returned when slug validation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    TooShort { min: usize, actual: usize },
    TooLong { max: usize, actual: usize },
    InvalidCharacter { position: usize, ch: char },
    InvalidStart,
    InvalidEnd,
    ConsecutiveHyphens,
    Reserved,
}

impl fmt::Display for SlugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { min, actual } => write!(f, "slug too short: {actual} < {min}"),
            Self::TooLong { max, actual } => write!(f, "slug too long: {actual} > {max}"),
            Self::InvalidCharacter { position, ch } => {
                write!(f, "invalid character '{ch}' at position {position}")
            },
            Self::InvalidStart => write!(f, "slug must start with [a-z0-9]"),
            Self::InvalidEnd => write!(f, "slug must end with [a-z0-9]"),
            Self::ConsecutiveHyphens => write!(f, "slug must not contain consecutive hyphens"),
            Self::Reserved => write!(f, "slug is reserved"),
        }
    }
}

impl std::error::Error for SlugError {}

/// Validate a slug string against the rules.
fn validate_slug(value: &str, kind: SlugKind) -> Result<(), SlugError> {
    let len = value.len();
    if len < kind.min_len() {
        return Err(SlugError::TooShort {
            min: kind.min_len(),
            actual: len,
        });
    }
    if len > kind.max_len() {
        return Err(SlugError::TooLong {
            max: kind.max_len(),
            actual: len,
        });
    }

    let bytes = value.as_bytes();

    // Must start with [a-z0-9]
    if !bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(SlugError::InvalidStart);
    }

    // Must end with [a-z0-9]
    if len > 1
        && !bytes
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(SlugError::InvalidEnd);
    }

    let mut prev_hyphen = false;
    for (i, &b) in bytes.iter().enumerate() {
        let valid = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        if !valid {
            return Err(SlugError::InvalidCharacter {
                position: i,
                ch: b as char,
            });
        }
        if b == b'-' {
            if prev_hyphen {
                return Err(SlugError::ConsecutiveHyphens);
            }
            prev_hyphen = true;
        } else {
            prev_hyphen = false;
        }
    }

    // Reserved slug check
    if is_reserved(value) {
        return Err(SlugError::Reserved);
    }

    Ok(())
}

/// Check if a slug matches a reserved word.
/// Uses a hardcoded set of common reserved words.
/// In production this would load from `reserved_slugs.txt`.
#[must_use]
pub fn is_reserved(slug: &str) -> bool {
    // Case-insensitive check (slugs are already lowercase, but be safe)
    let lower = slug.to_ascii_lowercase();
    RESERVED_SLUGS.contains(&lower.as_str())
}

/// Core reserved slugs. Extended list loaded at runtime from `reserved_slugs.txt`.
const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    "app",
    "auth",
    "billing",
    "blog",
    "cdn",
    "config",
    "console",
    "dashboard",
    "docs",
    "ftp",
    "git",
    "graphql",
    "grpc",
    "health",
    "help",
    "hooks",
    "internal",
    "login",
    "logout",
    "mail",
    "me",
    "metrics",
    "new",
    "null",
    "oauth",
    "openapi",
    "org",
    "orgs",
    "pricing",
    "ready",
    "root",
    "rss",
    "schema",
    "settings",
    "setup",
    "signup",
    "smtp",
    "sse",
    "status",
    "support",
    "system",
    "test",
    "undefined",
    "version",
    "webhook",
    "webhooks",
    "websocket",
    "ws",
    "wf",
    "www",
    "exe",
    "cred",
    "res",
    "usr",
    "svc",
    "sess",
    "trg",
    "evt",
    "att",
    "nbl",
    "wfv",
    "pat",
];

/// Detect whether a path segment is a ULID (prefixed) or a slug.
/// Returns true if it looks like a prefixed ULID (e.g., "org_01J9...").
#[must_use]
pub fn is_prefixed_ulid(segment: &str) -> bool {
    // Prefixed ULIDs have format: prefix_ULID where ULID is 26 chars of Crockford base32
    // Known prefixes: org_, ws_, wf_, wfv_, exe_, att_, nbl_, trg_, evt_, usr_, svc_, res_,
    // cred_, sess_, pat_
    const PREFIXES: &[&str] = &[
        "org_", "ws_", "wf_", "wfv_", "exe_", "att_", "nbl_", "trg_", "evt_", "usr_", "svc_",
        "res_", "cred_", "sess_", "pat_",
    ];
    PREFIXES.iter().any(|prefix| segment.starts_with(prefix))
}
