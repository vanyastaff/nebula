# Cluster 03: Cargo & rustc — Expert Notes

Dense notes for LLM consumption, tagged: **09-performance**, **10-testing-and-tooling**, **11-ecosystem-crate-picks**, **12-modern-rust**.

**Primary sources (fetched TOC + reference chapters):**


| Book               | Base URL                                                                             | Scope used here                                                                                                                                                                                                                                                                                                              |
| ------------------ | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The Cargo Book     | [https://doc.rust-lang.org/cargo/](https://doc.rust-lang.org/cargo/)                 | Manifest, dependencies, features, profiles, workspaces, resolver, build scripts, config, env vars, **[build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)**, **[registries](https://doc.rust-lang.org/cargo/reference/registries.html)**, publishing, source replacement, targets, unstable flags index |
| The rustc Book     | [https://doc.rust-lang.org/rustc/](https://doc.rust-lang.org/rustc/)                 | CLI, codegen (`-C`), `--emit`, `--check-cfg` (+ [Cargo specifics](https://doc.rust-lang.org/rustc/check-cfg/cargo-specifics.html)), JSON diagnostics, PGO, linker-plugin-LTO, instrument-coverage, platform tiers                                                                                                            |
| Rust Unstable Book | [https://doc.rust-lang.org/unstable-book/](https://doc.rust-lang.org/unstable-book/) | **Sanitizers** (`-Zsanitizer=…`) — there is no `rustc/sanitizer.html`; use Unstable Book “sanitizer” chapter                                                                                                                                                                                                                 |


Getting-started chapters were skipped per research brief.

---

## 09-PERFORMANCE

### Release profile tuning — the single knob set that matters

- `cargo build --release` uses `[profile.release]`. Defaults: `opt-level = 3`, `debug = false`, `debug-assertions = false`, `overflow-checks = false`, `lto = false`, `panic = "unwind"`, `incremental = false`, `codegen-units = 16`, `strip = "none"`.
- Only the workspace root manifest's `[profile.*]` is honored. Profile entries in member crates are ignored. For dependency-specific overrides you must declare them at the root.

```toml
# Cargo.toml (workspace root)
[profile.release]
opt-level = 3          # 0|1|2|3|"s"|"z"
lto = "fat"            # true|"fat"|"thin"|false|"off"
codegen-units = 1      # 1 for best perf; default 16
panic = "abort"        # ~10% size/speed win; loses std::panic::catch_unwind
strip = "symbols"      # none|"debuginfo"|"symbols"; also true/false
debug = false          # false|0|1|2|true|"line-tables-only"|"line-directives-only"|"limited"|"full"
overflow-checks = false
incremental = false
split-debuginfo = "packed"   # off|packed|unpacked
```

### opt-level: what each value means

- `0`: no optimizations. Fastest compile. Auto-enables `debug-assertions` unless explicitly disabled.
- `1`: basic opts; useful for dependencies in dev builds (`opt-level = 1` for `*` package override gives ~2–3x runtime without huge compile cost).
- `2`: "some" optimizations (LLVM's default).
- `3`: "all" optimizations. Default for release. `-O` is `-C opt-level=3`.
- `"s"`: optimize for size.
- `"z"`: aggressive size opt, disables loop vectorization. Smallest binaries (embedded, Wasm).

Practical dev-speed trick — optimize dependencies only:

```toml
[profile.dev]
opt-level = 0

[profile.dev.package."*"]   # all non-workspace dependencies
opt-level = 2               # stdlib-ish perf for deps, fast compile for your crate
```

### LTO — Link-Time Optimization

- `lto = false` (default): "thin-local" LTO on the local crate. Not real cross-crate LTO.
- `lto = "thin"`: real cross-crate ThinLTO. Typically 5–10% faster binaries, moderate link-time cost. Usually best bang-for-buck.
- `lto = "fat"` or `lto = true`: whole-program LTO. Biggest wins (10–20%+) but long linker time, high RAM.
- `lto = "off"`: completely disables, even thin-local.
- Fat LTO + `codegen-units = 1` are frequently combined for max perf.
- LTO requires `embed-bitcode = yes` (default). `-C embed-bitcode=no` cannot combine with `-C lto`.

### codegen-units — parallelism vs runtime

- Default: `16` for non-incremental, `256` for incremental.
- `codegen-units = 1` gives ~1–5% runtime win but serializes backend; combine with LTO for the squeezed binary.
- Higher values parallelize codegen → faster compile, slightly worse optimization (inlining limited to within a unit when LTO is off).

### target-cpu / target-feature — instruction-set tuning

- `RUSTFLAGS="-C target-cpu=native" cargo build --release` enables all ISA extensions of the host; often 5–30% win on number-crunching.
- `-C target-cpu=x86-64-v3` picks a microarchitecture level (v1/v2/v3/v4; v3 ≈ Haswell+AVX2).
- `-C target-feature=+avx2,+fma,-sse4.1` granular feature control. Unsafe if binary is shipped to older CPUs — will crash with illegal instruction.
- Discover: `rustc --print target-cpus`, `rustc --print target-features`.

```bash
# Workstation-only build
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Portable modern x86_64
RUSTFLAGS="-C target-cpu=x86-64-v3" cargo build --release
```

Put in `.cargo/config.toml` for persistence:

```toml
[build]
rustflags = ["-C", "target-cpu=native"]
# or target-specific:
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=x86-64-v3"]
```

### panic = "abort" — the cheap binary size win

- Aborts on panic instead of unwinding. Smaller binary, faster call paths (no landing pads in LLVM IR).
- Loses: `std::panic::catch_unwind`. Crates relying on this (some FFI boundaries) break.
- Tests, benches, build scripts, proc-macros always use `unwind` regardless.
- On stable, `cargo test` requires `panic = unwind` to build tests. Use `-Z panic-abort-tests` on nightly if you need `panic=abort` tests.

### strip — shrink binaries post-link

- `strip = "none"` (default), `"debuginfo"`, `"symbols"`. Booleans `true=symbols`, `false=none`.
- For size-sensitive releases: combine `lto=true`, `codegen-units=1`, `opt-level="z"`, `strip="symbols"`, `panic="abort"`. Typical stripping of 10-20 MB binaries down to 1-2 MB.

### split-debuginfo — separate .dSYM / .pdb / split DWARF

- `off`: everything in executable (Linux default historically).
- `packed`: separate single file (macOS `.dSYM`, Windows `.pdb` default).
- `unpacked`: per-CU debug files (faster link on Linux for big crates).
- Speeds up linking substantially when dev builds include debuginfo.

### Incremental compilation

- `incremental = true` speeds rebuilds (default in dev). Adds some codegen overhead and larger target dir.
- Off by default in release — release builds benefit more from whole-program optimizations.
- `CARGO_INCREMENTAL=0` or `build.incremental = false` (config) to disable globally.

### Profile-Guided Optimization (PGO)

Two-phase with LLVM profdata. Typical 5–15% speedup on branchy workloads.

```bash
# 0. llvm-profdata tool
rustup component add llvm-tools-preview

# 1. Build instrumented binary
RUSTFLAGS="-Cprofile-generate=/tmp/pgo" \
  cargo build --release --target x86_64-unknown-linux-gnu

# 2. Run representative workloads (emits .profraw files)
./target/x86_64-unknown-linux-gnu/release/mybin work1.dat
./target/x86_64-unknown-linux-gnu/release/mybin work2.dat

# 3. Merge
llvm-profdata merge -o /tmp/pgo/merged.profdata /tmp/pgo

# 4. Rebuild using profile
RUSTFLAGS="-Cprofile-use=/tmp/pgo/merged.profdata -Cllvm-args=-pgo-warn-missing-function" \
  cargo build --release --target x86_64-unknown-linux-gnu
```

Notes: always pass `--target` so build scripts aren't instrumented. Use absolute paths in `RUSTFLAGS`. Sensitive to working-set realism — trash profile = worse perf. `cargo-pgo` crate automates steps 1–4.

Stack with LTO: build step 4 with `-Clto=fat -Ccodegen-units=1` for maximum cumulative effect.

### Cross-language LTO with linker-plugin-lto

Interprocedural opts across Rust↔C/C++ boundary; requires matching LLVM versions and LLD.

```bash
# Rust staticlib consumed by clang
RUSTFLAGS="-Clinker-plugin-lto" cargo build --release
clang -c -O2 -flto=thin -o cmain.o cmain.c
clang -flto=thin -fuse-ld=lld -L . -lrustlib -O2 -o main cmain.o

# C static lib consumed by Rust
clang foo.c -flto=thin -c -o foo.o -O2
ar crus libfoo.a foo.o
RUSTFLAGS="-Clinker-plugin-lto -Clinker=clang -Clink-arg=-fuse-ld=lld" \
  cargo build --release
```

Known good combos: rustc 1.87-1.90 ↔ clang 20; 1.91+ ↔ clang 21. Check with `rustc -vV | grep LLVM`.

### BOLT — post-link binary optimization (not in rustc book)

BOLT (Binary Optimization and Layout Tool) is applied *after* linking, separate from PGO. Not part of the Rust toolchain proper but commonly applied to release binaries. It reorders basic blocks / functions based on perf profiles. Chain: build with LTO+PGO → run under `perf record` → `llvm-bolt -instrument` and/or `llvm-bolt` to rewrite. Typical 2–5% beyond PGO. See `cargo-pgo --bolt`.

### Custom release profile

```toml
[profile.release-lto]
inherits = "release"
lto = "fat"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

Build: `cargo build --profile release-lto`. Output: `target/release-lto/`.

**Size bundle (cross-ref):** `opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true`; optional `RUSTFLAGS="-C target-feature=+crt-static"` (Linux dyn), nightly `RUSTFLAGS="-Zlocation-detail=none"`.

### build-override — profile for proc-macros / build.rs

Proc-macros and build scripts default to `opt-level=0` and no debuginfo regardless of profile (they are host tools). To make proc-macro-heavy builds faster for downstream compile you can raise this:

```toml
[profile.dev.build-override]
opt-level = 3

[profile.release.build-override]
opt-level = 0       # keep compile-time low
```

### Profile override restrictions

`[profile.*.package.NAME]` can override most settings but NOT: `panic`, `lto`, `rpath` (these are link-time global).

---

## 10-TESTING AND TOOLING

### cargo test — structure

- Unit tests: `#[test]` fns inside `src/` (including `src/lib.rs`, `src/main.rs`, `src/**/*.rs`). Access private items.
- Integration tests: each `tests/*.rs` is its own crate (separate binary). Only sees the crate's `pub` API.
- Doc tests: `///` code blocks in doc comments. Compiled and run by `rustdoc`.
- `cargo test [TESTNAME]` filters by substring. Target selection flags mirror `cargo build`.

```bash
cargo test                              # everything
cargo test --lib                        # unit tests only
cargo test --doc                        # doc tests only
cargo test --test integration_name      # one integration test file
cargo test my_fn                        # substring filter
cargo test my_fn --exact                # exact match (-- to pass through)
cargo test -- --nocapture               # show println! output
cargo test -- --test-threads=1          # serial
cargo test -- --ignored                 # run #[ignore]d tests
cargo test -- --show-output             # show output of passing tests too
cargo test --no-run                     # compile only
cargo test --no-fail-fast               # don't abort on first failure
cargo test --release                    # optimized test binaries
cargo test --workspace --exclude slow-crate
```

After `--`: arguments go to libtest (`rustc --test` harness). Before `--`: to Cargo.

Tests run with CWD = package root → fixture paths like `tests/data/foo.json` work.

### Custom test harness — `harness = false`

```toml
[[test]]
name = "custom"
path = "tests/custom.rs"
harness = false
```

Then `tests/custom.rs` must have its own `fn main()` (used by libtest replacements like `datatest`, `criterion`, `trybuild`).

### libtest harness (`rustc --test`) — [rustc book — Tests](https://doc.rust-lang.org/rustc/tests/index.html)

- `rustc --test`: builds a **bin** linked with **libtest**, synthesizes a `main` that replaces yours as entry (your `main` still compiles), enables `cfg(test)`, compiles `#[test]` / `#[bench]`.
- Pass: return without panic; fail: panic or `Result` with non-zero `Termination`. `#[should_panic]`, `#[ignore]` as in reference.
- **Panic strategy:** tests need **unwind** (same process catches panics). `abort` incompatible unless nightly `**-Z panic-abort-tests`** (separate processes); related: `--force-run-in-process` + unstable options.
- **Filters (after `--`):** positional args = substring match on full test path; `--exact` = full path only; `--skip` (repeatable).
- **Selection:** `--test` (default: tests + bench smoke), `--bench` (benches only), `--ignored`, `--include-ignored`; unstable `--exclude-should-panic` needs `-Z unstable-options`.
- **Execution:** `--test-threads` / `RUST_TEST_THREADS`; unstable `--fail-fast`, `--shuffle` / `--shuffle-seed` (`RUST_TEST_SHUFFLE`, `RUST_TEST_SHUFFLE_SEED`), `--ensure-time`, `--report-time` — all need `-Z unstable-options` where marked in rustc book.
- **Output:** `--no-capture` / `RUST_TEST_NOCAPTURE`, `--show-output`, `--format=pretty|terse|json` (json unstable + `-Z unstable-options`), `--quiet`.
- Nightly: custom test frameworks ([custom_test_frameworks](https://doc.rust-lang.org/unstable-book/language-features/custom-test-frameworks.html)); `#[bench]` unstable — see unstable book.

### cargo check — the linter without codegen

- `cargo check` runs rustc to type-check but skips codegen. Typically 2–5× faster than `cargo build`. Produces `.rmeta` metadata files in `target/.../deps/`.
- Use in pre-commit hooks / `cargo-watch -x check` for continuous feedback.
- `cargo check --all-targets --all-features --workspace` is the comprehensive CI invocation.

### cargo clippy — linter

- Extra lint pass. Lints grouped:
  - `clippy::correctness` (deny by default — real bugs)
  - `clippy::suspicious` (warn; surprising code)
  - `clippy::complexity` (warn; simplifications)
  - `clippy::perf` (warn; faster alternatives)
  - `clippy::style` (warn; idiomatic style)
  - `clippy::pedantic` (allow; opinionated)
  - `clippy::nursery` (allow; WIP lints)
  - `clippy::restriction` (allow; opt-in restrictions, not to be blanket-enabled)
  - `clippy::cargo` (allow; Cargo.toml lints)

```bash
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo clippy --fix --allow-dirty --allow-staged
cargo clippy -- -W clippy::pedantic -A clippy::module_name_repetitions
```

Configuration: `clippy.toml` in repo root or `[lints.clippy]` in `Cargo.toml` (see §12).

### cargo fmt

- Not documented in the Cargo book (lives with rustfmt). Runs `rustfmt` across workspace. `cargo fmt --check` for CI.

### cargo fix — apply compiler suggestions automatically

```bash
cargo fix                                 # apply MachineApplicable suggestions
cargo fix --edition                       # migrate code to next edition
cargo fix --edition-idioms                # apply idiom lints for current edition
cargo fix --allow-dirty --allow-staged    # bypass VCS cleanliness check
cargo fix --broken-code                   # keep going through errors
cargo fix --features foo --target x86_64-pc-windows-gnu
```

Cargo fix only touches code that compiles under the active cfg — run multiple times with different `--features`/`--target` for full coverage.

### Continuous integration — [Cargo guide](https://doc.rust-lang.org/cargo/guide/continuous-integration.html)

- **Baseline:** `cargo build` + `cargo test` (often `--verbose`); matrix **stable / beta / nightly** — note any channel failure fails the job unless split.
- **“Latest deps” job:** `cargo update` then build/test; trade-offs vs lockfile determinism — optional `continue-on-error`, scheduled job, or Dependabot/Renovate PRs. Example sets `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=allow` so resolver does not avoid crates newer than your `rust-version`.
- `**rust-version` verification:** e.g. `cargo hack check --rust-version --workspace --all-targets --ignore-private` (third-party tool in doc example); single OS + `cargo check` balances cost vs API breakage.
- Other hosts: GitLab (`rust:latest` vs `rustlang/rust:nightly` + `allow_failure`), CircleCI `cimg/rust`, sr.ht — doc gives patterns; GitHub starter workflow linked from guide.

### cargo bench

- Uses `bench` profile (inherits from `release`).
- On stable, the built-in `#[bench]` attribute is unstable — use **Criterion** (`criterion` crate) with `harness = false`.
- Benchmarks default path: `benches/*.rs`.

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "my_bench"
harness = false
```

```bash
cargo bench                          # all
cargo bench --bench my_bench         # one target
cargo bench -- my_case --exact       # filter
cargo bench --no-run                 # compile only
```

### Profiles — full default table

```
dev         used by cargo build/run/check/rustc (and inherits for test)
release     used by cargo install, --release
test        inherits from dev. Used by cargo test.
bench       inherits from release. Used by cargo bench.
```

All tuning is via `[profile.NAME]` in root Cargo.toml:

```toml
[profile.dev]
opt-level = 0
debug = true
debug-assertions = true
overflow-checks = true
lto = false
panic = "unwind"
incremental = true
codegen-units = 256
rpath = false

[profile.test]
# inherits dev; override specific fields only

[profile.release]
opt-level = 3
debug = false
debug-assertions = false
overflow-checks = false
lto = false
panic = "unwind"
incremental = false
codegen-units = 16
```

Custom profile syntax (must set `inherits`):

```toml
[profile.fuzzing]
inherits = "release"
debug = 1
overflow-checks = true
lto = false
```

Selection precedence within a profile override: `[profile.NAME.package.foo]` > `[profile.NAME.package."*"]` > `[profile.NAME.build-override]` > `[profile.NAME]` > defaults.

### Workspaces — semantics (full TOML patterns → §12)

- Package root or **virtual** manifest (`[workspace]` only). Virtual → must set `resolver` (no `[package].edition` to infer).
- `members` globs; `exclude`; path deps become members. One `Cargo.lock`, one `target/`. Only root: `[patch]`, `[replace]`, `[profile.*]`.
- `workspace.package` / `workspace.dependencies` / `workspace.lints` + member `*.workspace = true` — MSRV 1.64+ / 1.74+ for lints. Member cannot mark inherited dep `optional`; extra `features` at use site OK.

### Features — the rules you must know

Core rules:

1. **Features must be additive.** Enabling a feature must not disable anything or change semantics breakingly. Never create a `no_std` feature — create a `std` feature.
2. **Cargo unifies features** across the whole build. If crate A asks for `serde/derive` and crate B asks for `serde/alloc`, serde is built with both.
3. `default` feature is implicit. Disable via `default-features = false` or `--no-default-features`.
4. Adding a feature is a minor change; removing one is SemVer-breaking.
5. crates.io limit: 300 features per crate.

```toml
[features]
default  = ["std"]
std      = ["alloc"]
alloc    = []
async    = ["dep:tokio"]
full     = ["std", "async"]

[dependencies]
tokio = { version = "1", optional = true }
```

### The `dep:` prefix (Rust 1.60+)

Optional deps implicitly create same-named features. Use `dep:` to suppress that and keep the dep a hidden implementation detail:

```toml
[dependencies]
serde = { version = "1", optional = true }
rgb   = { version = "0.8", optional = true }

[features]
# Enables serde+rgb without exposing them as features
json = ["dep:serde", "dep:rgb"]
```

Without `dep:`, features `serde` and `rgb` would exist implicitly. Removing that implicit exposure is **SemVer-breaking** — be careful retrofitting.

### Weak features — `?/` syntax

Enable a feature on an optional dep *only if* that dep is already pulled in by something else:

```toml
[dependencies]
serde = { version = "1", optional = true }
rgb   = { version = "0.8", optional = true }

[features]
serde = ["dep:serde", "rgb?/serde"]
# "rgb?/serde" => if rgb is enabled (by anyone), also enable serde feature of rgb.
# Without ?, "rgb/serde" would force-enable rgb as well.
```

### Transitive feature activation — `pkg/feat`

```toml
[features]
parallel = ["jpeg-decoder/rayon"]
```

Enables `rayon` feature of the already-declared `jpeg-decoder` dependency.

### Command-line features

```bash
cargo build --features "foo,bar"
cargo build -F foo -F bar
cargo build --all-features
cargo build --no-default-features --features minimal
# Resolver-2-only: select features across multiple -p targets:
cargo build -p foo -p bar --features foo-feat,bar-feat
```

Build script sees `CARGO_FEATURE_<NAME>=1` env vars (uppercased, `-` → `_`).

### Feature unification across target kinds — resolver "2" differences

Resolver v1 unifies features across *all* build kinds; this leaks features from dev/build-deps into the final crate.

Resolver v2 separates:

- Platform dependencies for non-target platforms: not unified.
- Build-dependencies and proc-macros: not unified with normal deps.
- Dev-dependencies: not unified with normal deps (unless a test/example is actually being built).

Result: fewer unintended feature activations, possibly more duplicate builds. Check with `cargo tree --duplicates`.

### build.rs — full reference

Placement: `build.rs` in package root. Cargo compiles and runs it before building the crate. Output streamed as `cargo::KEY=VALUE` on stdout.

MSRV notes:

- `cargo::KEY=VALUE` (new form) requires **Cargo 1.77+**. Old form `cargo:KEY=VALUE` (single colon) still works but is deprecated syntax for 1.77+ directives.
- `cargo::rustc-check-cfg` requires 1.80+.
- `cargo::error` requires 1.84+.

Directives:

```rust
// build.rs
fn main() {
    // Change detection — without any rerun-if-* Cargo re-runs on ANY source change
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/c_src/");
    println!("cargo::rerun-if-env-changed=MY_TOOL_PATH");

    // Link native libs
    println!("cargo::rustc-link-lib=dylib=foo");       // -l dylib=foo
    println!("cargo::rustc-link-lib=static=bar");      // -l static=bar
    println!("cargo::rustc-link-search=native=/opt/lib");
    println!("cargo::rustc-link-arg=-Wl,--as-needed"); // passed to linker
    println!("cargo::rustc-link-arg-bin=mybin=-Tscript.ld");
    println!("cargo::rustc-link-arg-cdylib=-Wl,-soname,libfoo.so");

    // cfg for conditional compilation in THIS crate
    println!("cargo::rustc-cfg=has_simd");
    println!("cargo::rustc-cfg=backend=\"native\"");

    // Declare expected cfgs (silences unexpected_cfgs)
    println!("cargo::rustc-check-cfg=cfg(has_simd)");
    println!("cargo::rustc-check-cfg=cfg(backend, values(\"native\", \"js\"))");

    // Env vars visible via env!() in the crate
    println!("cargo::rustc-env=GIT_HASH=abc123");

    // Metadata for dependent crates (with `links` key)
    println!("cargo::metadata=include_dir=/opt/include/foo");

    // User messages
    println!("cargo::warning=libfoo not found in default paths");
    // println!("cargo::error=could not find required system library");  // 1.84+
}
```

Env vars the build script **reads**:

- `CARGO`, `CARGO_MANIFEST_DIR`, `OUT_DIR` (write target), `TARGET`, `HOST`
- `NUM_JOBS`, `OPT_LEVEL`, `DEBUG` (bool), `PROFILE`
- `CARGO_PKG_`* (NAME, VERSION, AUTHORS, DESCRIPTION, HOMEPAGE, REPOSITORY, LICENSE)
- `CARGO_CFG_`*: `CARGO_CFG_TARGET_OS=linux`, `CARGO_CFG_TARGET_ARCH=x86_64`, `CARGO_CFG_TARGET_POINTER_WIDTH=64`, `CARGO_CFG_TARGET_FAMILY=unix`, `CARGO_CFG_UNIX=1` (no value), `CARGO_CFG_WINDOWS=1`, `CARGO_CFG_TARGET_FEATURE=fxsr,sse,sse2`
- `CARGO_FEATURE_<NAME>=1` for every enabled feature
- `RUSTC`, `RUSTDOC`, `RUSTC_LINKER`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`
- `CARGO_ENCODED_RUSTFLAGS` (0x1F-separated list of extra flags)
- `CARGO_MANIFEST_LINKS` (value of `[package].links`)

**Critical**: inside build.rs, use `env::var("CARGO_CFG_TARGET_OS")`, **not** `cfg!(target_os = "...")`. The `cfg!` macro reports the *host* platform where the build script is running, not the target.

Codegen example:

```rust
// build.rs
use std::{env, fs, path::Path};
fn main() {
    let out = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out).join("table.rs");
    fs::write(&dest, "pub const TABLE: [u32; 3] = [1,2,3];").unwrap();
    println!("cargo::rerun-if-changed=build.rs");
}
// In src/lib.rs:
// include!(concat!(env!("OUT_DIR"), "/table.rs"));
```

### `links` manifest key

```toml
[package]
links = "foo"       # this package links native libgoo
```

- Enforces "only one version of a native library" — errors if multiple packages in the graph declare `links = "foo"`.
- Package with `links` **must** have a build script that emits `cargo::rustc-link-lib=foo` (typically indirectly via `pkg-config`/`cc` crates).
- Metadata emitted by a `*-sys` crate's build script propagates to dependents as `DEP_<LINKS>_<KEY>`. `libgit2-sys` with `links = "git2"` and `cargo::metadata=version=1.5.0` → `git2` crate sees `DEP_GIT2_VERSION=1.5.0`.
- Convention: `foo-sys` crate provides raw bindings + linking; `foo` crate builds a safe wrapper.

### Overriding a build script from config.toml

If a package has `links = "foo"`, downstream users can override its build script entirely:

```toml
# .cargo/config.toml
[target.x86_64-unknown-linux-gnu.foo]
rustc-link-lib    = ["foo"]
rustc-link-search = ["/opt/foo/lib"]
rustc-cfg         = ['foo_version="1.5.0"']
rustc-env         = { FOO_VERSION = "1.5.0" }
rustc-cdylib-link-arg = ["-Wl,-soname,libfoo.so"]
# Any custom metadata:
version = "1.5.0"
```

When overridden, the build script is **not** compiled or executed. `warning`, `rerun-if-`* are ignored.

### config.toml — hierarchical configuration

Files merged from CWD up to `$CARGO_HOME/config.toml`. Precedence: CLI `--config` > env vars `CARGO_`* > nearest config file > default.

Locations walked:

```
./.cargo/config.toml
../.cargo/config.toml
...
$CARGO_HOME/config.toml     # ~/.cargo/config.toml (Unix) or %USERPROFILE%\.cargo\config.toml (Win)
```

Key sections:

```toml
# Command aliases
[alias]
c  = "check"
t  = "test"
b  = "build"
ck = "check --all-targets --all-features"
rr = ["run", "--release"]

# Build defaults
[build]
jobs         = 8                                     # default: CPU count
target       = "x86_64-unknown-linux-gnu"            # default build target
target-dir   = "target"
incremental  = true
rustflags    = ["-C", "target-cpu=x86-64-v3"]
rustdocflags = ["-D", "warnings"]
rustc-wrapper = "sccache"                            # wraps every rustc invocation
rustc-workspace-wrapper = "cargo-nextest"            # wraps only workspace members

# cargo new defaults
[cargo-new]
vcs = "git"   # git, hg, pijul, fossil, none

# Env injection into cargo invocations
[env]
OPENSSL_DIR = "/opt/openssl"
RUSTFLAGS   = { value = "-C link-arg=-fuse-ld=lld", force = false }
RELATIVE    = { value = "vendor/openssl", relative = true }

# HTTP
[http]
timeout        = 30
low-speed-limit = 10
multiplexing   = true
check-revoke   = true

# Network
[net]
retry             = 3
git-fetch-with-cli = true     # use system git (SSH, config files, keyring)
offline           = false

# Install
[install]
root = "~/.cargo"

# Profiles (override root Cargo.toml)
[profile.release]
lto = "thin"

# Per-target rustflags / linker / runner (cross-compile)
[target.x86_64-unknown-linux-gnu]
linker    = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[target.'cfg(all(target_arch = "arm", target_os = "none"))']
runner    = "qemu-system-arm"
rustflags = ["-C", "link-arg=-Tmylink.ld"]

# Source replacement
[source.crates-io]
replace-with = "vendored"

[source.vendored]
directory = "vendor"

# Patch (same syntax as [patch] in Cargo.toml)
[patch.crates-io]
serde = { path = "../my-serde" }

# Registries
[registries]
my-corp = { index = "sparse+https://corp.example.com/crates/" }

# Resolver
[resolver]
incompatible-rust-versions = "fallback"   # MSRV-aware; default for resolver=3

# Terminal
[term]
color      = "auto"      # auto|always|never
verbose    = false
quiet      = false
progress   = { when = "auto", width = 80 }

# Cache
[cache]
auto-clean-frequency = "1 day"   # or "never", "always", "2 weeks", "3 months"

# Future compatibility reports
[future-incompat-report]
frequency = "always"   # or "never"
```

Environment variable mapping: `CARGO_<SECTION>_<KEY>` (uppercase, dots/dashes → underscores).

- `CARGO_BUILD_JOBS=4` ≡ `build.jobs = 4`
- `CARGO_TERM_COLOR=always`
- `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=qemu-system-x86_64`
- `CARGO_REGISTRY_TOKEN=<…>` for crates.io, `CARGO_REGISTRIES_MY_CORP_TOKEN=<…>` for others

CLI `--config`:

```bash
cargo --config net.git-fetch-with-cli=true fetch
cargo --config "build.rustflags = ['--cap-lints', 'warn']" build
cargo --config ./extra-config.toml build
```

Credentials file (`$CARGO_HOME/credentials.toml`) — tokens only:

```toml
[registry]
token = "<crates-io-token>"

[registries.my-corp]
token = "<corp-token>"
```

### `[lints]` in Cargo.toml — manifest-based lint config (1.74+)

```toml
[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }   # enable group
module_name_repetitions = "allow"              # override single lint in group
unwrap_used = { level = "deny", priority = 0 }

[lints.rustdoc]
broken_intra_doc_links = "deny"
```

Level values: `allow`, `warn`, `deny`, `forbid`. Priority is signed int; lower priority set first on CLI so higher priority can override. Groups should get low priority (negative) so individual lint overrides work.

Lint table namespacing: key-before-`::` picks the table. `unsafe_code` (no `::`) goes in `[lints.rust]`. `clippy::unwrap_used` → `[lints.clippy]` with key `unwrap_used`.

Workspace inheritance:

```toml
# workspace root
[workspace.lints.rust]
unsafe_code = "forbid"
[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }

# member crate
[lints]
workspace = true
```

Scope: these lints apply only to the current package. Dependencies' lints are suppressed by Cargo via `--cap-lints` (they can't break your build with their warnings).

### Rustc lint system — levels, groups, CLI

Five levels: `allow`, `expect`, `warn`, `force-warn`, `deny`, `forbid`.

- `allow` silent; `warn` prints; `deny` errors; `forbid` errors and can't be downgraded (except `--cap-lints`).
- `expect`: like `allow`, but compiler warns if the lint *didn't* fire — useful for documenting "we silence this on purpose; warn me if it's no longer relevant".
- `force-warn`: always warns, cannot be overridden by attributes or other flags.

CLI:

```bash
rustc lib.rs -D missing-docs
rustc lib.rs -W unused -A unused-variables          # warn group, allow one lint
rustc lib.rs --force-warn deprecated
rustc lib.rs --cap-lints warn                       # max level = warn (compiler releases use this for dep builds)
rustc lib.rs -A clippy::pedantic                    # on clippy
```

Rightmost CLI flag wins for same lint.

Attributes in source override CLI (except `-F`/forbid cannot be downgraded). `#[allow(...)]`/`#[warn(...)]`/`#[deny(...)]`/`#[forbid(...)]`/`#[expect(...)]` can be inner attributes `#![...]` at crate root, or outer on item/block/fn:

```rust
#![warn(missing_docs)]
#![deny(clippy::pedantic)]

#[allow(unused_variables, reason = "platform-specific")]
fn linux_only() { let handle = 0; }

#[expect(unused_mut)]
fn note() { let mut x = 5; /* compiler fires warn if unused_mut doesn't trigger */ }
```

### Key rustc lint groups

- `warnings` — special: matches every lint set to warn.
- `unused` — dead_code, unused_variables, unused_imports, unused_mut, unreachable_code, unused_assignments, unreachable_patterns (+more).
- `deprecated`, `deprecated-safe`
- `nonstandard-style` — non_camel_case_types, non_snake_case, non_upper_case_globals.
- `future-incompatible` — will become hard errors.
- `rust-2018-idioms`, `rust-2018-compatibility`, `rust-2021-compatibility`, `rust-2024-compatibility`, `keyword-idents`.
- `unknown-or-malformed-diagnostic-attributes`.
- `let-underscore` (let_underscore_drop, let_underscore_lock).
- `refining-impl-trait`.

Enumerate: `rustc -W help` lists every lint+group for the compiler version.

### Warn-by-default rustc lints you'll hit

- `unused_variables`, `unused_imports`, `unused_mut`, `unused_assignments`, `unused_parens`, `unused_braces`
- `dead_code` (private items never used)
- `unused_must_use` (ignoring `#[must_use]` values, notably `Result`)
- `deprecated`
- `non_snake_case`, `non_camel_case_types`, `non_upper_case_globals`
- `bare_trait_objects` (use `dyn Trait`)
- `ellipsis_inclusive_range_patterns` (use `..=`)
- `unreachable_code`, `unreachable_patterns`
- `unconditional_recursion`, `while_true`
- `non_fmt_panics` (`panic!(s)` with dynamic string)
- `redundant_semicolons`
- `renamed_and_removed_lints`
- `tyvar_behind_raw_pointer`
- `unexpected_cfgs` (from `--check-cfg`)
- `clashing_extern_declarations`
- `semicolon_in_expressions_from_macros`

### Allowed-by-default rustc lints worth enabling

- `missing_docs` — public items must have doc comments.
- `missing_debug_implementations`, `missing_copy_implementations`.
- `unsafe_code` — catches `unsafe` blocks, `#[no_mangle]`, `#[export_name]`, `#[link_section]`. Pair with `#[allow(unsafe_code)]` + `reason` where needed.
- `unsafe_op_in_unsafe_fn` — forces explicit `unsafe {}` inside `unsafe fn`. Warn-by-default in 2024 edition.
- `trivial_casts`, `trivial_numeric_casts` — catch `as` uses that coercion would do.
- `unused_crate_dependencies` — deps declared but unused.
- `unused_import_braces`, `unused_lifetimes`, `unused_qualifications`.
- `elided_lifetimes_in_paths` — force `&Foo<'_>` instead of `&Foo`.
- `let_underscore_drop` — `let _ = value_with_Drop;` drops immediately (not end of scope).
- `meta_variable_misuse` — macro hygiene issues.
- `non_ascii_idents`, `keyword_idents`.
- `variant_size_differences` — large enum variant spread (consider `Box`).
- `unused_results` — every non-`()` return ignored.

Classic "strict mode" crate-root block:

```rust
#![deny(
    rust_2018_idioms,
    missing_docs,
    unsafe_code,
    clippy::pedantic,
)]
#![warn(
    unreachable_pub,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications,
    unused_lifetimes,
    elided_lifetimes_in_paths,
)]
#![forbid(unsafe_op_in_unsafe_fn)]
```

### --check-cfg — catching cfg typos

`--check-cfg` validates `#[cfg(...)]` against a declared expected set. Triggers `unexpected_cfgs` lint. Cargo emits `--check-cfg` automatically for every declared feature:

```toml
[features]
default = []
simd = []
avx = []
```

Cargo passes `--check-cfg 'cfg(feature, values("simd", "avx"))'` to rustc. A `#[cfg(feature = "avx2")]` typo warns.

Custom cfgs need explicit declaration. From build.rs:

```rust
// 1.80+
println!("cargo::rustc-check-cfg=cfg(has_neon)");
println!("cargo::rustc-check-cfg=cfg(backend, values(\"native\", \"wasm\"))");
if detect_neon() {
    println!("cargo::rustc-cfg=has_neon");
}
```

Well-known names (auto-added when any `--check-cfg` present): `target_os`, `target_arch`, `target_family`, `target_env`, `target_abi`, `target_pointer_width`, `target_endian`, `target_vendor`, `target_feature`, `target_has_atomic`, `target_thread_local`, `debug_assertions`, `doc`, `doctest`, `test`, `panic`, `proc_macro`, `unix`, `windows`, `overflow_checks`, `relocation_model`, `clippy`, `miri`, `rustfmt`, `sanitize`, `ub_checks`.

Suppress specific warnings: `#[allow(unexpected_cfgs)]` or `#![allow(unexpected_cfgs)]`.

**Static custom cfgs (no `build.rs`):** manifest can pass the same expectations via lint `unexpected_cfgs` and its `check-cfg` field ([rustc book — Cargo specifics](https://doc.rust-lang.org/rustc/check-cfg/cargo-specifics.html)):

```toml
[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(has_foo)', 'cfg(backend, values("native", "wasm"))'] }
```

Use when the cfg set is fixed in advance and not generated by a script. For probe-driven cfgs, still use `cargo::rustc-check-cfg` + `cargo::rustc-cfg` in `build.rs` (Cargo ≥ 1.80 for `cargo::` directives).

### Sanitizers (nightly, `-Z sanitizer=`)

**Doc home:** [https://doc.rust-lang.org/unstable-book/compiler-flags/sanitizer.html](https://doc.rust-lang.org/unstable-book/compiler-flags/sanitizer.html) (Rust Unstable Book). The rustc book does not expose a standalone sanitizer chapter at `rustc/sanitizer.html` (404 as of 2026).

Instrumentation-based runtime checkers. Not for production (the "unsafe" group); some are production-safe.

Testing/fuzz-only:

- `address` — heap/stack/global OOB, UAF, use-after-return, double-free, leaks. Linux + macOS (x86_64, aarch64).
- `hwaddress` — like address, lower memory overhead, ARM64 only, needs `-C target-feature=+tagged-globals`.
- `leak` — memory leak detector.
- `memory` — uninitialized read detector. Needs full-program instrumentation (`-Zbuild-std`) and C/C++ code with clang `-fsanitize=memory`.
- `thread` — data race detector.
- `realtime` — non-deterministic calls in `#[sanitize(realtime = "nonblocking")]` functions.

Production-safe:

- `cfi` — Control-Flow Integrity, forward edge; requires `-Clinker-plugin-lto`.
- `kcfi` — Kernel CFI.
- `safestack` — separates safe/unsafe stack.
- `shadow-call-stack` — ARM64, RISC-V; return address on shadow stack.
- `memtag` — ARMv8.5 MTE hardware.
- `dataflow` — generic dataflow analysis.

Workflow:

```bash
rustup +nightly component add rust-src
RUSTFLAGS="-Zsanitizer=address" cargo +nightly build \
    -Zbuild-std --target x86_64-unknown-linux-gnu

# for tests — cargo test needs --target so sanitizers skip build scripts:
RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test \
    -Zbuild-std --target x86_64-unknown-linux-gnu
```

ASan runtime env vars: `ASAN_OPTIONS=detect_stack_use_after_return=1:check_initialization_order=1`.

### instrument-coverage — code coverage

Rust uses LLVM source-based coverage.

```bash
rustup component add llvm-tools-preview

RUSTFLAGS="-Cinstrument-coverage" \
  LLVM_PROFILE_FILE="coverage-%p-%m.profraw" \
  cargo test

llvm-profdata merge -sparse coverage-*.profraw -o cov.profdata

llvm-cov report \
  --use-color --ignore-filename-regex='/.cargo/registry' \
  --instr-profile=cov.profdata \
  --object target/debug/deps/mycrate-*

llvm-cov show \
  --format=html --output-dir=target/cov \
  --instr-profile=cov.profdata \
  --object target/debug/deps/mycrate-*
```

Include doc tests (nightly):

```
RUSTDOCFLAGS="-Cinstrument-coverage -Zunstable-options --persist-doctests target/debug/doctestbins"
```

`LLVM_PROFILE_FILE` format specifiers: `%p` pid, `%h` host, `%m` binary signature (enables online merging), `%t` TMPDIR.

Metrics: function, instantiation (per-monomorphization), line, region. Turn off a function:

```rust
#![feature(coverage)]
#[coverage(off)]
fn bootstrapping_code() {}
```

Practical wrappers: `**cargo-llvm-cov**` and `**cargo-tarpaulin**` (see §11).

### Messaging formats — --message-format / --error-format

`cargo build --message-format=json` emits line-delimited JSON to stdout:

- `reason = "compiler-message"` wraps rustc diagnostics.
- `reason = "compiler-artifact"` reports final outputs (executables, rlibs) with paths.
- `reason = "build-script-executed"` reports build.rs output.
- `reason = "build-finished"` with success bool.

Variants:

- `human` (default), `short` — compact human
- `json`, `json-diagnostic-short`, `json-diagnostic-rendered-ansi`, `json-render-diagnostics`.

rustc flags:

- `--error-format=json` and `--json=diagnostic-short,artifacts,future-incompat,unused-externs,timings` (comma-list).
- Diagnostic schema: `{ "$message_type":"diagnostic", "level","message","code":{"code","explanation"}, "spans":[{file_name,line_start,byte_start,column_start,is_primary,text,suggested_replacement,suggestion_applicability:"MachineApplicable"|"MaybeIncorrect"|"HasPlaceholders"|"Unspecified"}], "children":[...], "rendered": "..." }`.

Use `cargo_metadata` crate to parse `cargo build --message-format=json` output in tools.

### --emit / --print (rustc)

`--emit`: control output artifacts, comma-separated: `asm`, `llvm-bc`, `llvm-ir`, `obj`, `metadata`, `link`, `dep-info`, `mir`.

```bash
rustc --emit=llvm-ir -O src/main.rs           # produces main.ll
rustc --emit=asm -O src/main.rs               # produces main.s
rustc --emit=mir --edition=2021 src/main.rs
cargo rustc --release -- --emit=asm=-         # stdout
```

`--print`: dump compiler info, no compilation:

- `cfg` — active cfg for this target
- `target-list` — all supported targets
- `target-cpus` — CPUs valid for `-C target-cpu`
- `target-features` — features valid for `-C target-feature`
- `sysroot`, `host-tuple`, `rustc-version`, `link-args`
- `native-static-libs` (with cdylib/staticlib crate-type)

```bash
rustc --print=cfg --target=wasm32-unknown-unknown
rustc --print=target-cpus --target=x86_64-unknown-linux-gnu
```

### Full rustc codegen flag cheat sheet (-C)

Core:

- `opt-level=0|1|2|3|s|z` — optimization (`-O` = `-Copt-level=3`)
- `debuginfo=0|1|2|line-tables-only|line-directives-only|limited|full` (`-g` = `-Cdebuginfo=2`)
- `overflow-checks=yes|no`, `debug-assertions=yes|no`
- `panic=unwind|abort|immediate-abort`
- `incremental=<path>`
- `codegen-units=<N>`

Link/LTO:

- `lto=off|false|true|fat|thin`
- `embed-bitcode=yes|no`
- `linker-plugin-lto[=path]`
- `linker=<exe>`, `linker-flavor=gcc|msvc|ld|lld-link|wasm-ld|em`
- `link-arg=...`, `link-args="..."`, `link-arg-bin=NAME=...`, `link-arg-cdylib=...`
- `link-self-contained=yes|no|+linker|-linker`
- `linker-features=+lld|-lld`
- `relocation-model=static|pic|pie|dynamic-no-pic|ropi|rwpi`
- `code-model=tiny|small|medium|large|kernel`
- `default-linker-libraries=yes|no`
- `rpath=yes|no`
- `prefer-dynamic=yes|no`
- `strip=none|debuginfo|symbols`
- `split-debuginfo=off|packed|unpacked`
- `dwarf-version=2|4|5`
- `relro-level=off|partial|full`
- `symbol-mangling-version=v0`

CPU/ISA:

- `target-cpu=<name>|native|generic`
- `target-feature=+name,-name,...`
- `soft-float` (deprecated)
- `no-redzone=yes|no`

Profile-guided:

- `profile-generate=<dir>`
- `profile-use=<profdata>`
- `instrument-coverage[=on|off|all]`

LLVM details:

- `llvm-args="..."` — raw flags
- `passes="..."` — extra passes
- `no-prepopulate-passes` — empty pass list
- `no-vectorize-loops`, `no-vectorize-slp`
- `remark=all|pass-name` — print opt remarks
- `save-temps=yes|no`
- `extra-filename=<suffix>`, `metadata="..."`

Security:

- `control-flow-guard=yes|no|nochecks` (Windows)
- `force-frame-pointers=yes|no`
- `force-unwind-tables=yes|no`
- `jump-tables=yes|no`

### Platform support tiers

- **Tier 1 with host tools** — guaranteed to work, CI-tested, official releases include host toolchain. `x86_64-unknown-linux-gnu` (kernel 3.2+, glibc 2.17+), `x86_64-pc-windows-msvc`, `x86_64-pc-windows-gnu`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `aarch64-pc-windows-msvc`, `i686-unknown-linux-gnu`, `i686-pc-windows-msvc`.
- **Tier 2 with host tools** — guaranteed to build, partial test coverage; usable as dev platform. E.g., `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-unknown-freebsd`, `armv7-unknown-linux-gnueabihf`, `powerpc64le-unknown-linux-gnu`, `s390x-unknown-linux-gnu`.
- **Tier 2 without host tools** — std available, cross-compile only. `wasm32-unknown-unknown`, `wasm32-wasip1`, `aarch64-linux-android`, `aarch64-apple-ios`, `aarch64-unknown-none`, `x86_64-unknown-none`, `riscv64gc-unknown-none-elf`, `thumbv7em-none-eabihf`.
- **Tier 3** — code exists, no official binaries, no CI. Use at your own risk.

### Target triple anatomy

`<arch>-<vendor>-<sys>-<abi>` e.g. `aarch64-unknown-linux-gnu`, `wasm32-wasip1`.

List via `rustc --print target-list`. Add via `rustup target add aarch64-unknown-linux-musl`.

---

## 11-ECOSYSTEM CRATE PICKS (cargo extensions)

Most entries below are **community tools** (not shipped with Rust). The **Cargo Book** and **rustc Book** only name a few; everything else is conventional ecosystem practice.

### Testing

- **cargo-nextest** — next-gen test runner. ~60% faster (per-test process isolation, parallel scheduling), better UI, test retries, partitioning (`--partition count:1/3` for CI sharding), JUnit XML. `cargo nextest run --workspace --all-features`. Doc tests NOT supported — still need `cargo test --doc` afterward.
- **cargo-hack** — feature matrix testing and powerset: `cargo hack check --feature-powerset --no-dev-deps`; `cargo hack check --each-feature`; MSRV sweeping via `cargo hack --version-range 1.70..1.85 check`.
- **cargo-mutants** — mutation testing. Introduces bugs and reports tests that still pass. Finds "weak" tests. `cargo mutants`.
- **cargo-tarpaulin** — code coverage (Linux x86_64 primarily). `cargo tarpaulin --out Html --workspace`. Uses ptrace + DWARF. Integrates with Codecov/Coveralls.
- **cargo-llvm-cov** — llvm source-based coverage wrapper. Works on all Tier 1. `cargo llvm-cov --workspace --lcov --output-path lcov.info`. Generally more accurate than tarpaulin.

### Static analysis / security

- **cargo-audit** — scans `Cargo.lock` against RustSec advisories DB. `cargo audit`. CI gate.
- **cargo-deny** — licenses, banned crates, duplicate deps, source policy. Config in `deny.toml`. `cargo deny check`.
- **cargo-machete** — detect unused dependencies (formerly `cargo-udeps` replacement that works on stable). `cargo machete`.
- **cargo-udeps** — unused deps (nightly only — uses internal APIs). `cargo +nightly udeps`.
- **cargo-outdated** — list deps with newer versions. `cargo outdated -R` (root deps only).
- **cargo-edit** — adds `cargo upgrade` (newer semver ranges). `cargo add` and `cargo rm` now built-in since 1.62.
- **cargo-semver-checks** — catch SemVer-breaking changes before publish. `cargo semver-checks`.

### Performance / binary analysis

- **cargo-flamegraph** — one-shot flamegraph via perf (Linux) / dtrace (macOS). `cargo flamegraph --bin mybin -- args`.
- **cargo-bloat** — what's taking up binary size. `cargo bloat --release -n 30` or `cargo bloat --release --crates`.
- **cargo-asm** / **cargo-show-asm** — disassembly of a function/inline monomorphization. `cargo asm my_crate::func`.
- **cargo-expand** — macro expansion output. Essential for debugging derive/proc macros. `cargo expand [path]`.
- **cargo-pgo** — one-command PGO + BOLT workflow. `cargo pgo build` → `cargo pgo optimize`.
- **samply** / **tracing-flame** — profilers with Rust integration.

### Workflow

- **cargo-watch** — re-run commands on file change. `cargo watch -x 'check --all-targets' -x test`.
- **cargo-make** — task runner (Makefile-ish) — `Makefile.toml`.
- **cargo-release** / **release-plz** / **cargo-smart-release** — automated versioning + publishing + changelog + tagging.
- **cargo-msrv** — find minimum supported Rust version. `cargo msrv find` via bisection.

### Code gen / extension

- **cargo-generate** — project template instantiation.
- **cargo-binstall** — install prebuilt binaries when available (much faster than `cargo install`).
- **cargo-nextest archive** — prebuild once, run elsewhere (great for containerized CI splits).

### Named in official Cargo / rustc documentation (use these rows for “source-backed” tool tips)


| Tool / topic                        | Where it appears                                                                                                                                                         |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **cargo vendor**                    | Cargo Book — [Source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html) (directory sources; `cargo vendor` manages unpacked vendored trees) |
| **cargo-local-registry**            | Cargo Book — same chapter (creates local-registry subsets for vendoring workflows)                                                                                       |
| **cargo-hack**                      | Cargo Book — [Rust version / MSRV](https://doc.rust-lang.org/cargo/reference/rust-version.html) (“verify packages under multiple Rust versions”)                         |
| **cargo-msrv**                      | Cargo Book — same MSRV chapter (find minimum toolchain)                                                                                                                  |
| **cargo-clone-crate**               | Cargo Book — [Features](https://doc.rust-lang.org/cargo/reference/features.html) (discovering upstream `Cargo.toml`)                                                     |
| **build-rs**, **jobserver** crates  | Cargo Book — [Build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html) (typed cfg reads; jobserver for parallel C builds)                            |
| **cc** crate                        | Cargo Book — build script examples (compile C from `build.rs`)                                                                                                           |
| **sccache**                         | Cargo Book — [Environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html) (`RUSTC_WRAPPER` example)                                     |
| **cargo-pgo**                       | **rustc Book** — [Profile-guided optimization](https://doc.rust-lang.org/rustc/profile-guided-optimization.html) (“Community Maintained Tools”)                          |
| **rustfilt**                        | **rustc Book** — [instrument-coverage](https://doc.rust-lang.org/rustc/instrument-coverage.html) (demangling for `llvm-cov`)                                             |
| **cargo_metadata** (crate)          | **rustc Book** — [JSON output](https://doc.rust-lang.org/rustc/json.html) (parse `rustc --error-format=json`)                                                            |
| **fwdansi**, **strip-ansi-escapes** | **rustc Book** — [command-line arguments](https://doc.rust-lang.org/rustc/command-line-arguments.html) (`--json=diagnostic-rendered-ansi` + Windows console)             |


**Not** named in those books (widely used anyway): **cargo-nextest**, **cargo-expand**, **cargo-audit**, **cargo-deny**, **cargo-machete**, **cargo-mutants**, **cargo-tarpaulin**, **cargo-flamegraph**, **cargo-bloat**, **cargo-outdated**, **cargo-udeps**, **cargo-llvm-cov**, **cargo-binstall**, **cargo-watch**, etc.

---

## 12-MODERN RUST (Cargo/rustc features)

### Editions

`[package] edition = "2015" | "2018" | "2021" | "2024"`. Default when omitted = 2015 (legacy; always declare one).

Per-target edition allowed:

```toml
[[bin]]
name = "legacy-tool"
edition = "2018"
```

Migrate with `cargo fix --edition`, then manually bump `edition` in manifest.

### Resolver "1" / "2" / "3"

Declared at root of workspace/package:

```toml
[package]
resolver = "3"     # or at [workspace.resolver]
```


| Version | Implicit default                | Key difference                                                                                                                                                         |
| ------- | ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1       | `edition < "2021"`              | Unifies features across all build kinds                                                                                                                                |
| 2       | `edition = "2021"`              | Splits features: target-specific deps not unified for non-matching target; build/proc-macro not unified with normal; dev-deps only unified when building their targets |
| 3       | `edition = "2024"` (Rust 1.84+) | Same graph as v2 + MSRV-aware resolution (`resolver.incompatible-rust-versions = "fallback"` by default)                                                               |


Virtual workspaces must set `resolver` explicitly (no `[package].edition` to infer from).

Resolver v2 also enables workspace-wide `--no-default-features` propagation and `-p` multi-package `--features` (`-p a -p b --features foo-feat,bar-feat`).

### MSRV declaration

```toml
[package]
rust-version = "1.75"   # bare x.y or x.y.z; no semver ops
```

- Cargo refuses to build on older toolchains with a clear error. Override via `cargo build --ignore-rust-version`.
- With resolver v3 (or `resolver.incompatible-rust-versions = "fallback"`), `cargo update`/`cargo add` prefer dep versions compatible with your MSRV.
- Workspace inheritance: `rust-version.workspace = true`.
- `cargo add <crate>` picks the latest version whose MSRV ≤ yours automatically.
- Raising `rust-version` is a minor-compatible change per SemVer conventions but considered a "soft-breaking" policy decision.

### MSRV-aware resolution config

```toml
# .cargo/config.toml
[resolver]
incompatible-rust-versions = "fallback"   # or "allow"
```

- `allow` (default for resolver v1/v2): picks highest version ignoring MSRV.
- `fallback` (default for resolver v3): prefers highest MSRV-compatible version, falls back to highest if none compatible.

### Workspace inheritance — the full sweep

Root:

```toml
[workspace]
members  = ["crates/*"]
resolver = "2"

[workspace.package]
version      = "0.5.0"
edition      = "2021"
rust-version = "1.75"
license      = "MIT OR Apache-2.0"
repository   = "https://github.com/me/proj"
authors      = ["Me <me@x.com>"]

[workspace.dependencies]
tokio  = { version = "1", features = ["rt-multi-thread"] }
serde  = { version = "1", features = ["derive"] }

[workspace.lints.rust]
unsafe_code    = "forbid"
missing_docs   = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }

[workspace.metadata.ci]    # opaque to Cargo; reserved for tools
retry = 2
```

Member:

```toml
[package]
name         = "cli"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true

[dependencies]
tokio = { workspace = true, features = ["net"] }     # additive
serde.workspace = true

[lints]
workspace = true
```

Minimum Cargo: package inheritance 1.64, lints inheritance 1.74.

### Dependencies — version requirement semantics

Caret (default): `"1.2.3"` ≡ `"^1.2.3"` ≡ `>=1.2.3, <2.0.0`. For `0.x.y` the leftmost non-zero component is the compat axis:

- `"0.2.3"` = `>=0.2.3, <0.3.0`
- `"0.0.3"` = `>=0.0.3, <0.0.4`

Tilde: `"~1.2.3"` = `>=1.2.3, <1.3.0`. Wildcard: `"1.2.*"` = `>=1.2.0, <1.3.0`. Exact: `"=1.2.3"`. Compound: `">=1.2, <1.5"`.

Pre-release exclusions: a request like `"1.0"` won't match `1.0.0-alpha.1`; to match you must request a pre-release explicitly: `"1.0.0-alpha"`.

Git + version fallback (publishable):

```toml
bitflags = { git = "https://github.com/bitflags/bitflags", version = "2" }
# Uses git locally; crates.io when published (local git ignored after publish).
```

Renaming for multiple versions:

```toml
[dependencies]
serde1 = { version = "1", package = "serde" }
serde2 = { version = "2", package = "serde" }
# Use as `extern crate serde1;` etc.
```

Features must reference the LOCAL name: `["serde1/derive"]`, not `["serde/derive"]`.

### [patch] — override without publishing

**Workspace root only** — `[patch]` in dependency manifests is ignored.

```toml
[patch.crates-io]
serde = { git = "https://github.com/me/serde", branch = "fix-bug" }
uuid  = { path = "../uuid-local" }

[patch."https://github.com/upstream/foo"]
foo = { git = "https://github.com/me/foo", branch = "patched" }
```

- The version in `[dependencies]` must still be satisfied by the patched source; you can depend on a version that exists only in git until it is published — then drop `[patch]`.
- `**[patch]` applies transitively** in the graph but is only declared at the **workspace root**; downstream users of your library may need to **repeat** their own `[patch]` ([Overriding dependencies](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)).
- Semver-incompatible requirements can yield **two** versions of the same crate name in one graph (e.g. app on `uuid 1.x`, lib on `uuid 2.0` via patch) — useful for staged migrations.
- **Patch in config / CLI** (local, uncommitted): `[patch]` is valid in `.cargo/config.toml` or `cargo --config 'patch.crates-io...'`.

Multiple lines for the same package name via `package` key:

```toml
[patch.crates-io]
serde = { git = "https://github.com/serde-rs/serde.git" }
serde2 = { git = "https://github.com/example/serde.git", package = "serde", branch = "v2" }
```

### `[replace]` (deprecated)

Table `[replace]` with [Package ID Spec](https://doc.rust-lang.org/cargo/reference/pkgid-spec.html) keys — **deprecated**; prefer `[patch]`.

### `paths` in `.cargo/config.toml` (limited override)

Top-level `paths = ["/path/to/crate"]` overrides a **published** crate checkout without editing `Cargo.toml`. **Cannot** change the dependency graph shape (e.g. add a new dependency to a crate solely via `paths`). For that, use `[patch]`.

### Build cache — `target/` vs build-dir ([ref](https://doc.rust-lang.org/cargo/reference/build-cache.html))

- **Locations:** `CARGO_TARGET_DIR`, `build.target-dir`, `--target-dir` → final artifacts (`bin`, `rlib`, `cargo doc` → `target/doc/`, `cargo package` → `target/package/`). `**build.build-dir`** / `CARGO_BUILD_BUILD_DIR` → intermediate/compiler-internal layout (opaque; may change).
- **Profiles:** `target/debug/` = `dev` + `test`; `target/release/` = `release` + `bench`; custom profile → `target/<name>/`. Cross-compile: `target/<triple>/debug|release|…/`.
- **No `--target`:** host build **shares** artifacts with build scripts + proc-macros; `**RUSTFLAGS` applies to all** `rustc` invocations. `**--target <triple>`:** proc-macros and `build.rs` run for **host** separately → **no shared RUSTFLAGS** with target crate (avoids sanitizers/PGO polluting host tools).
- Under each profile dir: `deps/` (rlibs etc.), `incremental/`, `build/<pkg>/` (build script outputs). Adjacent `***.d`** dep-info (Makefile syntax); `build.dep-info-basedir` → relative paths for external build systems.
- **sccache:** `RUSTFLAGS` unchanged; set `RUSTC_WRAPPER=sccache` or `build.rustc-wrapper` to share compiled crates across workspaces.

### Source replacement / vendoring

```toml
# .cargo/config.toml
[source.crates-io]
replace-with = "vendored"

[source.vendored]
directory = "vendor"
```

Populate with `cargo vendor > .cargo/config.toml.fragment`. Replacement source MUST contain identical sources (same checksums).

Alternative replacements:

- `directory = "…"` (vendor tree)
- `local-registry = "…"` (cargo-local-registry)
- `registry = "https://…"` or `registry = "sparse+https://…"`
- `git = "…"` + branch/tag/rev

Offline-capable chain: set up vendor dir → `--offline` or `[net] offline = true`.

### --locked, --frozen, --offline

- `--locked` — error if Cargo.lock would be modified; don't modify it.
- `--frozen` = `--locked` + `--offline`.
- `--offline` — don't touch network; use cached metadata.

Use `--locked` in CI for publish/release to guarantee reproducibility.

### [lints] as a first-class concept (1.74+)

Covered above. Crucially, `[workspace.lints]` + `[lints] workspace = true` in every member enables consistent lint policy. Priority ordering matters: `priority = -1` on a group, `priority = 0` on a specific lint → group enables first, then specific override wins.

### Artifact dependencies (`-Z bindeps`, unstable)

```toml
[build-dependencies]
tool = { version = "1", artifact = "bin" }

[dependencies]
fancy = { version = "1", artifact = "staticlib" }
```

Build.rs reads `CARGO_BIN_FILE_TOOL` (path). Crate reads `env!("CARGO_STATICLIB_FILE_FANCY")`.

### Cargo scripts (`-Z script`, unstable)

Single-file executable with embedded manifest:

```rust
#!/usr/bin/env -S cargo +nightly -Zscript
---cargo
[dependencies]
clap = { version = "4", features = ["derive"] }
---

use clap::Parser;
#[derive(Parser)] struct Args { #[clap(short)] name: String }
fn main() { let a = Args::parse(); println!("hi {}", a.name); }
```

### Key unstable -Z flags worth knowing

- `-Zbuild-std[=core,alloc,std]` — rebuild std; required for sanitizers, custom targets, PGO on std.
- `-Zbuild-std-features=backtrace,panic-unwind` — tune std features.
- `-Zminimal-versions`, `-Zdirect-minimal-versions` — pick *lowest* semver-compatible versions; verify your MSRVs declared in manifests aren't lying.
- `-Zpublic-dependency` — mark deps `public = true` to trigger `exported_private_dependencies` lint. Helps SemVer hygiene.
- `-Zpanic-abort-tests` — allows `panic=abort` test binaries.
- `-Zprofile-hint-mostly-unused` — hints dependencies are mostly unused for smaller codegen.
- `-Zcodegen-backend` — select cranelift/other codegen backend for a profile:
  ```toml
  [profile.dev.package.my-crate]
  codegen-backend = "cranelift"
  ```
- `-Zmtime-on-use` — updates mtimes for cache-cleanup tools.
- `-Zno-index-update` — skip registry index pull.
- `-Zunstable-options` — umbrella for various CLI additions (e.g., `--artifact-dir`, `--keep-going` before stable).
- `-Zper-package-target` — `forced-target = "wasm32-unknown-unknown"` in a specific `[package]`.
- `-Zcheck-cfg` — predecessor of now-stable check-cfg (still useful on older versions).
- `-Zsparse-registry` — stabilized in 1.70 as default for crates.io.

Persist in `.cargo/config.toml` without per-invocation flag:

```toml
[unstable]
build-std = ["std"]
mtime-on-use = true
```

### Registries — index, protocol, auth ([ref](https://doc.rust-lang.org/cargo/reference/registries.html))

- **Alternate index:** `[registries.NAME] index = "https://…"` (git repo) or `"sparse+https://…"`. Dep: `crate = { version = "1", registry = "NAME" }`. Env: `CARGO_REGISTRIES_<NAME>_INDEX`. **crates.io does not publish crates that depend on other registries.**
- **Protocol:** `sparse+` → HTTP per-crate metadata (default for crates.io since 1.70 via `[registries.crates-io] protocol = "sparse"`); plain `https` git URL → clone whole index (slow cold fetch). Mirroring/vendoring → [source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html).
- **Publish:** `cargo login --registry=NAME`; token `CARGO_REGISTRIES_<NAME>_TOKEN` or `--token`; `[registry] default = "NAME"`; `[package] publish = ["NAME"]` | `false` to gate. Tokens in `~/.cargo/credentials.toml` per registry.
- **Credentials (1.74+):** `[registry] global-credential-providers = ["cargo:token", "cargo:macos-keychain", …]`; per-registry `credential-provider`. Default `cargo:token` = plaintext file — prefer OS keychain on workstations.

### SemVer — API / Cargo quick map ([Cargo semver](https://doc.rust-lang.org/cargo/reference/semver.html))

- **Major:** remove/rename/move public API; add `cfg` that hides public items; trait breaking changes; new required type params; tighter bounds; struct field / enum variant without `#[non_exhaustive]`; repr/linkage changes; `no_std`→`std` requirement; remove feature / optional dep from default or drop implicit feature; add `#[non_exhaustive]` on all-public ADT (breaks literals).
- **Minor:** new items; optional new fields if private field existed; defaulted generics/traits; new Cargo feature / optional dep; loosen bounds; `unsafe fn`→safe; `#[deprecated]`.
- **Risk:** new inherent methods vs trait methods; MSRV bump; env/platform assumptions.

### Publishing — crates.io workflow

Required manifest fields: `name`, `version`, `description`, `license` (SPDX) or `license-file`, plus one of `homepage`/`repository`/`documentation`. `readme` auto-detected (`README.md` default).

```bash
cargo login                       # paste token from https://crates.io/me
cargo package --list              # see what's shipped
cargo publish --dry-run           # local verification
cargo publish                     # real upload
cargo publish --registry mycorp   # alt registry
cargo yank --version 1.2.3
cargo yank --version 1.2.3 --undo

cargo owner --add github:org:team-name
cargo owner --remove user
```

Package = `.crate` tarball ≤ 10 MB (crates.io limit). Contents determined by `include`/`exclude` patterns (gitignore syntax).

```toml
[package]
exclude = ["/.github", "/tests/fixtures/*.bin"]
include = ["/src/**/*.rs", "README.md", "LICENSE-*"]
```

Publish gating:

```toml
[package]
publish = ["mycorp"]    # only this registry
# publish = false        # never publish
```

### cargo add — quick reference

```bash
cargo add tokio --features rt-multi-thread,macros
cargo add --dev trybuild
cargo add --build cc
cargo add winapi --target 'cfg(windows)'
cargo add serde --no-default-features --features derive
cargo add my-crate@=1.2.3
cargo add --path ./local-crate
cargo add --git https://github.com/me/repo --branch main
cargo add my-crate --rename my-alias
cargo add my-crate -p member-crate           # in a workspace
cargo add my-crate --dry-run
```

`cargo add` honors `rust-version`: picks highest version whose MSRV is ≤ yours.

### cargo tree — investigative queries

```bash
cargo tree                                    # default
cargo tree --duplicates                       # -d: multi-version packages
cargo tree -e features                        # show enabled features per edge
cargo tree -e features -i tokio               # invert: who enables tokio features
cargo tree --target all --edges features --invert syn    # everything that pulled syn
cargo tree --prefix none --format "{p} {f}"   # one package per line with features
cargo tree --depth 1                          # direct deps only
cargo tree --prune rand                       # hide a subtree
cargo tree --workspace --target all --all-features
```

### cargo install — installing binaries

```bash
cargo install ripgrep                         # from crates.io, latest
cargo install ripgrep@13.0.0                  # version pin
cargo install --locked ripgrep                # use shipped Cargo.lock (reproducible)
cargo install --git https://github.com/me/tool --rev abc123
cargo install --path .                        # local dev install
cargo install --force ripgrep                 # overwrite existing
cargo install --list                          # inventory
cargo install --root /usr/local ripgrep
cargo install ripgrep --bin rg --features pcre2 --no-default-features
cargo install ripgrep --profile release-lto
```

Binary location: `$CARGO_HOME/bin/` by default. Installed artifacts not tied to any project; they live in a temp target dir unless `CARGO_TARGET_DIR` set.

Always pass `--locked` in CI and release scripts to avoid dependency drift. Faster alternative: `**cargo binstall**` pulls prebuilt binaries when published.

### Environment variables Cargo EXPORTS for crates at compile time

Accessible via `env!("NAME")` at compile time:

- `CARGO_PKG_NAME`, `CARGO_PKG_VERSION`, `CARGO_PKG_VERSION_MAJOR/MINOR/PATCH/PRE`
- `CARGO_PKG_DESCRIPTION`, `CARGO_PKG_AUTHORS` (colon-separated), `CARGO_PKG_HOMEPAGE`, `CARGO_PKG_REPOSITORY`, `CARGO_PKG_LICENSE`, `CARGO_PKG_LICENSE_FILE`, `CARGO_PKG_README`, `CARGO_PKG_RUST_VERSION`
- `CARGO_CRATE_NAME` — `-` → `_`
- `CARGO_BIN_NAME` (when building a bin)
- `CARGO_MANIFEST_DIR` (dir containing Cargo.toml), `CARGO_MANIFEST_PATH`
- `CARGO_TARGET_TMPDIR` (for tests — writable scratch dir)
- `CARGO_BIN_EXE_<name>` (in integration tests: absolute path to built binary — perfect for black-box testing CLIs)

### Environment variables Cargo READS (user-facing)

- `CARGO_HOME`, `CARGO_TARGET_DIR`, `CARGO_INCREMENTAL`, `CARGO_LOG`
- `RUSTFLAGS`, `RUSTDOCFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_ENCODED_RUSTDOCFLAGS`
- `RUSTC`, `RUSTDOC`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER`
- `CARGO_NET_OFFLINE`, `CARGO_NET_RETRY`
- `CARGO_TERM_COLOR`, `CARGO_TERM_VERBOSE`, `CARGO_TERM_QUIET`, `CARGO_TERM_PROGRESS_WHEN`
- `CARGO_BUILD_JOBS`, `CARGO_BUILD_TARGET`, `CARGO_BUILD_RUSTFLAGS`, `CARGO_BUILD_RUSTDOCFLAGS`
- `CARGO_PROFILE_<name>_<key>` e.g. `CARGO_PROFILE_RELEASE_LTO=fat`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`
- `CARGO_TARGET_<TRIPLE>_LINKER`, `CARGO_TARGET_<TRIPLE>_RUNNER`, `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`
- `CARGO_REGISTRY_TOKEN`, `CARGO_REGISTRIES_<NAME>_TOKEN`, `CARGO_REGISTRIES_<NAME>_INDEX`
- `HTTP_PROXY`, `HTTPS_PROXY`, `HTTP_TIMEOUT`

---

## QUICK REFERENCE — one screen

```bash
# CI / gate
cargo fmt --check && cargo clippy --all-targets --all-features --workspace -- -D warnings && cargo test --workspace --all-features
cargo build --release --locked --frozen

# Investigate
cargo tree -e features -i <crate>    cargo tree --duplicates

# Tooling (see §11)
cargo hack check --feature-powerset --no-dev-deps --depth 2
cargo +nightly update -Zminimal-versions && cargo check --all-targets
RUSTFLAGS="-C instrument-coverage" cargo test && cargo llvm-cov report
cargo vendor > .cargo/config.toml.fragment
```

```toml
# rust-toolchain.toml at repo root
[toolchain]
channel = "1.84.0"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
```

```toml
# Fast dev deps + thin debug (§10 profiles)
[profile.dev.package."*"]
opt-level = 2
[profile.dev]
split-debuginfo = "unpacked"
debug = "line-tables-only"
```

---

## CARGO FAQ (DENSE) — [faq](https://doc.rust-lang.org/cargo/faq.html)

- **Lockfile in VCS:** snapshot for reproducible CI / bisect / consistent dep verification; does **not** constrain library consumers — only `Cargo.toml` does. `cargo install` / new `cargo add` can still pull latest unless `--locked`. Lockfiles merge-conflict prone — resolve then `cargo check` or `cargo tree` to regenerate.
- **Offline:** `--offline` / `--frozen`, `cargo fetch` before disconnect; config `net.offline`; vendoring ↔ source replacement.
- **Unexpected rebuilds:** `CARGO_LOG=cargo::core::compiler::fingerprint=info` on the surprising rebuild. Common causes: `rerun-if-changed` pointing at missing file; different **feature sets** workspace vs single-crate build; flaky timestamps (weird FS); concurrent processes touching `target/`.
- **“Version conflict”:** duplicate `**links`** native libs; incompatible version ranges (“all possible versions conflict…”); `direct-minimal-versions` (unstable) vs min versions; missing **features**; lockfile merge mess — fix TOML ranges, then refresh lock.
- **Disk usage:** intentional — artifact cache per (toolchain, features, …), incremental, dev debuginfo. Trim with `cargo clean`, profile tweaks, or understanding [build-cache](https://doc.rust-lang.org/cargo/reference/build-cache.html).

---

## GOTCHAS AND COMMON PITFALLS

- Feature unification can silently enable dependencies you didn't want. Always audit `cargo tree --workspace --target all --all-features` before release.
- `cfg!(target_os = "…")` in `build.rs` reports the HOST, not the target. Use `env::var("CARGO_CFG_TARGET_OS")`.
- `[profile.*]` in a workspace member is IGNORED. Only the root counts.
- `lto = true` + `panic = "abort"` + `codegen-units = 1` is not strictly monotonic — fat LTO can regress on extremely inlineable code. Benchmark both thin and fat.
- Without a `rerun-if-*` directive, build.rs reruns on any source change in the package. Always emit at least one directive to get fine-grained rebuild control.
- Changing `Cargo.lock` with `cargo update` doesn't affect published library users — they resolve fresh. `Cargo.lock` matters for applications/workspaces.
- `panic = "abort"` + `cargo test` on stable is currently incompatible. Tests always build with `panic = "unwind"`. Use `-Z panic-abort-tests` on nightly if needed.
- `cargo install` ignores `Cargo.lock` by default — always pass `--locked` for reproducibility.
- `cargo test` binaries run with CWD = package root, BUT integration tests that `env!("CARGO_MANIFEST_DIR")` use the original manifest path.
- Build scripts and proc-macros are NOT sanitized by default. Always pair `RUSTFLAGS="-Zsanitizer=..."` with `--target` so host tools build normally.
- `Cargo.lock` is checked in for binaries/workspaces only — NOT for library crates (convention). The library's actual deps are re-resolved by each consumer.
- Rust edition ≠ Cargo resolver. You can have `edition = "2024"` with `resolver = "2"` (v3 would be implicit, override-able).
- `RUSTFLAGS` is considered part of the cache key — changing it triggers a full rebuild. Prefer profile settings or per-target config when possible.
- `[lints]` section silently does nothing on Cargo < 1.74.
- `cargo::` (double-colon) build script directives require Cargo 1.77+. Old `cargo:` (single colon) still works.

---

## NOTES ABOUT SOURCES

- **Cargo Book** and **rustc Book** on `doc.rust-lang.org` are the authoritative bases for manifest, resolver, profiles, build scripts, `RUSTFLAGS`, codegen flags, PGO, linker-plugin-LTO, coverage, JSON diagnostics, platform tiers, **build cache layout**, **registries** (index URL, sparse vs git, publishing), **libtest CLI** (rustc Tests chapter), and the **CI guide** / **FAQ** (latest-deps trade-offs, lockfile rationale, rebuild debugging, conflicts, disk).
- **Rust Unstable Book** documents **sanitizers** (`-Zsanitizer=…`) and many **Cargo `-Z`** flags listed in [Cargo unstable features](https://doc.rust-lang.org/cargo/reference/unstable.html) (`build-std`, `minimal-versions`, `panic-abort-tests`, etc.).
- §**11** ecosystem bullets are **expert picks** except rows in the “Named in official Cargo / rustc documentation” table.
- Always confirm flag names with `rustc -C help`, `cargo -Z help`, and your installed toolchain version.

