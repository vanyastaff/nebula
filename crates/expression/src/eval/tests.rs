use super::*;
use crate::{
    builtins::BuiltinRegistry,
    policy::{EvaluationPolicy, EvaluationStepLimit},
};

fn create_evaluator() -> Evaluator {
    let registry = Arc::new(BuiltinRegistry::new());
    Evaluator::new(registry)
}

fn create_evaluator_with_allowlist(functions: &[&str]) -> Evaluator {
    let registry = Arc::new(BuiltinRegistry::new());
    let policy = EvaluationPolicy::allow_only(functions.iter().copied());
    Evaluator::with_policy(registry, Some(Arc::new(policy)))
}

#[test]
fn test_eval_literal() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Literal(Value::Number(42.into()));
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_eval_arithmetic() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Number(10.into()))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Value::Number(5.into()))),
    };
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_i64(), Some(15));
}

#[test]
fn test_deep_nesting_within_limit() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // Create moderately nested expression (safe for both construction and evaluation)
    let mut expr = Expr::Literal(Value::Number(1.into()));
    for _ in 0..50 {
        // 50 levels is safe and tests recursion tracking works
        expr = Expr::Binary {
            left: Box::new(expr),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(1.into()))),
        };
    }

    // Should succeed (50 << 256)
    let result = evaluator.eval(&expr, &context);
    assert!(result.is_ok(), "50-level deep expression should succeed");
    assert_eq!(result.unwrap().as_i64(), Some(51));
}

#[test]
fn test_short_circuit_and_false() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // false && <anything> should short-circuit and not evaluate right side
    // Using a division by zero on the right to prove it's not evaluated
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Bool(false))),
        op: BinaryOp::And,
        right: Box::new(Expr::Binary {
            left: Box::new(Expr::Literal(Value::Number(1.into()))),
            op: BinaryOp::Divide,
            right: Box::new(Expr::Literal(Value::Number(0.into()))),
        }),
    };

    // Should succeed without dividing by zero (short-circuit)
    let result = evaluator.eval(&expr, &context);
    assert!(
        result.is_ok(),
        "Short-circuit should prevent division by zero"
    );
    assert_eq!(result.unwrap().as_bool(), Some(false));
}

#[test]
fn test_short_circuit_or_true() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // true || <anything> should short-circuit and not evaluate right side
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Bool(true))),
        op: BinaryOp::Or,
        right: Box::new(Expr::Binary {
            left: Box::new(Expr::Literal(Value::Number(1.into()))),
            op: BinaryOp::Divide,
            right: Box::new(Expr::Literal(Value::Number(0.into()))),
        }),
    };

    // Should succeed without dividing by zero (short-circuit)
    let result = evaluator.eval(&expr, &context);
    assert!(
        result.is_ok(),
        "Short-circuit should prevent division by zero"
    );
    assert_eq!(result.unwrap().as_bool(), Some(true));
}

#[test]
fn test_and_evaluates_both_when_left_true() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // true && false should evaluate both
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Bool(true))),
        op: BinaryOp::And,
        right: Box::new(Expr::Literal(Value::Bool(false))),
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_or_evaluates_both_when_left_false() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // false || true should evaluate both
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Bool(false))),
        op: BinaryOp::Or,
        right: Box::new(Expr::Literal(Value::Bool(true))),
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
#[cfg(feature = "regex")]
fn test_regex_caching() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // First regex match - should compile and cache
    let expr1 = Expr::Binary {
        left: Box::new(Expr::Literal(Value::String("hello world".to_string()))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::String("hello.*".to_string()))),
    };
    let result1 = evaluator.eval(&expr1, &context).unwrap();
    assert_eq!(result1.as_bool(), Some(true));

    // Second regex match with same pattern - should use cached regex
    let expr2 = Expr::Binary {
        left: Box::new(Expr::Literal(Value::String("hello universe".to_string()))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::String("hello.*".to_string()))),
    };
    let result2 = evaluator.eval(&expr2, &context).unwrap();
    assert_eq!(result2.as_bool(), Some(true));

    // Verify cache has the pattern
    evaluator.regex_cache.run_pending_tasks();
    assert_eq!(evaluator.regex_cache.entry_count(), 1);
    let key: Arc<str> = Arc::from("hello.*");
    assert!(evaluator.regex_cache.get(&key).is_some());
}

