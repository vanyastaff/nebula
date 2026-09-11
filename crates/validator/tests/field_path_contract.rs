use nebula_validator::foundation::{FieldPath, FieldPathError, ValidationError};
use serde_json::{Value, json};

#[test]
fn root_pointer_has_no_segments_or_parent() {
    let root = FieldPath::parse("").unwrap();
    assert_eq!(root.as_str(), "");
    assert_eq!(root.depth(), 0);
    assert_eq!(root.segments().count(), 0);
    assert_eq!(root.last_segment(), None);
    assert_eq!(root.parent(), None);
}

#[test]
fn segment_construction_preserves_empty_keys() {
    let path = FieldPath::from_segments(["", "0", "", "a/b", "c~d", ""]);
    assert_eq!(path.as_str(), "//0//a~1b/c~0d/");
    assert_eq!(
        path.segments().collect::<Vec<_>>(),
        ["", "0", "", "a/b", "c~d", ""]
    );
    assert_eq!(path.depth(), 6);
}

#[test]
fn single_segment_parent_is_root() {
    for path in [FieldPath::single("name"), FieldPath::single("")] {
        assert_eq!(path.parent().unwrap().as_str(), "");
    }
}

#[test]
fn serde_distinguishes_root_and_empty_key() {
    let root: FieldPath = serde_json::from_value(json!("")).unwrap();
    let empty_key: FieldPath = serde_json::from_value(json!("/")).unwrap();
    assert_ne!(root, empty_key);
    assert_eq!(serde_json::to_value(root).unwrap(), json!(""));
    assert_eq!(serde_json::to_value(empty_key).unwrap(), json!("/"));
}

#[test]
fn serde_rejects_path_aliases() {
    for alias in ["a.b", "a[0]", "#/a", " /a"] {
        assert!(
            serde_json::from_value::<FieldPath>(json!(alias)).is_err(),
            "wire path was reinterpreted: {alias:?}"
        );
    }
}

#[test]
fn serde_rejects_invalid_pointer_escapes() {
    for pointer in ["/bad~", "/bad~2", "/bad~~0"] {
        assert!(
            serde_json::from_value::<FieldPath>(json!(pointer)).is_err(),
            "malformed pointer accepted: {pointer:?}"
        );
    }
}

#[test]
fn pointer_parsing_preserves_literal_spaces_and_zero_keys() {
    let pointer = "/0//a~1b/c~0d/ ";
    let path = FieldPath::parse(pointer).unwrap();
    assert_eq!(path.as_str(), pointer);
    assert_eq!(
        path.segments().collect::<Vec<_>>(),
        ["0", "", "a/b", "c~d", " "]
    );
}

#[test]
fn strict_constructor_and_serde_agree() {
    for pointer in [
        "",
        "/",
        "//",
        "/0",
        "/00",
        "/-",
        "/~01",
        "/a~1b/c~0d",
        "/ ",
        "/a.b/[0]",
        "/\0",
    ] {
        let path = FieldPath::from_pointer(pointer).unwrap();
        assert_eq!(path.as_str(), pointer);
        assert_eq!(
            serde_json::from_value::<FieldPath>(json!(pointer)).unwrap(),
            path
        );
    }
    for invalid in ["a.b", "#/a", " /a", "/x~", "/x~2", "/x~1~"] {
        assert!(FieldPath::from_pointer(invalid).is_err(), "{invalid:?}");
        assert!(
            serde_json::from_value::<FieldPath>(json!(invalid)).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn pointer_errors_classify_syntax() {
    assert_eq!(
        FieldPath::from_pointer("name"),
        Err(FieldPathError::MissingLeadingSlash)
    );
    assert_eq!(
        FieldPath::from_pointer("/~2"),
        Err(FieldPathError::InvalidEscape { byte_offset: 1 })
    );
}

#[test]
fn root_is_composition_identity() {
    let root = FieldPath::root();
    let path = FieldPath::from_segments(["", "0", "~", "/"]);
    assert!(root.is_root());
    assert_eq!(FieldPath::from_segments(Vec::<&str>::new()), root);
    assert_eq!(root.append(&path), path);
    assert_eq!(path.append(&root), path);
    assert_eq!(root.push(""), FieldPath::single(""));
    assert!(!root.push("").is_root());
}

#[test]
fn prefixes_match_complete_segments() {
    for (path, prefix, expected) in [
        ("", "", true),
        ("/name", "", true),
        ("", "/name", false),
        ("/name", "/name", true),
        ("/name/0", "/name", true),
        ("/names", "/name", false),
        ("/", "/", true),
        ("//child", "/", true),
        ("/child", "/", false),
        ("/a~1b/child", "/a~1b", true),
        ("/a~1bc", "/a~1b", false),
        ("/a~1b", "/a", false),
    ] {
        assert_eq!(
            FieldPath::from_pointer(path)
                .unwrap()
                .starts_with(&FieldPath::from_pointer(prefix).unwrap()),
            expected,
            "{path:?} starts with {prefix:?}"
        );
    }
}

#[test]
fn typed_root_diagnostic_is_distinct_from_unspecified_field() {
    let error = ValidationError::new("test", "rejected");
    assert_eq!(error.field, None);
    assert_eq!(error.clone().with_field("").field, None);
    let root_error = error.with_field_path(FieldPath::root());
    assert_eq!(root_error.field.as_deref(), Some(""));
    assert_eq!(root_error.field_pointer().as_deref(), Some(""));
    assert_eq!(root_error.to_json_value()["pointer"], json!(""));
}

proptest::proptest! {
    #[test]
    fn arbitrary_segments_roundtrip_and_locate_json_values(
        characters in proptest::collection::vec(
            proptest::collection::vec(proptest::char::any(), 0..12), 0..8
        )
    ) {
        let segments: Vec<String> = characters.into_iter().map(|chars| chars.into_iter().collect()).collect();
        let path = FieldPath::from_segments(&segments);
        let decoded: Vec<_> = path.segments().map(std::borrow::Cow::into_owned).collect();
        proptest::prop_assert_eq!(decoded.as_slice(), segments.as_slice());
        proptest::prop_assert_eq!(path.depth(), segments.len());
        proptest::prop_assert_eq!(&FieldPath::from_pointer(path.as_str()).unwrap(), &path);
        let wire = serde_json::to_value(&path).unwrap();
        proptest::prop_assert_eq!(&serde_json::from_value::<FieldPath>(wire).unwrap(), &path);

        let leaf = json!(37);
        let document = segments.iter().rev().fold(leaf.clone(), |value, key| {
            Value::Object(serde_json::Map::from_iter([(key.clone(), value)]))
        });
        proptest::prop_assert_eq!(document.pointer(path.as_str()), Some(&leaf));
    }
}
