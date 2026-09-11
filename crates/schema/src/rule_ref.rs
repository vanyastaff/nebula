//! Pure schema-path/rule-reference parsing for the dependency-graph and
//! secret lints.
//!
//! These helpers translate the address forms rule references and validator
//! field pointers use (`$root.foo`, `/foo/bar` JSON Pointers, `items[0].name`)
//! into the schema's [`FieldPath`] addressing. They carry no error-mapping or
//! evaluation logic. Runtime diagnostics retain their data pointers instead of
//! narrowing arbitrary data keys into schema identifiers.

use nebula_validator::foundation::FieldPath as ValidatorFieldPath;

use crate::{FieldPath, key::FieldKey, path::PathSegment};

/// Resolve a validator rule reference to an absolute schema path.
///
/// Supported forms:
/// - `$root.foo` for legacy root-relative refs
/// - `/foo/bar` JSON Pointer refs emitted by current predicates
pub(crate) fn resolve_rule_dependency(field_ref: &str) -> Option<FieldPath> {
    if let Some(rest) = field_ref.strip_prefix("$root.") {
        if rest.split('.').any(str::is_empty) {
            return None;
        }
        let vp = ValidatorFieldPath::parse(rest)?;
        return validator_path_to_schema_path(&vp);
    }
    if field_ref.starts_with('/') {
        let vp = ValidatorFieldPath::parse(field_ref)?;
        return validator_path_to_schema_path(&vp);
    }
    None
}

/// Return the root key referenced by a validator rule reference.
pub(crate) fn referenced_root_key(field_ref: &str) -> Option<FieldKey> {
    let path = resolve_rule_dependency(field_ref)?;
    match path.segments().first()? {
        PathSegment::Key(key) => Some(key.clone()),
        PathSegment::Index(_) => None,
    }
}

/// Normalize rule paths to schema-path shape.
///
/// Rule refs may point at concrete list instances (`items[0].name`), while
/// schema paths address the item shape (`items.name`).
pub(crate) fn normalize_rule_target_path(path: &FieldPath) -> FieldPath {
    let mut normalized = FieldPath::root();
    for segment in path.segments() {
        if matches!(segment, PathSegment::Index(_)) && !normalized.is_root() {
            continue;
        }
        normalized = normalized.join(segment.clone());
    }
    normalized
}

fn validator_path_to_schema_path(vp: &ValidatorFieldPath) -> Option<FieldPath> {
    let mut out = FieldPath::root();
    let mut any = false;
    for seg in vp.segments() {
        let s = seg.as_ref();
        if s.is_empty() {
            return None;
        }
        any = true;
        let segment = if s.chars().all(|c| c.is_ascii_digit()) {
            PathSegment::Index(s.parse().ok()?)
        } else {
            PathSegment::Key(FieldKey::new(s).ok()?)
        };
        out = out.join(segment);
    }
    if any { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_root_key_requires_first_path_segment_to_be_key() {
        assert_eq!(
            referenced_root_key("/items/0/name")
                .expect("root key")
                .as_str(),
            "items"
        );
        assert!(referenced_root_key("/0/items").is_none());
    }

    #[test]
    fn normalize_rule_target_path_preserves_leading_index() {
        let list_child = resolve_rule_dependency("/items/0/name").unwrap();
        assert_eq!(
            normalize_rule_target_path(&list_child).to_string(),
            "items.name"
        );

        let leading_index = resolve_rule_dependency("/0/items").unwrap();
        assert_eq!(
            normalize_rule_target_path(&leading_index).to_string(),
            "[0].items"
        );
    }

    #[test]
    fn rule_dependency_rejects_empty_path_segments() {
        assert!(resolve_rule_dependency("/items//name").is_none());
        assert!(resolve_rule_dependency("/items/").is_none());
        assert!(resolve_rule_dependency("$root.items..name").is_none());
    }

    #[test]
    fn data_pointers_are_decoded_before_narrowing_to_schema_identifiers() {
        let path = resolve_rule_dependency("/items/0/name").unwrap();
        assert_eq!(path.to_string(), "items[0].name");
        let data = ValidatorFieldPath::from_pointer("/field~0name/field~1name/").unwrap();
        assert_eq!(
            data.segments()
                .map(|segment| segment.to_string())
                .collect::<Vec<_>>(),
            ["field~name", "field/name", ""]
        );
        assert!(validator_path_to_schema_path(&data).is_none());
        for pointer in ["/field~Xname", "/field~"] {
            assert!(ValidatorFieldPath::from_pointer(pointer).is_err());
            assert!(resolve_rule_dependency(pointer).is_none());
        }
        assert!(resolve_rule_dependency("/items//name").is_none());
        assert!(resolve_rule_dependency("/items/").is_none());
        assert!(resolve_rule_dependency("/field~0name").is_none());
        assert!(resolve_rule_dependency("/foo~1bar").is_none());
    }
}