#[test]
#[cfg(feature = "regex")]
fn test_regex_no_match() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::String("goodbye world".to_string()))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::String("^hello".to_string()))),
    };
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ReDoS protection tests

#[test]
#[cfg(feature = "regex")]
fn test_redos_pattern_length_limit() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // Create a pattern that exceeds the maximum length
    let long_pattern = "a".repeat(MAX_REGEX_PATTERN_LEN + 1);

    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::String("test".to_string()))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::String(long_pattern))),
    };

    let result = evaluator.eval(&expr, &context);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("too long"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_nested_quantifiers_plus_plus() {
    // Test pattern like (a+)+ which can cause catastrophic backtracking
    assert!(Evaluator::is_potentially_dangerous_regex("(a+)+"));
    assert!(Evaluator::is_potentially_dangerous_regex("(a+)+b"));
    assert!(Evaluator::is_potentially_dangerous_regex("^(a+)+$"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_nested_quantifiers_star_star() {
    // Test pattern like (a*)* which can cause catastrophic backtracking
    assert!(Evaluator::is_potentially_dangerous_regex("(a*)*"));
    assert!(Evaluator::is_potentially_dangerous_regex("(.*)*"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_nested_quantifiers_mixed() {
    // Test mixed quantifier patterns
    assert!(Evaluator::is_potentially_dangerous_regex("(a+)*"));
    assert!(Evaluator::is_potentially_dangerous_regex("(a*)+"));
    assert!(Evaluator::is_potentially_dangerous_regex("([a-z]+)*"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_nested_quantifiers_with_braces() {
    // Test patterns with curly brace quantifiers
    assert!(Evaluator::is_potentially_dangerous_regex("(a{2,})+"));
    assert!(Evaluator::is_potentially_dangerous_regex("(a{1,5})*"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_safe_patterns() {
    // These patterns should NOT be flagged as dangerous
    assert!(!Evaluator::is_potentially_dangerous_regex("hello.*"));
    assert!(!Evaluator::is_potentially_dangerous_regex("^[a-z]+$"));
    assert!(!Evaluator::is_potentially_dangerous_regex("\\d{3}-\\d{4}"));
    assert!(!Evaluator::is_potentially_dangerous_regex("(abc)+"));
    assert!(!Evaluator::is_potentially_dangerous_regex("a+b+c+"));
    assert!(!Evaluator::is_potentially_dangerous_regex("(foo|bar)+"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_rejection_in_eval() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // This dangerous pattern should be rejected
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::String(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaa!".to_string(),
        ))),
        op: BinaryOp::RegexMatch,
        right: Box::new(Expr::Literal(Value::String("(a+)+$".to_string()))),
    };

    let result = evaluator.eval(&expr, &context);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("nested quantifiers"));
}

#[test]
#[cfg(feature = "regex")]
fn test_regex_cache_size_limit() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // Fill the cache with many patterns
    for i in 0..MAX_REGEX_CACHE_SIZE + 10 {
        let pattern = format!("pattern_{i}");
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("test".to_string()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String(pattern))),
        };
        let _ = evaluator.eval(&expr, &context);
    }

    // Cache should not exceed MAX_REGEX_CACHE_SIZE.
    // moka admits eagerly but evicts asynchronously — running pending
    // tasks forces eviction-state to converge so the size we read is
    // the post-LRU steady state.
    evaluator.regex_cache.run_pending_tasks();
    let cache_size = evaluator.regex_cache.entry_count();
    assert!(
        cache_size <= MAX_REGEX_CACHE_SIZE as u64,
        "Cache size {cache_size} exceeds limit {MAX_REGEX_CACHE_SIZE}"
    );
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_escaped_characters() {
    // Escaped parentheses and quantifiers should not trigger false positives
    assert!(!Evaluator::is_potentially_dangerous_regex(r"\(a+\)+"));
    assert!(!Evaluator::is_potentially_dangerous_regex(r"\+\*"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_nested_parens_inside_group_are_scanned() {
    // The matching-parenthesis scan must track depth, so a group whose
    // content is itself a parenthesized group is inspected as a whole:
    // "(a+)" inside the outer group carries a quantifier, "(abc)" does not.
    assert!(Evaluator::is_potentially_dangerous_regex("((a+))+"));
    assert!(!Evaluator::is_potentially_dangerous_regex("((abc))"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_quantified_nested_group_is_dangerous() {
    // Quantifier directly on a nested group whose inner group is quantified:
    // catastrophic backtracking shape ((a+))+
    assert!(Evaluator::is_potentially_dangerous_regex("((a+))+"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_escaped_quantifier_inside_group_is_true_by_design() {
    // The heuristic is deliberately char-based: it inspects the raw
    // characters of the group content without unescaping them, so the
    // escaped '+' inside (a\+)+ is counted as a real quantifier and the
    // pattern is flagged. A known false positive, pinned as-is.
    assert!(Evaluator::is_potentially_dangerous_regex(r"(a\+)+"));
}

#[test]
#[cfg(feature = "regex")]
fn test_redos_unbalanced_groups_are_not_dangerous() {
    // A group that never closes makes `find_group_end` return `None`,
    // and the detector's `None => break` arm must stop the scan there —
    // unbalanced groups are invalid regex shapes the engine will refuse
    // later, not ReDoS shapes, so none of these is flagged dangerous.
    assert!(!Evaluator::is_potentially_dangerous_regex("((("));
    assert!(!Evaluator::is_potentially_dangerous_regex("(("));
    assert!(!Evaluator::is_potentially_dangerous_regex("(a"));
    assert!(!Evaluator::is_potentially_dangerous_regex("(a+"));
}

#[test]
#[cfg(feature = "regex")]
fn regex_cache_keeps_hot_pattern_under_load() {
    // ROADMAP #590: under the previous `keys().next()` eviction the hot
    // pattern could be thrown out because HashMap iteration order is
    // undefined. moka's true-LRU eviction must keep a frequently-touched
    // pattern alive even when N+1 colder patterns are added.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let hot = "hello.*".to_string();

    // Prime the cache with the hot pattern, then keep it warm.
    let warm = |pat: &str| {
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::String("hello world".into()))),
            op: BinaryOp::RegexMatch,
            right: Box::new(Expr::Literal(Value::String(pat.into()))),
        };
        let _ = evaluator.eval(&expr, &context);
    };
    warm(&hot);

    // Push more cold patterns than the cache holds, but keep touching
    // the hot one in between so its recency stays fresh.
    for i in 0..MAX_REGEX_CACHE_SIZE + 20 {
        let cold = format!("cold_{i}");
        warm(&cold);
        warm(&hot); // refresh recency
    }

    evaluator.regex_cache.run_pending_tasks();
    let hot_key: Arc<str> = Arc::from(hot.as_str());
    assert!(
        evaluator.regex_cache.get(&hot_key).is_some(),
        "hot pattern was evicted under load — LRU regression"
    );
}

// Higher-order function tests

#[test]
fn test_filter_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // filter([1, 2, 3, 4, 5], x => x > 2) should return [3, 4, 5]
    let expr = Expr::FunctionCall {
        name: Arc::from("filter"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
                Expr::Literal(Value::Number(4.into())),
                Expr::Literal(Value::Number(5.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("x"))),
                    op: BinaryOp::GreaterThan,
                    right: Box::new(Expr::Literal(Value::Number(2.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.first().unwrap().as_i64(), Some(3));
    assert_eq!(arr[1].as_i64(), Some(4));
    assert_eq!(arr[2].as_i64(), Some(5));
}

#[test]
fn test_map_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // map([1, 2, 3], x => x * 2) should return [2, 4, 6]
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("x"))),
                    op: BinaryOp::Multiply,
                    right: Box::new(Expr::Literal(Value::Number(2.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.first().unwrap().as_i64(), Some(2));
    assert_eq!(arr[1].as_i64(), Some(4));
    assert_eq!(arr[2].as_i64(), Some(6));
}

#[test]
fn test_reduce_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // reduce([1, 2, 3], 0, x => $acc + x) should return 6
    let expr = Expr::FunctionCall {
        name: Arc::from("reduce"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
            ]),
            Expr::Literal(Value::Number(0.into())),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("$acc"))),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Variable(Arc::from("x"))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_i64(), Some(6));
}

#[test]
fn test_find_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // find([1, 2, 3, 4], x => x > 2) should return 3
    let expr = Expr::FunctionCall {
        name: Arc::from("find"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
                Expr::Literal(Value::Number(4.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("x"))),
                    op: BinaryOp::GreaterThan,
                    right: Box::new(Expr::Literal(Value::Number(2.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_every_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // every([2, 4, 6], x => x % 2 == 0) should return true
    let expr = Expr::FunctionCall {
        name: Arc::from("every"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(4.into())),
                Expr::Literal(Value::Number(6.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::Modulo,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                    op: BinaryOp::Equal,
                    right: Box::new(Expr::Literal(Value::Number(0.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_some_with_lambda() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // some([1, 2, 3], x => x > 2) should return true
    let expr = Expr::FunctionCall {
        name: Arc::from("some"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("x"))),
                    op: BinaryOp::GreaterThan,
                    right: Box::new(Expr::Literal(Value::Number(2.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(true));

    // some([1, 2, 3], x => x > 5) should return false
    let expr2 = Expr::FunctionCall {
        name: Arc::from("some"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(1.into())),
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(3.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("x"))),
                    op: BinaryOp::GreaterThan,
                    right: Box::new(Expr::Literal(Value::Number(5.into()))),
                }),
            },
        ],
    };

    let result2 = evaluator.eval(&expr2, &context).unwrap();
    assert_eq!(result2.as_bool(), Some(false));
}

#[test]
fn test_allowlist_alias_for_higher_order_function() {
    let evaluator = create_evaluator_with_allowlist(&["all"]);
    let context = EvaluationContext::new();

    let expr = Expr::FunctionCall {
        name: Arc::from("every"),
        args: vec![
            Expr::Array(vec![
                Expr::Literal(Value::Number(2.into())),
                Expr::Literal(Value::Number(4.into())),
                Expr::Literal(Value::Number(6.into())),
            ]),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Binary {
                        left: Box::new(Expr::Variable(Arc::from("x"))),
                        op: BinaryOp::Modulo,
                        right: Box::new(Expr::Literal(Value::Number(2.into()))),
                    }),
                    op: BinaryOp::Equal,
                    right: Box::new(Expr::Literal(Value::Number(0.into()))),
                }),
            },
        ],
    };

    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ────────────────────────────────────────────────────────────────
// CO-C1-01 / issue #252 regression guards — step-budget enforcement
// across higher-order combinators, with thread-safety under a
// shared Arc<Evaluator>.
// ────────────────────────────────────────────────────────────────

/// Build an `Evaluator` with a hard step budget.
fn create_evaluator_with_step_budget(max_steps: usize) -> Evaluator {
    let registry = Arc::new(BuiltinRegistry::new());
    let limit = EvaluationStepLimit::new(max_steps).unwrap();
    let policy = EvaluationPolicy::new().with_max_eval_steps(limit);
    Evaluator::with_policy(registry, Some(Arc::new(policy)))
}

/// Build a literal array `[0, 1, ..., n-1]`.
fn literal_array(n: usize) -> Expr {
    Expr::Array(
        (0..n)
            .map(|i| Expr::Literal(Value::Number((i as i64).into())))
            .collect(),
    )
}

/// `x => x + 1` — one lambda body evaluation = ~3 steps (binary op +
/// two operands). Used as a cheap predicate that nonetheless multiplies
/// out under higher-order traversal.
fn increment_lambda() -> Expr {
    Expr::Lambda {
        param: Arc::from("x"),
        body: Box::new(Expr::Binary {
            left: Box::new(Expr::Variable(Arc::from("x"))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(1.into()))),
        }),
    }
}

#[test]
fn step_budget_bounds_linear_expression() {
    // Sanity check: a trivial top-level expression still fires the
    // step-limit error when the cap is tight.
    let evaluator = create_evaluator_with_step_budget(2);
    let context = EvaluationContext::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Binary {
            left: Box::new(Expr::Literal(Value::Number(1.into()))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Value::Number(2.into()))),
        }),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Value::Number(3.into()))),
    };
    let err = evaluator.eval(&expr, &context).unwrap_err();
    assert!(
        err.to_string().contains("Step budget exhausted"),
        "unexpected error: {err}"
    );
}

#[test]
fn step_budget_bounds_map_over_large_array() {
    // Pre-fix: `eval_lambda` called `self.eval(...)` which reset the
    // step counter per element, so this test passed. After the fix
    // the lambda reuses the caller's frame and the cap stops the
    // traversal after a handful of elements.
    let evaluator = create_evaluator_with_step_budget(50);
    let context = EvaluationContext::new();
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![literal_array(1000), increment_lambda()],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("map over 1000 elements must exceed a 50-step budget");
    assert!(
        err.to_string().contains("Step budget exhausted"),
        "unexpected error: {err}"
    );
}

#[test]
fn step_budget_bounds_nested_higher_order() {
    // `map(arr, x => filter(arr2, y => y > x))` — nested lambdas
    // used to double-reset the counter. Now the budget is honoured
    // across the whole traversal.
    let evaluator = create_evaluator_with_step_budget(80);
    let context = EvaluationContext::new();
    let inner_filter = Expr::FunctionCall {
        name: Arc::from("filter"),
        args: vec![
            literal_array(20),
            Expr::Lambda {
                param: Arc::from("y"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("y"))),
                    op: BinaryOp::GreaterThan,
                    right: Box::new(Expr::Variable(Arc::from("x"))),
                }),
            },
        ],
    };
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![
            literal_array(20),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(inner_filter),
            },
        ],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("nested map/filter must exceed an 80-step budget");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_bounds_reduce_across_iterations() {
    // `reduce` clones the context per iteration — before the fix
    // the clone carried a reset step counter. Afterwards, each
    // iteration reuses the caller's frame.
    let evaluator = create_evaluator_with_step_budget(30);
    let context = EvaluationContext::new();
    let expr = Expr::FunctionCall {
        name: Arc::from("reduce"),
        args: vec![
            literal_array(100),
            Expr::Literal(Value::Number(0.into())),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("$acc"))),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Variable(Arc::from("x"))),
                }),
            },
        ],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("reduce over 100 elements must exceed a 30-step budget");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_bounds_flat_map() {
    let evaluator = create_evaluator_with_step_budget(40);
    let context = EvaluationContext::new();
    let expr = Expr::FunctionCall {
        name: Arc::from("flat_map"),
        args: vec![literal_array(200), increment_lambda()],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("flat_map over 200 elements must exceed a 40-step budget");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_bounds_group_by() {
    let evaluator = create_evaluator_with_step_budget(40);
    let context = EvaluationContext::new();
    let expr = Expr::FunctionCall {
        name: Arc::from("group_by"),
        args: vec![literal_array(200), increment_lambda()],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("group_by over 200 elements must exceed a 40-step budget");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_resets_between_successive_eval_calls() {
    // Guard against a future mistake that would move the step
    // counter onto `EvaluationContext` (or `Evaluator`) and have
    // it leak across top-level calls. Two back-to-back `eval`
    // calls on the same evaluator and context must each start
    // from a fresh budget.
    let evaluator = create_evaluator_with_step_budget(10);
    let context = EvaluationContext::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Literal(Value::Number(1.into()))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Value::Number(2.into()))),
    };
    // First call: 3 steps, well under the cap.
    let r1 = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(r1.as_i64(), Some(3));
    // Second call: also starts at 0 steps, must also succeed.
    let r2 = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(r2.as_i64(), Some(3));
}

#[test]
fn step_budget_respects_context_policy_when_evaluator_has_none() {
    // An evaluator with no policy can still be bounded via the
    // `EvaluationContext` builder's policy override.
    let evaluator = create_evaluator();
    let limit = EvaluationStepLimit::new(5).unwrap();
    let policy = EvaluationPolicy::new().with_max_eval_steps(limit);
    let context = EvaluationContext::builder().policy(policy).build();
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![literal_array(100), increment_lambda()],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("context-level budget of 5 must also bound a map over 100 elements");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_error_path_does_not_leak_depth_into_next_call() {
    // A recursion-depth error on one `eval` call must not
    // contaminate the next call's depth tracking — each top-level
    // call builds a fresh `EvalFrame` on the caller's stack.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();

    // Build a deeply nested unary-not chain that exceeds MAX_AST_DEPTH.
    let mut deep_expr = Expr::Literal(Value::Bool(true));
    for _ in 0..(MAX_AST_DEPTH + 10) {
        deep_expr = Expr::Not(Box::new(deep_expr));
    }
    let err = evaluator.eval(&deep_expr, &context).unwrap_err();
    assert!(err.to_string().contains("Recursion depth exhausted"));

    // Next call on the same evaluator must start fresh.
    let ok = evaluator
        .eval(&Expr::Literal(Value::Bool(true)), &context)
        .expect("fresh call after a depth error must succeed");
    assert_eq!(ok.as_bool(), Some(true));
}

#[test]
fn step_budget_concurrent_arc_evaluator_is_independent_per_task() {
    // Thread-safety regression: N threads sharing a single
    // `Arc<Evaluator>` must each get their own stack-local
    // `EvalFrame`. Before the fix, all threads shared a single
    // `AtomicUsize` counter on the evaluator and could see
    // spurious "step limit exceeded" errors caused by another
    // thread's work.
    use std::thread;

    let evaluator = Arc::new(create_evaluator_with_step_budget(500));
    let expr = Arc::new(Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![literal_array(50), increment_lambda()],
    });

    let mut handles = Vec::new();
    for _ in 0..8 {
        let evaluator = Arc::clone(&evaluator);
        let expr = Arc::clone(&expr);
        handles.push(thread::spawn(move || {
            let context = EvaluationContext::new();
            // 50 elements × ~3-step body + overhead is well under
            // 500. Every thread should succeed. If the counters
            // were shared, some threads would see 500 exceeded.
            for _ in 0..10 {
                evaluator
                    .eval(&expr, &context)
                    .expect("per-thread budget must be independent");
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

#[test]
fn step_budget_bounds_reduce_nested_in_map() {
    // Reduce has its own per-iteration context clone path (the
    // `$acc` lambda var lives on a fresh clone per element). The
    // other nested test exercises map + filter; this one exercises
    // map-of-reduce so the reduce-specific clone cannot become a
    // hidden counter reset in future refactors.
    let evaluator = create_evaluator_with_step_budget(80);
    let context = EvaluationContext::new();
    let inner_reduce = Expr::FunctionCall {
        name: Arc::from("reduce"),
        args: vec![
            literal_array(10),
            Expr::Literal(Value::Number(0.into())),
            Expr::Lambda {
                param: Arc::from("y"),
                body: Box::new(Expr::Binary {
                    left: Box::new(Expr::Variable(Arc::from("$acc"))),
                    op: BinaryOp::Add,
                    right: Box::new(Expr::Variable(Arc::from("y"))),
                }),
            },
        ],
    };
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![
            literal_array(10),
            Expr::Lambda {
                param: Arc::from("x"),
                body: Box::new(inner_reduce),
            },
        ],
    };
    let err = evaluator
        .eval(&expr, &context)
        .expect_err("map-of-reduce over 10x10 must exceed an 80-step budget");
    assert!(err.to_string().contains("Step budget exhausted"));
}

#[test]
fn step_budget_permissive_budget_still_completes_large_map() {
    // Smoke test that a reasonable budget does NOT spuriously
    // reject a realistic higher-order expression — guards against
    // off-by-one or arithmetic regressions in `tick`.
    let evaluator = create_evaluator_with_step_budget(10_000);
    let context = EvaluationContext::new();
    let expr = Expr::FunctionCall {
        name: Arc::from("map"),
        args: vec![literal_array(100), increment_lambda()],
    };
    let result = evaluator.eval(&expr, &context).unwrap();
    let arr = result.as_array().expect("map returns an array");
    assert_eq!(arr.len(), 100);
    assert_eq!(arr.first().and_then(Value::as_i64), Some(1));
    assert_eq!(arr.last().and_then(Value::as_i64), Some(100));
}

#[test]
fn test_negate_integer() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(42.into()))));
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_i64(), Some(-42));
}

#[test]
fn test_negate_float_preserves_fraction() {
    // Regression for #280: `-3.7` must NOT truncate to `-3`.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(serde_json::json!(3.7))));
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_f64(), Some(-3.7));
}

#[test]
fn test_negate_negative_float() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(serde_json::json!(-2.5))));
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_f64(), Some(2.5));
}

