# 7. Modern Rust (1.95+ / Edition 2024)

[← Functional Concepts](./06-functional-concepts.md) | [README](./README.md) | [Next: Review Checklist →](./08-review-checklist.md)

---

## [R-001] `if let` guards (Rust 1.95)

```rust
match tok {
    Token::Num(n) if let Some(v) = n.checked_mul(2) => use_double(v),
    Token::Num(n) => use_raw(n),
    _ => unreachable!(),
}
```

Remember `[L-PAT-002]`: not exhaustive; always add a wildcard.

## [R-002] `cfg_select!` macro (Rust 1.95)

Replaces `cfg-if` crate:

```rust
cfg_select! {
    unix => { fn clock() -> u64 { unix_clock() } }
    target_pointer_width = "32" => { fn clock() -> u64 { safe_32() } }
    _ => { fn clock() -> u64 { fallback() } }
}
```

Prefer `cfg_select!` in new code.

## [R-003] `core::range::Range` and `RangeInclusive` (Rust 1.95)

New types in `core::range` separate range values from iterators. For new APIs accepting ranges as data (not iteration), prefer `core::range::Range<T>` / `RangeInclusive<T>` over `ops::Range<T>`.

## [R-004] Atomic `update` / `try_update` (Rust 1.95)

```rust
let counter = AtomicUsize::new(0);
counter.update(Ordering::Relaxed, Ordering::Relaxed, |v| v + 1);

counter.try_update(Ordering::Acquire, Ordering::Release, |v| {
    if v < 10 { Some(v + 1) } else { None }
}).ok();
```

Prefer over hand-written `compare_exchange_weak` loops and `fetch_update`.

## [R-005] Let chains (Rust 1.88)

```rust
if let Some(x) = opt && x.valid() && let Ok(y) = parse(&x) {
    use_y(y);
}
```

Reduces nesting; use where it clearly wins over `match`.

## [R-006] Async closures + `AsyncFn*` (Rust 1.85)

```rust
async fn each<F>(items: &[Item], f: F)
where F: AsyncFn(&Item) -> Result<(), E>,
{
    for i in items { f(i).await?; }
}

each(items, async |i| { store(i).await; Ok(()) }).await;
```

Async closures can borrow across `.await`, solving the HRTB problem with `impl Fn(&T) -> impl Future<Output=_>`.

## [R-007] `async fn` in traits (Rust 1.75)

Stable. No `async-trait` crate needed for static dispatch. For dyn compatibility, see `[L-DYN-003]`.

## [R-008] `&raw const` / `&raw mut` (Rust 1.82)

See `[L-UNSAFE-007]`. Replaces `addr_of!`/`addr_of_mut!`.

## [R-009] `LazyLock` / `OnceLock` in std (Rust 1.80)

See `[A-004]`. Use instead of `lazy_static!`, `once_cell::sync`, or `static mut`.

## [R-010] `MaybeUninit` slice methods (Rust 1.93+)

- `assume_init_drop`: drop elements of `[MaybeUninit<T>]` in place.
- `assume_init_ref`/`assume_init_mut`: reinterpret as `&[T]`/`&mut [T]`.
- `write_copy_of_slice`: initialize from a source slice.

Reduces boilerplate and centralizes SAFETY comments.

## [R-011] RPITIT (return position `impl Trait` in trait, Rust 1.75+)

```rust
trait Parser {
    fn tokens(&self) -> impl Iterator<Item = Token>;
}
```

Opaque; each impl picks a concrete iterator. Not dyn-compatible (`[L-DYN-001]`).

## [R-012] Precise capturing (`use<>`, Rust 1.82+)

```rust
fn scan<'a>(s: &'a str) -> impl Iterator<Item = &'a str> + use<'a> { /* … */ }
```

Limit what lifetimes/generic parameters an opaque type captures. Default captures everything in-scope, which often blocks valid reborrow chains.

## [R-013] Tooling baseline

- `cargo nextest` for CI (parallelism, retries, JUnit).
- `cargo hakari` for workspace feature unification.
- `cargo deny` for license/security audit.
- `cargo insta` for snapshot tests.
- `cargo mutants` for mutation testing of critical code.
- `miri` for ALL unsafe code.
- `loom` for lock-free synchronization.

---

[← Functional Concepts](./06-functional-concepts.md) | [README](./README.md) | [Next: Review Checklist →](./08-review-checklist.md)
