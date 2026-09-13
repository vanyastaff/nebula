// Baseline benchmarks for nebula-expression
// Run with: cargo bench --bench baseline

use std::{assert_matches, hint::black_box, sync::Arc, thread};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_expression::{
    BuiltinOutput, BuiltinOutputBound, BuiltinOutputBuilder, BuiltinOutputLimit, EvaluationContext,
    EvaluationPolicy, ExpressionEngine, ExpressionError, ExpressionResult, Template, Value,
    eval::BuiltinView,
};

// ================================
// Template Benchmarks
// ================================

fn benchmark_template_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("template/parse");

    // Simple template
    group.bench_function("simple", |b| {
        b.iter(|| Template::new(black_box("Hello {{ $input }}!")));
    });

    // Multiple expressions
    group.bench_function("multiple_expressions", |b| {
        b.iter(|| Template::new(black_box("{{ $a }} + {{ $b }} = {{ $a + $b }}")));
    });

    // Complex template
    group.bench_function("complex", |b| {
        let template = r"
            <html>
                <title>{{ $workflow.name }}</title>
                <body>
                    <h1>{{ $execution.id }}</h1>
                    <p>Result: {{ $input | uppercase() }}</p>
                    <span>{{ $node.data.count * 2 }}</span>
                </body>
            </html>
        ";
        b.iter(|| Template::new(black_box(template)));
    });

    group.finish();
}

fn benchmark_template_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("template/render");

    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::String("World".to_string()));

    let simple = Template::new("Hello {{ $input }}!").unwrap();
    let complex = Template::new(
        r"
        <html>
            <title>{{ $input | uppercase() }}</title>
            <p>Length: {{ length($input) }}</p>
        </html>
    ",
    )
    .unwrap();

    group.bench_function("simple", |b| {
        b.iter(|| simple.render(black_box(&engine), black_box(&context)));
    });

    group.bench_function("complex", |b| {
        b.iter(|| complex.render(black_box(&engine), black_box(&context)));
    });

    group.finish();
}

fn benchmark_template_clone(c: &mut Criterion) {
    let template = Template::new("Hello {{ $input }}!").unwrap();

    c.bench_function("template/clone", |b| b.iter(|| black_box(template.clone())));
}

// ================================
// Engine Benchmarks
// ================================

fn benchmark_evaluate_no_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine/evaluate_no_cache");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let test_cases = vec![
        ("literal", "42"),
        ("arithmetic", "2 + 3 * 4"),
        ("comparison", "10 > 5"),
        ("string_concat", r#""hello" + " " + "world""#),
        ("function_call", "uppercase('hello')"),
        // `nested` (`abs(min(-5, -10)) * 2`) is intentionally absent on
        // this PR. The pre-T1 parser on `main` failed to parse that
        // shape with `"Expected ), found ("`, and the bench closure
        // silently swallowed the `Err` — so the existing CodSpeed
        // baseline for `nested` measures parse-error fast-path time
        // (~13 µs), not real parse + eval. Comparing PR's working eval
        // against that baseline produces a phantom 36% "regression".
        // Removing the case lets CodSpeed treat it as `skipped`
        // (baseline carried forward) on this PR; a follow-up commit
        // after merge will re-add the case so a fresh, real baseline
        // gets recorded against the fixed parser.
        ("conditional", "if true then 1 else 2"),
    ];

    for (name, expr) in test_cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, expr| {
            // `.unwrap()` is intentional: a benchmark that silently swallows
            // an `Err` measures only the error fast-path. Pre-T1, the
            // parser failed on `abs(min(-5, -10))`-shape expressions
            // ("Expected ), found ("), so this benchmark on the previous
            // baseline was timing parse-error production rather than
            // full parse + eval. Forcing a panic on Err keeps that class
            // of false-positive baseline out of future runs.
            b.iter(|| {
                engine
                    .evaluate(black_box(expr), black_box(&context))
                    .unwrap()
            });
        });
    }

    group.finish();
}

