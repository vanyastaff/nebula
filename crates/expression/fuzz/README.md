# nebula-expression fuzz harness

Coverage-guided fuzzing for the public parsing surface of
`nebula-expression`. Excluded from the workspace (`exclude =
["crates/*/fuzz"]`) so it does not affect routine `cargo` invocations.

## Targets

| Target              | Surface exercised                                   |
| ------------------- | --------------------------------------------------- |
| `parse_expression`  | `nebula_expression::parse_expression` (parser path) |
| `parse_template`    | `Template::new` (template state machine)            |
| `tokenize`          | `lexer::Lexer::tokenize` (lexer-only failures)      |

## Running

Requires nightly + `cargo-fuzz`:

```bash
cargo +nightly install cargo-fuzz
cd crates/expression/fuzz

# Quick smoke (60 s):
cargo +nightly fuzz run parse_expression -- -max_total_time=60
cargo +nightly fuzz run parse_template   -- -max_total_time=60
cargo +nightly fuzz run tokenize         -- -max_total_time=60

# Long soak (1 hour) on a single target:
cargo +nightly fuzz run parse_expression -- -max_total_time=3600
```

`cargo-fuzz` writes corpus and crashes under `crates/expression/fuzz/`
in a `corpus/` and `artifacts/` directory respectively. Crashes get
their own minimised reproducer that you can replay with:

```bash
cargo +nightly fuzz run parse_expression artifacts/parse_expression/<crash-file>
```

## Adding a target

1. Add a file in `fuzz_targets/<name>.rs` modelled on the existing
   ones (`#![no_main]` + `libfuzzer_sys::fuzz_target!`).
2. Append a matching `[[bin]]` block to `Cargo.toml`.
3. Document it in this README.