#[test]
fn test_negate_i64_min_uses_representable_json_unsigned_integer() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(i64::MIN.into()))));
    let value = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(value, serde_json::json!(9_223_372_036_854_775_808_u64));
}

#[test]
fn test_negate_first_u64_above_i64_max_uses_i64_min() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let big = (i64::MAX as u64) + 1;
    let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(big.into()))));
    let value = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(value, serde_json::json!(i64::MIN));
}

#[test]
fn test_negate_u64_max_errors() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(Value::Number(u64::MAX.into()))));
    let err = evaluator.eval(&expr, &context).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("overflow"),
        "expected overflow error, got: {msg}"
    );
}

#[test]
fn test_negate_non_number_type_error() {
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = Expr::Negate(Box::new(Expr::Literal(Value::Bool(true))));
    let err = evaluator.eval(&expr, &context).unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("type"));
}

/// Helper: build a `2 ** exp` expression.
fn power_expr(base: i64, exp: f64) -> Expr {
    Expr::Binary {
        left: Box::new(Expr::Literal(Value::Number(base.into()))),
        op: BinaryOp::Power,
        right: Box::new(Expr::Literal(serde_json::json!(exp))),
    }
}

#[test]
fn power_overflow_returns_error() {
    // 2 ** 1024 overflows f64 to +Infinity. Without the is_finite guard
    // this would silently serialize as JSON null.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = power_expr(2, 1024.0);
    let err = evaluator.eval(&expr, &context).unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("non-finite"),
        "expected non-finite error, got: {msg}"
    );
}

#[test]
fn power_negative_fractional_returns_error() {
    // (-1) ** 0.5 is NaN in floating-point. Without the guard it would
    // serialize as JSON null.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = power_expr(-1, 0.5);
    let err = evaluator.eval(&expr, &context).unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("non-finite"),
        "expected non-finite error, got: {msg}"
    );
}

#[test]
fn power_normal_case_still_works() {
    // Regression guard: ordinary power must keep working.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = power_expr(2, 10.0);
    let result = evaluator.eval(&expr, &context).unwrap();
    // f64-typed literal stays float on the wire
    assert_eq!(result.as_f64(), Some(1024.0));
}

#[test]
fn power_zero_to_zero_returns_one() {
    // 0 ** 0 is mathematically undefined but f64::powf returns 1.0.
    // is_finite passes, so we surface the float result rather than erroring.
    let evaluator = create_evaluator();
    let context = EvaluationContext::new();
    let expr = power_expr(0, 0.0);
    let result = evaluator.eval(&expr, &context).unwrap();
    assert_eq!(result.as_f64(), Some(1.0));
}
