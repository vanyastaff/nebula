use nebula_validator::foundation::ValidationError;

fn build_error_tree(depth: usize, width: usize) -> ValidationError {
    if depth == 0 {
        return ValidationError::new("leaf", "leaf");
    }

    let children = (0..width)
        .map(|_| build_error_tree(depth - 1, width))
        .collect::<Vec<_>>();
    ValidationError::new("node", "node").with_nested(children)
}

#[test]
fn nested_error_tree_count_is_deterministic_and_bounded() {
    let tree = build_error_tree(3, 2);
    assert_eq!(tree.total_error_count(), 15);
    assert_eq!(tree.flatten().len(), tree.total_error_count());
}

#[test]
fn nested_error_tree_serialization_is_parseable() {
    let tree = build_error_tree(2, 3);
    let json = tree.to_json_value();
    let nested = json
        .get("nested")
        .and_then(serde_json::Value::as_array)
        .expect("nested must be an array");
    assert_eq!(nested.len(), 3);
}
