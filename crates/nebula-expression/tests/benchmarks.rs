// Simple baseline benchmarks without external dependencies
// Run with: cargo test --release --test simple_baseline -- --nocapture --test-threads=1

use nebula_expression::{Template, ExpressionEngine, EvaluationContext};
use nebula_value::Value;
use std::time::{Duration, Instant};

const ITERATIONS: usize = 1000;
const WARMUP_ITERATIONS: usize = 100;

// Simple benchmark helper
fn benchmark<F>(name: &str, mut f: F) -> Duration
where
    F: FnMut(),
{
    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        f();
    }

    // Actual measurement
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        f();
    }
    let duration = start.elapsed();

    let avg = duration / ITERATIONS as u32;
    println!("{:45} {:>12.2?}", name, avg);

    duration
}

#[test]
#[ignore]
fn run_all_benchmarks() {
    println!("\n{:=^60}", " BASELINE BENCHMARKS ");
    println!("{:45} {:>12}", "Benchmark", "Avg Time");
    println!("{:-^60}", "");

    template_benchmarks();
    engine_benchmarks();
    context_benchmarks();
    concurrent_benchmarks();
    builtin_benchmarks();

    println!("{:=^60}\n", "");
}

fn template_benchmarks() {
    println!("\n{} TEMPLATE BENCHMARKS", "📝");

    // Parse simple
    benchmark("template/parse/simple", || {
        let _ = Template::new("Hello {{ $input }}!");
    });

    // Parse multiple expressions
    benchmark("template/parse/multiple", || {
        let _ = Template::new("{{ $a }} + {{ $b }} = {{ $a + $b }}");
    });

    // Parse complex
    let complex_template = r#"
        <html>
            <title>{{ $workflow.name }}</title>
            <body>
                <h1>{{ $execution.id }}</h1>
                <p>Result: {{ $input | uppercase() }}</p>
            </body>
        </html>
    "#;
    benchmark("template/parse/complex", || {
        let _ = Template::new(complex_template);
    });

    // Render simple
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::text("World"));
    let simple = Template::new("Hello {{ $input }}!").unwrap();

    benchmark("template/render/simple", || {
        let _ = simple.render(&engine, &context);
    });

    // Clone
    let template = Template::new("Hello {{ $input }}!").unwrap();
    benchmark("template/clone", || {
        let _ = template.clone();
    });
}

fn engine_benchmarks() {
    println!("\n{} ENGINE BENCHMARKS", "⚙️");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    // Different expression types (no cache)
    benchmark("engine/eval/literal", || {
        let _ = engine.evaluate("42", &context);
    });

    benchmark("engine/eval/arithmetic", || {
        let _ = engine.evaluate("2 + 3 * 4", &context);
    });

    benchmark("engine/eval/comparison", || {
        let _ = engine.evaluate("10 > 5", &context);
    });

    benchmark("engine/eval/function_call", || {
        let _ = engine.evaluate("uppercase('hello')", &context);
    });

    benchmark("engine/eval/nested", || {
        let _ = engine.evaluate("abs(min(-5, -10)) * 2", &context);
    });

    benchmark("engine/eval/conditional", || {
        let _ = engine.evaluate("if true then 1 else 2", &context);
    });

    // With cache
    let cached_engine = ExpressionEngine::with_cache_size(1000);
    let expr = "2 + 3 * 4";

    // Warm up cache
    let _ = cached_engine.evaluate(expr, &context);

    benchmark("engine/eval_cached/hit", || {
        let _ = cached_engine.evaluate(expr, &context);
    });
}

fn context_benchmarks() {
    println!("\n{} CONTEXT BENCHMARKS", "📦");

    // Create context with many variables
    let mut context = EvaluationContext::new();
    for i in 0..100 {
        context.set_execution_var(format!("var_{}", i), Value::integer(i as i64));
    }

    benchmark("context/clone_100_vars", || {
        let _ = context.clone();
    });

    benchmark("context/lookup", || {
        let _ = context.get_execution_var("var_50");
    });
}

fn concurrent_benchmarks() {
    println!("\n{} CONCURRENT BENCHMARKS", "🔀");

    use std::sync::Arc;
    use std::thread;

    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    // Warm up
    {
        let ctx = EvaluationContext::new();
        let _ = engine.evaluate(expr, &ctx);
    }

    // Single thread baseline
    let ctx = EvaluationContext::new();
    benchmark("concurrent/1_thread", || {
        let _ = engine.evaluate(expr, &ctx);
    });

    // 2 threads
    let duration = benchmark("concurrent/2_threads", || {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let engine = Arc::clone(&engine);
                thread::spawn(move || {
                    let ctx = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = engine.evaluate(expr, &ctx);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    });

    // 8 threads
    let duration = benchmark("concurrent/8_threads", || {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let engine = Arc::clone(&engine);
                thread::spawn(move || {
                    let ctx = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = engine.evaluate(expr, &ctx);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    });

    // Throughput estimate
    let single_duration = {
        let ctx = EvaluationContext::new();
        let start = Instant::now();
        for _ in 0..10000 {
            let _ = engine.evaluate(expr, &ctx);
        }
        start.elapsed()
    };

    let ops_per_sec = (10000.0 / single_duration.as_secs_f64()) as u64;
    println!("{:45} {:>12} ops/sec", "concurrent/throughput", ops_per_sec);
}

fn builtin_benchmarks() {
    println!("\n{} BUILTIN BENCHMARKS", "🔧");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    benchmark("builtin/string/uppercase", || {
        let _ = engine.evaluate("uppercase('hello world')", &context);
    });

    benchmark("builtin/string/length", || {
        let _ = engine.evaluate("length('hello world')", &context);
    });

    benchmark("builtin/math/abs", || {
        let _ = engine.evaluate("abs(-42)", &context);
    });

    benchmark("builtin/math/max", || {
        let _ = engine.evaluate("max(1, 2, 3, 4, 5)", &context);
    });

    benchmark("builtin/conversion/to_string", || {
        let _ = engine.evaluate("to_string(42)", &context);
    });
}

// Individual benchmark tests (can run separately)

#[test]
#[ignore]
fn bench_template_parse() {
    println!("\nTemplate Parse Benchmark:");
    benchmark("template/parse/simple", || {
        let _ = Template::new("Hello {{ $input }}!");
    });
}

#[test]
#[ignore]
fn bench_template_render() {
    println!("\nTemplate Render Benchmark:");
    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::text("World"));
    let template = Template::new("Hello {{ $input }}!").unwrap();

    benchmark("template/render", || {
        let _ = template.render(&engine, &context);
    });
}

#[test]
#[ignore]
fn bench_engine_eval() {
    println!("\nEngine Eval Benchmark:");
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    benchmark("engine/eval/arithmetic", || {
        let _ = engine.evaluate("2 + 3 * 4", &context);
    });
}

#[test]
#[ignore]
fn bench_concurrent() {
    use std::sync::Arc;
    use std::thread;

    println!("\nConcurrent Benchmark:");
    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    benchmark("concurrent/8_threads", || {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let engine = Arc::clone(&engine);
                thread::spawn(move || {
                    let ctx = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = engine.evaluate(expr, &ctx);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    });
}
