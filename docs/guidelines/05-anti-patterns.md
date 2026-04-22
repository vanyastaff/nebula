# 5. Anti-Patterns

[← Design Patterns](./04-design-patterns.md) | [README](./README.md) | [Next: Functional Concepts →](./06-functional-concepts.md)

---

## [A-001] `.clone()` to silence the borrow checker

```rust
// Bad
let y = x.clone();
use_y(y);
mutate(&mut x);   // wouldn't compile without clone
```

Diagnosis:
- Need shared access → `Rc`/`Arc`.
- Need enum transition → `mem::take`/`mem::replace` (`[I-006]`).
- Need multiple readers → proper lifetimes.
- Actually want two owners (snapshot, send-through-channel) → `clone` is fine, make it explicit.

`Arc::clone(&x)` (not `x.clone()`) — explicit intent. `String::clone()` for error construction is acceptable (overhead negligible).

Clippy: `redundant_clone`, `clone_on_copy`, `clone_on_ref_ptr`.

## [A-002] `#![deny(warnings)]` in library code

Breaks every consumer's build when a new lint lands in stable. Instead:

```toml
# Cargo.toml workspace-level lints
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
missing_docs = "warn"
unreachable_pub = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery  = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
```

```shell
# CI
RUSTFLAGS="-D warnings" cargo build
cargo clippy --all-targets -- -D warnings
```

## [A-003] `Deref` polymorphism

Using `Deref` to emulate inheritance:

```rust
// Bad
struct Foo; impl Foo { fn m(&self) {} }
struct Bar { f: Foo }
impl Deref for Bar { type Target = Foo; fn deref(&self) -> &Foo { &self.f } }
// bar.m() works — surprising, breaks with name collisions, no subtyping
```

Problems:
- `Bar` does not become a subtype of `Foo`.
- Trait impls on `Foo` don't transfer to `Bar`.
- `self` inside `Foo::m` is `&Foo`, not `&Bar` — semantics differ from real inheritance.
- Name collisions silently change resolution.

Solution:
- Explicit delegation (`impl Bar { fn m(&self) { self.f.m() } }`).
- `delegate` or `ambassador` crate for automation.
- Extract shared behavior into a trait, impl for both.

Reserve `Deref` for actual pointer types: `Box`, `Rc`, `Arc`, `MutexGuard`, `Pin`, custom smart pointers. Heuristic: if `Target` is "what I point to", `Deref` is correct. If `Target` is "parent I inherit from", it's the anti-pattern.

## [A-004] `static mut`

Edition 2024 requires `unsafe` per access. Replace with:

```rust
// Good
static LOGGER: LazyLock<Logger> = LazyLock::new(Logger::new);
static METRICS: OnceLock<Metrics> = OnceLock::new();
static COUNTER: AtomicU64 = AtomicU64::new(0);
```

## [A-005] `Box<dyn Error>` in library return types

Library code MUST return a typed error:

```rust
// Bad (in a library)
pub fn parse(s: &str) -> Result<Config, Box<dyn std::error::Error>> { /* … */ }

// Good
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing field: {0}")]
    MissingField(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
pub fn parse(s: &str) -> Result<Config, ParseError> { /* … */ }
```

`anyhow::Error` / `eyre::Report` are acceptable in binary crates and tests.

## [A-006] `Arc<Mutex<Vec<T>>>` as a state bus

Usually masks missing architecture. Alternatives:
- Channels: `tokio::sync::mpsc` / `crossbeam::channel`.
- Actor with owned state.
- `DashMap`/`flurry` for concurrent maps.
- Sharding: `RwLock<Vec<…>>` split by key hash.

## [A-007] `.unwrap()` in spawned tasks

`tokio::spawn`/`std::thread::spawn` swallow panics. Return errors explicitly:

```rust
// Bad
tokio::spawn(async move { do_work().await.unwrap() });

// Good
let handle = tokio::spawn(async move { do_work().await });
let res = handle.await.expect("task panicked")?;
```

## [A-008] Over-broad `pub`

Default to private / `pub(crate)`. Use `pub` only for items in the crate's public API. Edition 2024 elevates `unreachable_pub`; respect it.

## [A-009] Unnecessary `async` / unnecessary `Box::pin`

Do NOT mark a function `async` that performs no `.await`. Do NOT `Box::pin` a future just because the compiler complains — first try `use<'a, T>` precise capturing (1.82+) or restructure.

```rust
// Bad
async fn parse(s: &str) -> Config { Config::from_str(s).unwrap() }

// Good
fn parse(s: &str) -> Config { Config::from_str(s).unwrap() }
```

## [A-010] String formatting with `+` / `concat!` for runtime values

```rust
// Bad — allocates per `+`
let greeting = "Hello, ".to_string() + name + "!";

// Good
let greeting = format!("Hello, {name}!");
```

## [A-011] `unwrap`/`expect` on user input

```rust
// Bad
let id: u32 = s.parse().unwrap();

// Good
let id: u32 = s.parse().map_err(|_| Error::InvalidId)?;
```

Reserve `unwrap`/`expect` for invariants the surrounding code guarantees (post-validation, unreachable branches in tests).

## [A-012] Reinventing `Drop`-based state machines

If a state transition is "run code on exit from this scope", it's RAII. Don't write manual `close()` call at every return point.

## [A-013] Growing a trait until it is no longer dyn-compatible

If a trait is used as `dyn Trait` and you add a generic method, it becomes unusable as `dyn`. Detect this before committing: either add `where Self: Sized` to the new method, or factor the dyn-compatible subset into a supertrait.

## [A-014] `#[allow(…)]` without justification

Every `#[allow(…)]` annotation MUST have an adjacent comment explaining why. `#![allow(…)]` crate-wide must be avoided in library code.

---

[← Design Patterns](./04-design-patterns.md) | [README](./README.md) | [Next: Functional Concepts →](./06-functional-concepts.md)
