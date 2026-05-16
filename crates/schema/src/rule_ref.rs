//! Pure schema-path/rule-reference parsing for the dependency-graph and
//! secret lints (no validator coupling).
//!
//! These helpers translate the address forms rule references and validator
//! field pointers use (`$root.foo`, `/foo/bar` JSON Pointers, `items[0].name`)
//! into the schema's [`FieldPath`] addressing. They carry no error-mapping or
//! evaluation logic — they are consumed by the build-time dependency-graph and
//! secret lints and by the single validator-error merge in `validated`.

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

/// Parse a validator RFC-6901 field pointer into a schema [`FieldPath`].
///
/// Returns `None` when the pointer is structurally invalid (empty segment,
/// non-`FieldKey` segment); callers fall back to a known path in that case.
pub(crate) fn field_path_from_json_pointer(pointer: &str) -> Option<FieldPath> {
    let pointer = pointer.strip_prefix('#').unwrap_or(pointer);
    if pointer.is_empty() || pointer == "/" {
        return Some(FieldPath::root());
    }
    if !pointer.starts_with('/') {
        return FieldPath::parse(pointer).ok();
    }

    let mut out = FieldPath::root();
    for encoded in pointer.split('/').skip(1) {
        if encoded.is_empty() {
            return None;
        }
        let decoded = decode_json_pointer_segment(encoded);
        let segment = if decoded.chars().all(|c| c.is_ascii_digit()) {
            PathSegment::Index(decoded.parse().ok()?)
        } else {
            PathSegment::Key(FieldKey::new(decoded).ok()?)
        };
        out = out.join(segment);
    }
    Some(out)
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

fn decode_json_pointer_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                Some(other) => {
                    out.push('~');
                    out.push(other);
                },
                None => out.push('~'),
            }
        } else {
            out.push(ch);
        }
    }
    out
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
    fn json_pointer_parser_decodes_segments_and_rejects_empty_segments() {
        let path = field_path_from_json_pointer("/items/0/name").unwrap();
        assert_eq!(path.to_string(), "items[0].name");

        assert_eq!(decode_json_pointer_segment("field~0name"), "field~name");
        assert_eq!(decode_json_pointer_segment("field~1name"), "field/name");
        assert_eq!(decode_json_pointer_segment("field~Xname"), "field~Xname");
        assert_eq!(decode_json_pointer_segment("field~"), "field~");

        assert!(field_path_from_json_pointer("/items//name").is_none());
        assert!(field_path_from_json_pointer("/items/").is_none());
        assert!(field_path_from_json_pointer("/field~0name").is_none());
        assert!(field_path_from_json_pointer("/foo~1bar").is_none());
    }
}
