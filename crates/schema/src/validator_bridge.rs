//! Internal adapters for `nebula-validator` path and error types.

use nebula_validator::foundation::FieldPath as ValidatorFieldPath;

use crate::{FieldPath, key::FieldKey, path::PathSegment};

/// Resolve a validator rule reference to an absolute schema path.
///
/// Supported forms:
/// - `$root.foo` for legacy root-relative refs
/// - `/foo/bar` JSON Pointer refs emitted by current predicates
pub(crate) fn resolve_rule_dependency(field_ref: &str) -> Option<FieldPath> {
    if let Some(rest) = field_ref.strip_prefix("$root.") {
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

/// Prefer the path carried by a validator error, falling back to the caller's
/// schema path when the validator error has no field pointer.
pub(crate) fn schema_path_from_validator_error(
    fallback: &FieldPath,
    err: &nebula_validator::foundation::ValidationError,
) -> FieldPath {
    err.field_pointer()
        .as_deref()
        .and_then(field_path_from_json_pointer)
        .unwrap_or_else(|| fallback.clone())
}

fn validator_path_to_schema_path(vp: &ValidatorFieldPath) -> Option<FieldPath> {
    let mut out = FieldPath::root();
    let mut any = false;
    for seg in vp.segments() {
        let s = seg.as_ref();
        if s.is_empty() {
            continue;
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

fn field_path_from_json_pointer(pointer: &str) -> Option<FieldPath> {
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
}