#[cfg(feature = "cache")]
fn benchmark_evaluate_with_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine/evaluate_with_cache");

    let engine = ExpressionEngine::with_cache_size(1000);
    let context = EvaluationContext::new();

    let expr = "2 + 3 * 4";

    // Warm up cache
    let _ = engine.evaluate(expr, &context);

    group.bench_function("cache_hit", |b| {
        b.iter(|| engine.evaluate(black_box(expr), black_box(&context)));
    });

    // Cache miss
    group.bench_function("cache_miss", |b| {
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let expr = format!("{} + {}", counter, counter + 1);
            engine.evaluate(black_box(&expr), black_box(&context))
        });
    });

    group.finish();
}

// ================================
// Context Benchmarks
// ================================

fn benchmark_context_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("context/operations");

    // Create context with many variables
    let mut context = EvaluationContext::new();
    for i in 0..100 {
        context.set_execution_var(format!("var_{i}"), Value::Number((i as i64).into()));
    }

    // Clone benchmark
    group.bench_function("clone_100_vars", |b| b.iter(|| black_box(context.clone())));

    // Lookup benchmark
    group.bench_function("lookup", |b| {
        b.iter(|| context.get_execution_var(black_box("var_50")));
    });

    for payload_bytes in [4 * 1024usize, 256 * 1024] {
        let mut shared_context = EvaluationContext::new();
        shared_context.set_node_data("large", Value::String("x".repeat(payload_bytes)));
        group.bench_with_input(
            BenchmarkId::new("resolve_shared_node", payload_bytes),
            &shared_context,
            |bencher, shared_context| {
                bencher.iter(|| {
                    black_box(
                        shared_context
                            .resolve_variable(black_box("node"))
                            .unwrap()
                            .unwrap(),
                    )
                });
            },
        );
    }

    group.finish();
}

