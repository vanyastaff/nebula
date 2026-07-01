use std::fmt;

use serde::{Deserialize, Serialize};

/// The kind of entity a slug identifies, which determines length constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
#[non_exhaustive]
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
/// Uses a hardcoded set of reserved words (see `RESERVED_SLUGS` in this module).
#[must_use]
pub fn is_reserved(slug: &str) -> bool {
    // Case-insensitive check (slugs are already lowercase, but be safe)
    let lower = slug.to_ascii_lowercase();
    RESERVED_SLUGS.contains(&lower.as_str())
}

/// Reserved slugs that may not be used as org/workspace/workflow slugs.
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Slug::new valid ──────────────────────────────────────────────────────

    #[test]
    fn slug_new_valid_org() {
        let slug = Slug::new("my-org", SlugKind::Org).unwrap();
        assert_eq!(slug.as_str(), "my-org");
    }

    #[test]
    fn slug_new_valid_workflow() {
        let slug = Slug::new("a", SlugKind::Workflow).unwrap();
        assert_eq!(slug.as_str(), "a");
    }

    #[test]
    fn slug_new_valid_digits_and_hyphens() {
        let slug = Slug::new("abc-123-def", SlugKind::Workflow).unwrap();
        assert_eq!(slug.as_str(), "abc-123-def");
    }

    // ── TooShort ────────────────────────────────────────────────────────────

    #[test]
    fn slug_new_too_short_org() {
        // Org min is 3; "ab" is only 2 chars.
        let err = Slug::new("ab", SlugKind::Org).unwrap_err();
        assert!(
            matches!(err, SlugError::TooShort { min: 3, actual: 2 }),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn slug_new_too_short_service_account() {
        // ServiceAccount min is 3; "a" is only 1 char.
        let err = Slug::new("a", SlugKind::ServiceAccount).unwrap_err();
        assert!(
            matches!(err, SlugError::TooShort { min: 3, .. }),
            "unexpected: {err:?}"
        );
    }

    // ── TooLong ─────────────────────────────────────────────────────────────

    #[test]
    fn slug_new_too_long_org() {
        // Org max is 39 chars.
        let long = "a".repeat(40);
        let err = Slug::new(&long, SlugKind::Org).unwrap_err();
        assert!(
            matches!(
                err,
                SlugError::TooLong {
                    max: 39,
                    actual: 40
                }
            ),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn slug_new_too_long_workspace() {
        // Workspace max is 50 chars.
        let long = "a".repeat(51);
        let err = Slug::new(&long, SlugKind::Workspace).unwrap_err();
        assert!(
            matches!(
                err,
                SlugError::TooLong {
                    max: 50,
                    actual: 51
                }
            ),
            "unexpected: {err:?}"
        );
    }

    // ── InvalidCharacter ────────────────────────────────────────────────────

    #[test]
    fn slug_new_invalid_char_uppercase() {
        // Uppercase at position 0 is caught by the start-character check first,
        // which produces InvalidStart rather than InvalidCharacter.
        let err = Slug::new("My-Org", SlugKind::Workflow).unwrap_err();
        assert!(
            matches!(err, SlugError::InvalidStart),
            "unexpected: {err:?}"
        );
        // Uppercase at a non-start position reaches the per-char loop.
        let err2 = Slug::new("my-Org", SlugKind::Workflow).unwrap_err();
        assert!(
            matches!(
                err2,
                SlugError::InvalidCharacter {
                    ch: 'O',
                    position: 3
                }
            ),
            "unexpected: {err2:?}"
        );
    }

    #[test]
    fn slug_new_invalid_char_underscore() {
        let err = Slug::new("my_org", SlugKind::Workflow).unwrap_err();
        assert!(
            matches!(err, SlugError::InvalidCharacter { ch: '_', .. }),
            "unexpected: {err:?}"
        );
    }

    // ── InvalidStart ────────────────────────────────────────────────────────

    #[test]
    fn slug_new_invalid_start_hyphen() {
        let err = Slug::new("-my-org", SlugKind::Workflow).unwrap_err();
        assert!(
            matches!(err, SlugError::InvalidStart),
            "unexpected: {err:?}"
        );
    }

    // ── InvalidEnd ──────────────────────────────────────────────────────────

    #[test]
    fn slug_new_invalid_end_hyphen() {
        let err = Slug::new("my-org-", SlugKind::Workflow).unwrap_err();
        assert!(matches!(err, SlugError::InvalidEnd), "unexpected: {err:?}");
    }

    // ── ConsecutiveHyphens ──────────────────────────────────────────────────

    #[test]
    fn slug_new_consecutive_hyphens() {
        let err = Slug::new("my--org", SlugKind::Workflow).unwrap_err();
        assert!(
            matches!(err, SlugError::ConsecutiveHyphens),
            "unexpected: {err:?}"
        );
    }

    // ── Reserved ────────────────────────────────────────────────────────────

    #[test]
    fn slug_new_reserved_word() {
        let err = Slug::new("admin", SlugKind::Workflow).unwrap_err();
        assert!(matches!(err, SlugError::Reserved), "unexpected: {err:?}");
    }

    #[test]
    fn slug_new_reserved_word_api() {
        let err = Slug::new("api", SlugKind::Org).unwrap_err();
        assert!(matches!(err, SlugError::Reserved), "unexpected: {err:?}");
    }

    // ── is_reserved ─────────────────────────────────────────────────────────

    #[test]
    fn is_reserved_returns_true_for_reserved_word() {
        assert!(is_reserved("admin"));
        assert!(is_reserved("api"));
        assert!(is_reserved("webhook"));
    }

    #[test]
    fn is_reserved_returns_false_for_ordinary_slug() {
        assert!(!is_reserved("my-cool-workflow"));
        assert!(!is_reserved("acme-corp"));
    }

    // ── is_prefixed_ulid ────────────────────────────────────────────────────

    #[test]
    fn is_prefixed_ulid_true_for_each_known_prefix() {
        // One representative value per recognized prefix — format doesn't matter for the
        // prefix-existence check (is_prefixed_ulid only checks starts_with).
        let samples = [
            "org_01ABC",
            "ws_01ABC",
            "wf_01ABC",
            "wfv_01ABC",
            "exe_01ABC",
            "att_01ABC",
            "nbl_01ABC",
            "trg_01ABC",
            "evt_01ABC",
            "usr_01ABC",
            "svc_01ABC",
            "res_01ABC",
            "cred_01ABC",
            "sess_01ABC",
            "pat_01ABC",
        ];
        for sample in samples {
            assert!(
                is_prefixed_ulid(sample),
                "expected is_prefixed_ulid({sample:?}) == true"
            );
        }
    }

    #[test]
    fn is_prefixed_ulid_false_for_plain_slug() {
        assert!(!is_prefixed_ulid("my-workflow"));
        assert!(!is_prefixed_ulid("acme-corp"));
        assert!(!is_prefixed_ulid(""));
    }

    // ── B6: drift-guard — prefix list matches live ID types ─────────────────

    #[test]
    fn prefixed_ulid_recognizes_live_id_type_strings() {
        use crate::id::{
            AttemptId, CredentialId, ExecutionId, InstanceId, OrgId, ResourceId, ServiceAccountId,
            SessionId, TriggerEventId, TriggerId, UserId, WorkflowId, WorkflowVersionId,
            WorkspaceId,
        };

        macro_rules! assert_recognized {
            ($id_type:ident) => {{
                let id = $id_type::new();
                let as_string = id.to_string();
                assert!(
                    is_prefixed_ulid(&as_string),
                    "is_prefixed_ulid should recognize a freshly-constructed {}: {as_string}",
                    stringify!($id_type)
                );
            }};
        }

        assert_recognized!(OrgId);
        assert_recognized!(WorkspaceId);
        assert_recognized!(WorkflowId);
        assert_recognized!(WorkflowVersionId);
        assert_recognized!(ExecutionId);
        assert_recognized!(AttemptId);
        assert_recognized!(InstanceId);
        assert_recognized!(TriggerId);
        assert_recognized!(TriggerEventId);
        assert_recognized!(UserId);
        assert_recognized!(ServiceAccountId);
        assert_recognized!(ResourceId);
        assert_recognized!(CredentialId);
        assert_recognized!(SessionId);
    }
}
