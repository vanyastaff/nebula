use nebula_validator::foundation::{MAX_ERROR_TREE_DEPTH, ValidationError};

fn build_error_tree(depth: usize, width: usize) -> ValidationError {
    if depth == 0 {
        return ValidationError::new("leaf", "leaf");
    }

    let children = (0..width)
        .map(|_| build_error_tree(depth - 1, width))
        .collect::<Vec<_>>();
    ValidationError::new("node", "node").with_nested(children)
}

/// A linear chain of `depth` nested errors.
fn chain(depth: usize) -> ValidationError {
    let mut error = ValidationError::new("leaf", "leaf");
    for _ in 0..depth {
        error = ValidationError::new("branch", "branch").with_nested_error(error);
    }
    error
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

/// A tree cannot be built deeper than the documented ceiling, no matter how
/// many times the caller wraps it.
#[test]
fn construction_caps_the_tree_depth() {
    let deep = chain(MAX_ERROR_TREE_DEPTH * 100);
    assert!(
        deep.max_depth() <= MAX_ERROR_TREE_DEPTH,
        "constructed depth {} exceeds the {} ceiling",
        deep.max_depth(),
        MAX_ERROR_TREE_DEPTH
    );
}

/// Every traversal completes on a pathological chain instead of overflowing
/// the stack. Before the construction-time cap each of these aborted the
/// process — including `Drop`.
#[test]
fn traversals_complete_on_an_over_deep_chain() {
    let deep = chain(50_000);
    let mut display = String::new();
    let _ = std::fmt::Write::write_fmt(&mut display, format_args!("{deep}"));

    let count = deep.total_error_count();
    let flat = deep.flatten().len();
    let json = deep.to_json_value();
    let cloned = deep.clone();
    let _ = cloned == deep;
    let _ = format!("{deep:?}");

    assert_eq!(count, flat, "flatten and total_error_count must agree");
    assert!(count <= MAX_ERROR_TREE_DEPTH);
    assert!(json.get("nested").is_some());
}

/// Truncation is observable: the node where the cut happened records how many
/// diagnostics were dropped, so the caller is not silently misled.
#[test]
fn truncation_reports_omitted_diagnostics() {
    let deep = chain(MAX_ERROR_TREE_DEPTH + 10);
    let omitted_total: usize = deep
        .flatten()
        .iter()
        .filter_map(|error| error.param("nested_errors_omitted"))
        .filter_map(|value| value.parse::<usize>().ok())
        .sum();
    assert!(
        omitted_total > 0,
        "truncation must record the omitted diagnostic count"
    );
}