fn benchmark_context_population(c: &mut Criterion) {
    let mut group = c.benchmark_group("context/populate_nodes");

    for node_count in [16usize, 64, 256] {
        let entries: Vec<_> = (0..node_count)
            .map(|index| {
                (
                    format!("node_{index}"),
                    Value::Number((index as i64).into()),
                )
            })
            .collect();
        group.throughput(Throughput::Elements(node_count as u64));

        group.bench_with_input(
            BenchmarkId::new("individual_updates", node_count),
            &entries,
            |bencher, entries| {
                bencher.iter_batched(
                    EvaluationContext::new,
                    |mut context| {
                        for (node_key, value) in entries {
                            context.set_node_data(node_key, value.clone());
                        }
                        black_box(context)
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("batch", node_count),
            &entries,
            |bencher, entries| {
                bencher.iter_batched(
                    EvaluationContext::new,
                    |mut context| {
                        context.set_node_data_batch(
                            entries
                                .iter()
                                .map(|(node_key, value)| (node_key, value.clone())),
                        );
                        black_box(context)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

// ================================
// Concurrent Benchmarks
// ================================

#[expect(
    clippy::excessive_nesting,
    reason = "thread::spawn inside criterion bench closure naturally requires this depth"
)]
fn benchmark_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent/access");
    group.throughput(Throughput::Elements(1));

    #[cfg(feature = "cache")]
    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    #[cfg(not(feature = "cache"))]
    let engine = Arc::new(ExpressionEngine::new());
    let expr = "2 + 2";

    // Warm up cache
    {
        let context = EvaluationContext::new();
        let _ = engine.evaluate(expr, &context);
    }

    // Single thread baseline
    group.bench_function("1_thread", |b| {
        let context = EvaluationContext::new();
        b.iter(|| engine.evaluate(black_box(expr), black_box(&context)));
    });

    // Multi-threaded
    for num_threads in [2, 4, 8] {
        group.bench_function(format!("{num_threads}_threads"), |b| {
            b.iter(|| {
                let handles: Vec<_> = (0..num_threads)
                    .map(|_| {
                        let engine = Arc::clone(&engine);
                        thread::spawn(move || {
                            let context = EvaluationContext::new();
                            for _ in 0..10 {
                                let _ = engine.evaluate(expr, &context);
                            }
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.join().unwrap();
                }
            });
        });
    }

    group.finish();
}

fn benchmark_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent/throughput");
    group.throughput(Throughput::Elements(1));

    #[cfg(feature = "cache")]
    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    #[cfg(not(feature = "cache"))]
    let engine = Arc::new(ExpressionEngine::new());
    let context = EvaluationContext::new();

    group.bench_function("ops_per_sec", |b| {
        b.iter(|| engine.evaluate(black_box("2 + 2"), black_box(&context)));
    });

    group.finish();
}

// ================================
// Builtin Functions
// ================================

fn benchmark_builtins(c: &mut Criterion) {
    let mut group = c.benchmark_group("builtins");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let test_cases = vec![
        ("string/uppercase", "uppercase('hello world')"),
        ("string/length", "length('hello world')"),
        ("math/abs", "abs(-42)"),
        ("math/max", "max(1, 2, 3, 4, 5)"),
        ("array/first", "first([1, 2, 3, 4, 5])"),
        ("array/join", "join([1, 2, 3, 4, 5], ', ')"),
        ("array/concat", "concat([1, 2], [3, 4], [5, 6])"),
        ("array/flatten", "flatten([[1, 2], [3, 4], [5, 6]])"),
        ("object/keys", "keys({a: 1, b: 2, c: 3})"),
        ("object/values", "values({a: 1, b: 2, c: 3})"),
        ("conversion/to_string", "to_string(42)"),
    ];

    for (name, expr) in test_cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, expr| {
            b.iter(|| engine.evaluate(black_box(expr), black_box(&context)));
        });
    }

    group.finish();
}

fn oversized_custom_output(
    _args: &[&Value],
    _view: BuiltinView<'_>,
    _context: &EvaluationContext,
    output: BuiltinOutputBuilder,
) -> ExpressionResult<BuiltinOutput> {
    output.repeat_string("0123456789abcdef", 256)
}

fn benchmark_builtin_output_rejection(c: &mut Criterion) {
    let policy = EvaluationPolicy::new()
        .with_max_builtin_output_string_bytes(BuiltinOutputBound::new(64).unwrap());
    let mut engine = ExpressionEngine::new().with_policy(policy);
    engine.register_function("oversized", oversized_custom_output);
    let context = EvaluationContext::new();
    let expression = "oversized()";
    let rejection = engine.evaluate(expression, &context).unwrap_err();
    assert_matches!(
        rejection,
        ExpressionError::BuiltinOutputLimitExceeded {
            dimension: BuiltinOutputLimit::StringBytes,
            limit: 64,
            actual: 4096,
        }
    );

    c.bench_function(
        "builtins/output_limit/reject_before_allocation",
        |bencher| {
            bencher.iter(|| {
                let rejection = engine
                    .evaluate(black_box(expression), black_box(&context))
                    .unwrap_err();
                assert_matches!(
                    rejection,
                    ExpressionError::BuiltinOutputLimitExceeded {
                        dimension: BuiltinOutputLimit::StringBytes,
                        limit: 64,
                        actual: 4096,
                    }
                );
            });
        },
    );
}

// ================================
// Criterion Groups
// ================================

criterion_group!(
    template_benches,
    benchmark_template_parse,
    benchmark_template_render,
    benchmark_template_clone
);

#[cfg(feature = "cache")]
criterion_group!(
    engine_benches,
    benchmark_evaluate_no_cache,
    benchmark_evaluate_with_cache
);

#[cfg(not(feature = "cache"))]
criterion_group!(engine_benches, benchmark_evaluate_no_cache);

criterion_group!(
    context_benches,
    benchmark_context_operations,
    benchmark_context_population
);

criterion_group!(
    concurrent_benches,
    benchmark_concurrent_access,
    benchmark_throughput
);

criterion_group!(
    builtin_benches,
    benchmark_builtins,
    benchmark_builtin_output_rejection
);

criterion_main!(
    template_benches,
    engine_benches,
    context_benches,
    concurrent_benches,
    builtin_benches
);
