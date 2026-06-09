//! Per-instance credential display metadata.
//!
//! [`CredentialDisplay`] is the human-facing, **non-secret** metadata a user
//! attaches to a credential *instance* (a created credential like
//! "Prod GitHub PAT") — distinct from the type-level
//! [`CredentialMetadata`](crate::CredentialMetadata), which describes the
//! credential *kind*.
//!
//! It is carried on [`CredentialSnapshot`](crate::CredentialSnapshot) and
//! persisted by the credential runtime under a reserved `metadata["display"]`
//! sub-object on the stored credential (a single-writer key, sibling to the
//! tenancy `owner_id` key). It never holds secret material, so it derives
//! `Debug`/`Serialize` without redaction.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Human-facing, non-secret display metadata for a credential instance.
///
/// All fields are optional: a system-acquired credential may carry none. The
/// type is the API/management-tier shape (a created credential's name,
/// description, and organizational tags), kept out of the secret/lifecycle
/// state. `BTreeMap` keeps tag ordering deterministic for stable wire output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialDisplay {
    /// Human-readable instance name (e.g. `"Prod GitHub PAT"`). `None` when the
    /// creator supplied none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Optional free-text description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// User-defined tags for organization and filtering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
}

impl CredentialDisplay {
    /// True when no display field is set (the empty default).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none() && self.description.is_none() && self.tags.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty() {
        assert!(CredentialDisplay::default().is_empty());
    }

    #[test]
    fn populated_is_not_empty() {
        let d = CredentialDisplay {
            display_name: Some("Prod key".to_owned()),
            ..Default::default()
        };
        assert!(!d.is_empty());
    }

    #[test]
    fn empty_fields_are_skipped_in_serialization() {
        let json = serde_json::to_value(CredentialDisplay::default()).expect("serialize");
        // No `display_name`/`description`/`tags` keys when empty.
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn round_trips_through_json() {
        let mut tags = BTreeMap::new();
        tags.insert("env".to_owned(), "prod".to_owned());
        let d = CredentialDisplay {
            display_name: Some("n".to_owned()),
            description: Some("d".to_owned()),
            tags,
        };
        let back: CredentialDisplay =
            serde_json::from_value(serde_json::to_value(&d).expect("ser")).expect("de");
        assert_eq!(d, back);
    }
}
