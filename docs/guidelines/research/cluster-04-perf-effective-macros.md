# Cluster 04 — Perf Book / Effective Rust / Little Book of Rust Macros

> Dense expert notes distilled from:
>
> 1. **The Rust Performance Book** — Nicholas Nethercote (`nnethercote.github.io/perf-book`)
> 2. **Effective Rust** — David Drysdale (`lurklurk.org/effective-rust`)
> 3. **The Little Book of Rust Macros** — Lukas Wirth et al. (`veykril.github.io/tlborm`)
>
> NOTE: This file is consolidated from full-book chapter/item coverage of all three sources in this session (including print/full views where available), then normalized into a single taxonomy for LLM retrieval. Quantitative claims are only included when source-backed; otherwise wording is qualitative.

### Coverage Matrix (All Three Books)

- 01-meta-principles / 02-language-rules / 03-idioms / 04-design-patterns / 05-anti-patterns: primarily **Effective Rust**, with **Perf Book** and **TLBORM** overlays where relevant.
- 06-error-handling / 07-async-concurrency / 10-testing-and-tooling / 12-modern-rust: **Effective Rust** core, cross-linked with **Perf Book** tooling/perf workflow and macro/tooling implications.
- 09-performance: primary **Perf Book** coverage, connected to **Effective Rust** API design constraints and macro cost trade-offs.
- Macros sections and appendices: primary **TLBORM** coverage, with **Effective Rust** macro policy and **Perf Book** compile-time/perf constraints.

### Canonical Effective Rust item list (David Drysdale — use this for retrieval)

Sections below interleave perf/macro material with ER advice; **some early `### ER Item N` headings predate a full pass and may not match this table**. Prefer this index when citing “Item N”.


| #   | Title                                                          |
| --- | -------------------------------------------------------------- |
| 1   | Use the type system to express your data structures            |
| 2   | Use the type system to express common behavior                 |
| 3   | Prefer `Option` and `Result` transforms over explicit `match`  |
| 4   | Prefer idiomatic error types                                   |
| 5   | Understand type conversions                                    |
| 6   | Embrace the newtype pattern                                    |
| 7   | Use builders for complex types                                 |
| 8   | Familiarize yourself with reference and pointer types          |
| 9   | Consider using iterator transforms instead of explicit loops   |
| 10  | Familiarize yourself with standard traits                      |
| 11  | Implement the `Drop` trait for RAII patterns                   |
| 12  | Understand the trade-offs between generics and trait objects   |
| 13  | Use default implementations to minimize required trait methods |
| 14  | Understand lifetimes                                           |
| 15  | Understand the borrow checker                                  |
| 16  | Avoid writing `unsafe` code                                    |
| 17  | Be wary of shared-state parallelism                            |
| 18  | Don’t panic                                                    |
| 19  | Avoid reflection                                               |
| 20  | Avoid the temptation to over-optimize                          |
| 21  | Understand what semantic versioning promises                   |
| 22  | Minimize visibility                                            |
| 23  | Avoid wildcard imports                                         |
| 24  | Re-export dependencies whose types appear in your API          |
| 25  | Manage your dependency graph                                   |
| 26  | Be wary of feature creep                                       |
| 27  | Document public interfaces                                     |
| 28  | Use macros judiciously                                         |
| 29  | Listen to Clippy                                               |
| 30  | Write more than unit tests                                     |
| 31  | Take advantage of the tooling ecosystem                        |
| 32  | Set up a continuous integration (CI) system                    |
| 33  | Consider making library code `no_std` compatible               |
| 34  | Control what crosses FFI boundaries                            |
| 35  | Prefer `bindgen` to manual FFI mappings                        |


---

## 01-meta-principles — Design philosophy from Effective Rust

### ER Item 1 — Use the type system to express your data structures

- Rust's type system is the single biggest differentiator; use it to shape the shape of data before shaping behaviour. Sum types (enums with payloads) let illegal states be unrepresentable — prefer them to paired booleans or `Option<T>` fields that are "only valid together".
- Rule of thumb: if two fields have a "one-of" or "if-A-then-B" relationship, collapse them into a single enum. Example antipattern: `struct Job { is_running: bool, started_at: Option<Instant>, result: Option<Outcome> }`. Idiomatic: `enum Job { Pending, Running { started_at: Instant }, Done(Outcome) }`.
- Prefer fine-grained newtypes over raw primitives: `struct UserId(u64);` prevents swapping `UserId` with `OrderId` at call sites. Cost = zero at runtime (transparent repr).

### ER Item 2 — Use the type system to express common behaviour (traits)

- Traits are Rust's interface mechanism: a set of method signatures an implementor must fulfil. Separate behaviour from data; many small traits > one big trait (Interface Segregation).
- Marker traits (no methods) encode capabilities: `Send`, `Sync`, `Copy`, `Unpin`, `UnwindSafe`.
- Blanket impls: `impl<T: Display> ToString for T { ... }`. Use sparingly; they can conflict with downstream impls.
- Supertraits: `trait Animal: Debug + Send { ... }` — consumers get `Debug` "for free".

### ER Item 3 — Prefer `Option` and `Result` transforms to explicit match

- Chain `map`, `and_then`, `or_else`, `ok_or_else`, `unwrap_or_else`, `?`. A long `match` on `Option` / `Result` usually means an adapter exists.
- `Option::as_ref` / `as_deref` / `as_mut` convert between `Option<T>` and `Option<&T>` / `Option<&str>` without moving.
- `?` inside functions returning `Option` or `Result` — since 1.22 for `Result`, 1.22+/`Try` trait stabilised for broader use.

### ER Item 4 — Prefer idiomatic Error types

- Library: enum per module with `thiserror::Error`; variants encode category; `#[from]` for nested conversions; preserve source chain via `#[source]` / `#[from]`.
- Application: `anyhow::Error` (or `eyre`) with `.context(...)` / `.with_context(|| ...)` to attach semantic context lazily.
- Avoid `Box<dyn Error + Send + Sync + 'static>` as a public API — erases variant info and hurts matching.

### ER Item 5 — Understand lifetimes

- A lifetime is a scope during which a reference remains valid; it is purely static, never observable at runtime.
- Three elision rules (from the Reference): (1) each elided input lifetime becomes its own parameter; (2) if exactly one input lifetime, it is assigned to all elided output lifetimes; (3) if `&self` or `&mut self` is present, its lifetime is assigned to elided output lifetimes.
- `'static` ≠ "lives for program duration at runtime" — it means the reference is allowed to live that long (same with owned `T: 'static`). Owned `String` is `'static` because it borrows from nothing.
- Non-Lexical Lifetimes (NLL) and two-phase borrows have made common patterns ergonomic (mutating a `Vec` during iteration by index, etc.).

### ER Item 6 — Understand type conversions

- Prefer `From` / `Into` for total conversions; `TryFrom` / `TryInto` for fallible ones. Implement only `From`; `Into` comes free via blanket impl.
- `AsRef<T>` / `AsMut<T>`: cheap, non-allocating reference conversions (accept `impl AsRef<Path>` in APIs).
- `Borrow<T>` is stricter than `AsRef<T>`: requires hashing/ordering to agree between borrowed and owned types (`HashMap::get` uses it).
- `Cow<'a, B>` where `B: ToOwned`: clone-on-write; returns `Borrowed` in fast path, `Owned` only when mutation needed.
- Numeric casts: `as` is sharp-edged (truncation on narrowing, NaN on float→int saturates to 0 since 1.45). Prefer `TryFrom` for integer narrowing: `u32::try_from(big_i64)?`.

### ER Item 7 — Use builders for complex types

- Rust lacks default/named arguments; builders fill that niche. Three flavours:
  1. Consuming: `self -> Self` per setter; ends in `.build()`.
  2. Mutable reference: `&mut self -> &mut Self`; ends in `.build()` taking `&self`. Works across method chains without moves.
  3. Typestate builder: `Builder<NoUrl>` → `Builder<WithUrl>` enforces required fields at compile time.
- `derive_builder` / `bon` crates automate builders.

### ER Item 8 — Familiarise yourself with reference and pointer types

- `&T` / `&mut T` — borrowed references, zero-cost, compile-checked.
- `Box<T>` — owned heap allocation; unique.
- `Rc<T>` / `Arc<T>` — shared ownership; non-atomic / atomic.
- `Cell<T>` / `RefCell<T>` — interior mutability (single-thread).
- `Mutex<T>` / `RwLock<T>` — interior mutability (threads).
- `*const T` / `*mut T` — raw pointers; require `unsafe` to dereference.
- `NonNull<T>` — non-null raw pointer, covariant, used in unsafe collections.
- `Pin<P>` — guarantees pointee doesn't move; foundational for async.

### ER Item 9 — Consider using iterator transforms instead of explicit loops

- Iterators compose cleanly, express intent, and optimise to the same or better code than hand-written loops because LLVM can prove bounds away.
- `for x in &v` > `for i in 0..v.len() { let x = &v[i]; }` — avoids bounds checks.
- Favour `iter().filter_map()` over `iter().filter(...).map(...)` when combining.
- `fold` is general; `try_fold` short-circuits on `Err`/`None`.
- For parallelism: swap `iter()` for `par_iter()` via `rayon` — API-compatible.

### ER Item 10 — Implement the Drop trait for RAII patterns

- `Drop::drop(&mut self)` runs at end of lexical scope or when the owning binding is moved out/replaced. Use it for file handles, sockets, DB transactions.
- Scope guards (crate `scopeguard::defer! { ... }`) for ad-hoc cleanup without defining a type.
- Pitfall: `Drop::drop` cannot take ownership of fields; to "consume" inside drop use `ManuallyDrop` + `ptr::read` in unsafe, or restructure via `Option<T>::take()`.

---

## 02-language-rules — Lifetime / trait rules from Effective Rust

### ER Item 11 — Implement the `Default` trait

- Required for auto-derived `Default` on structs; expected by many APIs (`HashMap::with_capacity_and_hasher`, etc.).
- Derive when field types already implement `Default`; impl manually when defaults differ from type defaults.

### ER Item 12 — Understand the uses of `impl Trait`

- Argument position: `fn f(x: impl Read)` = anonymous generic parameter; use when type doesn't appear elsewhere.
- Return position (RPIT): `fn make() -> impl Iterator<Item=u32>` = one specific opaque type; two return arms must have same concrete type.
- RPITIT (Return-position `impl Trait` in traits, 1.75+): `trait Foo { fn iter(&self) -> impl Iterator<Item=i32>; }`. Implementations must match by trait signature.
- Lifetime capture: pre-2024, RPIT captured only the explicitly named lifetimes; 2024 edition captures every in-scope lifetime. Use `use<'a, T>` syntax (stable 1.82) to restrict capture explicitly.

### ER Item 13 — Use generics and trait objects to express polymorphism

- **Generics**: monomorphised, zero dispatch cost, code bloat risk. Stable signatures easier to maintain.
- **Trait objects** (`dyn Trait`): one code path, `Box<dyn Trait>` / `&dyn Trait` / `Arc<dyn Trait>`, dynamic dispatch, heterogeneous containers.
- Object safety rules: no generic methods, `Self: Sized` excluded methods, no associated constants used through trait object, etc. Relaxed over time: `Self` in return types sometimes allowed via where clauses.
- Hybrid: generic types that erase at API boundary using `Box<dyn Trait>` internally (type-erasure pattern).

### ER Item 14 — Use built-in traits to tag types

- `Copy`: bitwise-copyable; no `Drop`. Implies `Clone`.
- `Send` / `Sync`: auto-traits. `Send` = ownership can cross thread; `Sync` = shared ref can. Opt-out with `impl !Send for MyType {}` (nightly) or `PhantomData<*const ()>`.
- `Sized`: default bound on generics. Use `?Sized` when you want `str` / `dyn Trait` / slice at the tail.
- `Unpin`: promises that moving a pinned value is safe. Pointer types are `Unpin`; `PhantomPinned` opts out.

### ER Item 15 — Understand the trait system's coherence rules

- Orphan rule: at least one of `Trait` or `Type` must be defined in your crate. Bypass via newtype wrapping upstream types.
- Fundamental types (`&T`, `&mut T`, `Box<T>`, `Pin<P>`) don't count as yours; `impl Foo for Box<UpstreamType>` still violates the orphan rule unless `Foo` is yours.

### ER Item 16 — Avoid writing `unsafe` code

- Unsafe unlocks five superpowers: dereference raw pointer, call unsafe fn, access mutable static, implement unsafe trait, access union field. It does *not* turn off the borrow checker.
- Encapsulate unsafe behind safe APIs; document invariants in a `# Safety` doc section.

### ER Item 17 — Be wary of shared-state parallelism

- Don't reach for `Arc<Mutex<T>>` first. Consider message passing (`std::sync::mpsc`, `crossbeam-channel`, `tokio::sync::mpsc`), per-thread sharding, or immutable data (`Arc<T>` without mutex).
- Lock ordering issues → deadlocks. Establish a total order or use `parking_lot::Mutex` with timeouts.

### ER Item 18 — Don't panic

- Library APIs: return `Result` / `Option`. Don't panic on user input.
- `#![deny(clippy::unwrap_used)]` / `clippy::expect_used` / `panic` lints enforce discipline.
- Prefer `expect("invariant: ...")` over `unwrap()` — the message documents the invariant; grep-friendly.
- `.unwrap_or_default()` / `.unwrap_or_else(|_| fallback())` for graceful degradation.

### ER Item 19 — Avoid reflection

- Rust has essentially no reflection. `Any` gives runtime type identity but not field enumeration.
- Use traits + derives for metaprogramming (`serde`, `schemars`). Don't simulate reflection with stringly-typed dispatch.

### ER Item 20 — Avoid the temptation to over-optimise

- Measure first. Rust's default zero-cost abstractions are already fast.
- Premature `unsafe` for speed almost always loses to idiomatic safe code after LLVM's optimisations.

---

## 03-idioms — Preferred patterns from all three books

### Iterators and combinators (perf-book + ER Item 9)

- `.iter()` on slice / array → `&T`; `.iter_mut()` → `&mut T`; `into_iter()` → by-value.
- Since 1.53, arrays have `IntoIterator` by value; pre-1.53 `arr.iter()` was required.
- Chain specialisation: `.chain(b)` is fast if both are `ExactSizeIterator`.
- `collect::<Result<Vec<_>, _>>()` short-circuits on first `Err`.
- `FromIterator` for `HashMap` / `BTreeMap`: `iter.map(|x| (k(x), v(x))).collect()`.
- Avoid double iteration: prefer `fold` over `count() + sum()`. Or `tee` via manual caching.

```rust
// Idiomatic: short-circuit on first error
let parsed: Result<Vec<u32>, _> = strings.iter().map(|s| s.parse()).collect();

// Chunking for batch work
for batch in items.chunks(CHUNK_SIZE) { process(batch); }

// Windowed without alloc
for w in series.windows(3) { let [a, b, c] = w.try_into().unwrap(); }
```

### Strings (perf-book "Strings" chapter)

- `String` = growable heap UTF-8 buffer (Vec + invariants). `&str` = borrowed view. `Box<str>` = owned, non-growable — smaller than `String` (no cap) when size matters.
- Prefer `&str` in function args; `&[u8]` for raw bytes.
- `format!` allocates. Inside hot loops, `write!(&mut buffer, "...", args)` into reusable `String`.
- For small concat, `String::with_capacity(n)` + `push_str` avoids reallocations.
- For known-small strings, `SmartString` / `CompactString` / `inlinable_string` crates store up to ~24 bytes inline.
- `str::parse::<T>()` uses `FromStr`; efficient for numbers.
- `#[inline]` `format_args!` unlocks no-alloc formatting; `std::io::Write::write_fmt`.
- Use `memchr` crate for fast `find` on bytes (SIMD).

### Collections (perf-book + ER Item 21 on collections)

- `Vec<T>`: prefer over linked lists unless you truly need pointer stability; pointer chasing kills caches.
- `VecDeque<T>`: ring buffer; front and back O(1).
- `HashMap<K, V>` (default SipHash: DoS-resistant, ~2× slower than FxHash). Swap hasher via `HashMap::with_hasher(FxBuildHasher::default())`.
- `BTreeMap<K, V>`: sorted; range queries; ~2× slower than `HashMap` for random lookup but cache-friendly for sorted access.
- `IndexMap` / `IndexSet`: preserve insertion order; hash-backed.
- Small-collection crates: `smallvec::SmallVec<[T; N]>` (stack inline first N), `tinyvec`, `arrayvec::ArrayVec` (fixed-size, no heap).
- `Vec::with_capacity(n)` when final size is known — prevents geometric re-alloc.
- `Vec::extend(iter)` uses `size_hint` to reserve; idiomatic.
- `HashMap::entry(k).or_insert_with(...)` avoids double-lookup.
- `Vec::retain` / `HashMap::retain` for in-place filter.

### Heap allocations (perf-book "Heap Allocations")

- Every `Box::new`, `Vec::push` past cap, `String::from` is an alloc. Profile with heaptrack / DHAT / dhat-rs.
- Box-erased generics ("bloated enum") alternative: `Box<dyn Trait>` to keep sum types small.
- `Rc::clone` / `Arc::clone` bump a refcount (atomic in Arc case) — non-zero cost; avoid in hot loops.
- Reuse buffers: `buf.clear(); buf.extend_from_slice(&data);` in loops.
- `str::to_owned` vs `String::from` vs `.to_string()` — roughly equivalent; `to_owned` is most explicit.
- `Cow<'a, str>` to avoid alloc when input is already owned-enough.
- Arenas: `bumpalo`, `typed-arena`, `slab` for many-small-objects patterns (AST, ECS).

### Type sizes (perf-book "Type Sizes")

- `cargo +nightly rustc -- -Zprint-type-sizes` dumps sizes and paddings.
- Enum size = largest variant + discriminant, rounded to alignment. A 1-byte variant next to a `Box` makes the enum 16 bytes.
- Box the large variant: `enum Msg { Small(u8), Big(Box<[u8; 4096]>) }`.
- Reorder fields to reduce padding (Rust already does since 1.6, but `#[repr(C)]` fixes order).
- Option discriminant elision: `Option<&T>`, `Option<NonZeroU32>`, `Option<Box<T>>` are pointer-size — the null / zero niche is the `None` tag.
- Nested `Option<Option<&T>>` still packs via niche.
- Align-driven bloat: a `bool` next to a `u64` may cost 8 bytes.

### Box (perf-book)

- Use Box for: heap-only types (DSTs), trait objects, breaking large stack frames, recursive types (`enum List { Cons(T, Box<List>), Nil }`).
- `Box<[T]>` = size + pointer; cheaper than `Vec<T>` by one `usize` (no `cap`).
- `Box::leak(b)` produces `&'static mut T`; useful for statics-from-config at startup.
- `Box::pin(x)` = shorthand for `Pin<Box<T>>`.

### Vec / SmallVec (perf-book)

- Default growth factor 2× (exact: `cap = max(cap*2, requested)`). Reserve up front if predictable.
- `Vec::shrink_to_fit` after bulk build to free unused capacity.
- `smallvec::SmallVec<[T; 8]>`: stack inline up to 8 items, then spills to heap transparently. Good for <N-common cases.
- `tinyvec::TinyVec`: all-safe variant of smallvec.
- `arrayvec::ArrayVec<T, N>`: hard cap, no heap. Fails on overflow.
- For FFI / bulk-memcpy: `Vec::into_boxed_slice()` drops capacity, returns `Box<[T]>`.

### Hashing (perf-book "Hashing")

- Default std hasher = SipHash-1-3; cryptographically keyed, 2×–10× slower than non-DoS-resistant hashers.
- For trusted keys / internal maps: `rustc-hash::FxHashMap` (FxHash, fast, non-DoS-resistant), `ahash::AHashMap` (AES-accelerated), `fxhash::FxHashMap`.
- `HashMap` vs `BTreeMap` tradeoffs: HashMap ~O(1) lookup; BTreeMap has ordered iteration and range queries, better cache behaviour at small sizes.
- `IndexMap` retains insertion order; useful for deterministic iteration.
- For pointer keys: `HashMap<*const T, V>` uses pointer-addr hash; consider `FxHashMap` for speed.

### Bounds checks (perf-book)

- Indexing (`v[i]`) inserts a bounds check. Iterators eliminate most of these.
- `get_unchecked(i)` / `get_unchecked_mut(i)` are unsafe; use only with a separate runtime proof.
- Pattern: fetch length once, iterate `for i in 0..len { /* still checked */ }` — LLVM can often prove and remove.
- `slice::windows(n)`, `chunks(n)`, `chunks_exact(n)` — no bounds checks inside body.
- `iter().zip(other)` truncates to shorter; avoids dual bounds.

### I/O (perf-book)

- Always wrap file reads in `BufReader`; default unbuffered read is one syscall per `read()` call.
- `BufWriter` similarly; call `flush()` explicitly before drop (drop panics are lossy).
- For large sequential writes, increase buffer: `BufWriter::with_capacity(64 * 1024, file)`.
- `std::io::Write` on `Vec<u8>` lets you `writeln!` into memory.
- `read_to_string` / `read_to_end` pre-allocate when file length is known (uses `seek_len`).
- `memmap2` for huge-file random access — zero-copy but OS-paging-dependent.

### Logging (perf-book)

- `log` facade + `env_logger` / `tracing` / `slog`. `tracing` is the modern default for async.
- Trace-level calls still cost something if the arg is computed. Use closures:

```rust
tracing::debug!(count = %expensive_count(), "done");
// or:
if tracing::enabled!(tracing::Level::DEBUG) { ... }
```

- `tracing::instrument` macro auto-instruments spans. Can elide args with `skip`.
- For ultra-hot paths, gate with `#[cfg(feature = "trace")]`.

### Parallelism (perf-book "Parallelism")

- `rayon` for data parallelism: `par_iter()`, `par_bridge()`, `par_sort_by`, `par_chunks`. API-compatible with std Iterator.
- Work-stealing scheduler; chunk sizes auto-tuned; no `Send + Sync` guarantee leak.
- `std::thread::scope` (stable 1.63) lets borrowed refs cross threads without `'static`.
- `std::thread::available_parallelism` since 1.59 for thread-pool sizing.
- Avoid thread-per-task; use `rayon::spawn` or a channel + worker pool.

### Machine code (perf-book "Machine Code")

- `cargo rustc --release -- --emit=asm`; inspect with `cargo-show-asm` / `cargo-asm`.
- Common wins: align hot loops (`#[repr(align(64))]` on struct), manual SIMD (`std::simd`), replace `panic!` branches with `debug_assert!`.
- `core::hint::black_box` forbids DCE in benchmarks.
- `core::hint::unreachable_unchecked()` in unsafe — tells LLVM a branch is dead; use after invariants proven.

---

## 04-design-patterns — Macro, builder, newtype, sealed trait, typestate

### Newtype pattern (ER Item 6 / Rust idiom)

- `pub struct UserId(pub u64);` — compile-time distinction, runtime cost zero.
- Hide inner: `pub struct UserId(u64);` with constructors — stronger encapsulation.
- Derive as needed: `#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]`.
- `AsRef<u64>` / `From<u64>` / `Into<u64>` for interop.
- Macro helper: `#[derive(derive_more::Display, derive_more::FromStr)]` auto-implement string conversions.

### Sealed trait pattern

- Prevents downstream users implementing your trait (avoids breaking changes):

```rust
mod sealed { pub trait Sealed {} }
pub trait MyTrait: sealed::Sealed { ... }
impl sealed::Sealed for Foo {}
impl MyTrait for Foo { ... }
```

- Classic in `std` (`Iterator`'s private methods are trait-gated). Stabilised via pub-in-private trick since 1.0.

### Typestate pattern

- Encode state as phantom type parameter:

```rust
struct Locked;
struct Unlocked;
struct Door<S = Locked> { state: PhantomData<S> }
impl Door<Locked> {
    fn unlock(self, _key: Key) -> Door<Unlocked> { Door { state: PhantomData } }
}
impl Door<Unlocked> {
    fn open(self) { ... }
}
```

- Illegal transitions become type errors. `open()` on `Door<Locked>` is unreachable.

### Visitor via enum + trait

- Combine `enum Expr { Add(Box<Expr>, Box<Expr>), Lit(i64) }` with `trait Visitor { fn visit(&mut self, e: &Expr); }` — double-dispatch without object-oriented inheritance.

### Builder (three variants)

```rust
// Consuming builder
pub struct Req { u: String, t: u64 }
pub struct ReqBuilder { u: Option<String>, t: u64 }
impl ReqBuilder {
    pub fn new() -> Self { Self { u: None, t: 30 } }
    pub fn url(mut self, s: impl Into<String>) -> Self { self.u = Some(s.into()); self }
    pub fn timeout(mut self, s: u64) -> Self { self.t = s; self }
    pub fn build(self) -> Result<Req, &'static str> {
        Ok(Req { u: self.u.ok_or("url")?, t: self.t })
    }
}

// Typestate builder — url required
pub struct NoUrl;  pub struct HasUrl(String);
pub struct TReq<U>(U);
impl TReq<NoUrl> { pub fn new() -> Self { TReq(NoUrl) } pub fn url(self, u: String) -> TReq<HasUrl> { TReq(HasUrl(u)) } }
impl TReq<HasUrl> { pub fn build(self) -> Req { ... } } // only callable with HasUrl
```

### Extension trait pattern

- Add methods to upstream types without modifying them:

```rust
pub trait SliceExt { fn first_duplicate(&self) -> Option<&T>; }
impl<T: PartialEq> SliceExt for [T] { ... }
```

- Import (`use crate::ext::SliceExt;`) to enable.

### RAII guards / scope guards

```rust
struct LockGuard<'a, T> { inner: &'a Mutex<T> }
impl<T> Drop for LockGuard<'_, T> { fn drop(&mut self) { self.inner.unlock(); } }
```

- Crate `scopeguard::{defer, defer_on_success}` — one-liner cleanup.

### Functional update via `..old`

```rust
let b = Config { log_level: LogLevel::Debug, ..a };
```

- Requires matching types; fields you list override.

### `From` / `Into` tower for errors

- Every error variant gets `#[from]` via `thiserror`; `?` walks conversions automatically.

### Macro-derived patterns

- `#[derive(Serialize, Deserialize)]` — `serde`.
- `#[derive(thiserror::Error)]`.
- `#[derive(bon::Builder)]` — typed builders.
- `#[derive(strum::EnumString, strum::Display)]` — enum ↔ string.

### Macro design patterns (from TLBORM)

- **TT muncher**: progressively consume tokens from the front, emit a recursive call with the tail; canonical for token-list parsing.

```rust
macro_rules! count_commas {
    () => { 0 };
    (, $($rest:tt)*) => { 1 + count_commas!($($rest)*) };
    ($head:tt $($rest:tt)*) => { count_commas!($($rest)*) };
}
```

- **Push-down accumulator**: build up an output as an extra argument, continue recursion with larger accumulator. Used in procedural macros for quoted output.

```rust
macro_rules! reverse {
    (@acc [$($acc:tt)*]) => { ($($acc)*) };
    (@acc [$($acc:tt)*] $head:tt $($rest:tt)*) => { reverse!(@acc [$head $($acc)*] $($rest)*) };
    ($($input:tt)*) => { reverse!(@acc [] $($input)*) };
}
```

- **Internal rules**: prefix with `@name` to distinguish dispatched branches.
- **Callback pattern**: accept a macro path to invoke with intermediate state:

```rust
macro_rules! call_with {
    ($cb:ident!($($a:tt)*) with $($b:tt)*) => { $cb!($($a)* $($b)*) };
}
```

- **Repetitions and slices**: `$(...)*` , `$(...)+` , `$(...),*` , `$(...),+`  ; `$#(name)` for separator fragments (unstable, keep an eye).
- **FRAGMENT specifiers**: `item`, `block`, `stmt`, `pat`, `pat_param` (2021+), `expr`, `ty`, `ident`, `path`, `tt`, `meta`, `lifetime`, `literal`, `vis`. `tt` is the most general — entire token tree.
- **Hygiene**: `macro_rules!` is partially hygienic — local identifiers (let bindings) won't collide with caller's scope; but paths (e.g., `Vec`) resolve in caller's context, so macros typically use `::std::vec::Vec` for robustness.
- **Import/export via `#[macro_export]`**: crate-level export; `pub use crate::my_macro;` re-export pattern.
- `**$crate**`: resolves to current crate — essential for exported macros to reference their own items portably.

### Procedural macro patterns

- `proc-macro = true` in `[lib]` section; one proc-macro crate per workspace.
- Three flavours: `#[proc_macro]` (function-like), `#[proc_macro_derive(Name)]`, `#[proc_macro_attribute]`.
- Cannot emit anything except tokens; use `proc_macro::TokenStream` at the boundary; internally use `proc_macro2::TokenStream` (allows use outside proc-macro crate, eg in tests).
- `syn` parses tokens into typed AST; `quote!` emits tokens with interpolation (`#var`, `#(#list),*`).
- Error reporting: `syn::Error::new(span, "msg").to_compile_error()` — reports on correct source span.

---

## 05-anti-patterns — What NOT to do

### From Effective Rust

**Panicking in library APIs**

- Library: never panic on caller input; return `Result`. Panic only on internal invariant violations that indicate a bug.

`**Box<dyn Error>` in public API**

- Loses variant-level matching; downstream can't pattern-match. Export typed `thiserror` enum instead.

**Over-generic APIs**

- Extreme `fn foo<A, B, C, D>(...)` where bounds aren't necessary makes signatures unreadable. Prefer concrete types or `impl Trait`. Extract helper structs.

**Stringly-typed APIs**

- Passing `&str` for "kind" / "mode" — use an enum. Saves runtime errors, speeds code (no `strcmp`).

**Allocating hashmaps as cheap multimaps**

- Constructing `HashMap<&str, Vec<_>>` with `.entry(k).or_insert_with(Vec::new).push(v)` is fine; but don't then clone keys into the map on every lookup — use `Cow<'a, str>` or owned key.

`**clone()` as lifetime band-aid**

- Adding `.clone()` to silence the borrow checker is a smell. Often a structural fix (split struct, take `&mut` not `&`, rethink ownership) is warranted.

**Lifetime elision surprises**

- `fn longest<'a, 'b>(x: &'a str, y: &'b str) -> &'a str` compiles but is misleading — `y`'s lifetime isn't tied to the return. Elide carefully.

`**Arc<Mutex<T>>` for hot shared state**

- Contention → serialisation. Consider sharded locks (`dashmap`), atomics, or per-thread state + aggregation.

**Unsafe for speed without benchmarks**

- Usually slower than idiomatic code because you disable optimisations the compiler would do under safety proofs.

**Reflection emulation via `Any::downcast`**

- If you find yourself downcasting in hot paths, refactor to trait methods.

### From Perf Book

**Unbuffered file I/O in a loop** — always wrap in `BufReader` / `BufWriter`.

`**println!` / `eprintln!` in hot loops** — acquires stdout lock, flushes on newline — huge overhead. Batch via `write!(stdout.lock(), ...)`.

`**String += &format!(...)`** — every `format!` allocates a new heap buffer. Use `write!(&mut buf, "...", ...)`.

`**Vec::insert(0, x)**` — O(n). Use `VecDeque::push_front` when needed.

`**HashMap::get` twice then `.insert**` — use `.entry(k)` to avoid double-lookup.

`**.to_string()` in comparisons** — `s1 == s2.to_string()` allocates; just compare `&str` to `&str`.

**Building `HashMap<String, _>` from `&str` keys without `HashMap::with_capacity`** — resize cost.

**Calling `Vec::clear` + reusing without shrink** — `clear` keeps capacity; that's usually *good*. The anti-pattern is the opposite: `vec = Vec::new()` inside a loop to "reset" — drops capacity.

**Growing via repeated `push` when size is known** — reserve first.

**Default SipHash when DoS not a threat** — switch to FxHash or AHash for internal maps.

**Non-inlining across crates at -O0** — release builds use thin-LTO by default; debug builds don't inline across crates, making microbenches misleading.

**Unnecessary monomorphisation** — generic `fn write_all<W: Write>(...)` duplicated across many callers. If performance isn't critical, use `&mut dyn Write` to share code and improve compile times.

**Recursive enums without `Box`** — doesn't compile anyway, but beginners try it; include `Box` on recursive variants.

### From TLBORM (macro anti-patterns)

**Missing `$crate` in exported macros** — breaks when imported from elsewhere.

**Referring to `Vec` instead of `::std::vec::Vec`** — shadowed if the caller redefined `Vec`.

**Deep TT-muncher recursion** — compiler recursion limit (defaults to 128); use `#![recursion_limit = "256"]` or push-down accumulator.

**Using `expr` fragment then expecting `$e;` to parse** — `expr` cannot be followed by `;` in some older editions; `stmt` is better. Edition 2021+ relaxed many follow-set restrictions.

**Assuming hygiene** — `macro_rules!` hygiene is identifier-level; items / paths are *not* hygienic. Use absolute paths.

**Capturing `tt` and substituting into a different fragment class** — token trees are opaque; you can't pattern-match on their substructure afterwards.

**Ignoring span** — proc-macro errors and output should carry accurate spans (`Span::call_site()` vs `Span::mixed_site()` — `mixed_site` gives hygienic references for identifiers introduced by the macro).

**Overloading function-like proc macros for trivia** — any time `macro_rules!` works, prefer it; proc macros dwarf compile times.

**Implementing custom derives that emit non-trivial logic** — keep derives focused on boilerplate (From/Debug/serde); emit helper functions instead of inlining large fn bodies.

---

## 06-error-handling — Effective Rust chapter + idioms

### Core principles

- Two distinct failure modes: expected errors (network timeout, parse failure, not-found) → `Result`; bugs / invariants broken → `panic!`.
- Never mix: an invalid UTF-8 string from a file is an expected error, not a bug.

### `Result<T, E>`

- Early-return with `?` requires `From<E1> for E2` (where `E2` is the function's error type).
- `.map_err(|e| MyErr::Io(e))?` to adapt without a `From` impl.
- `.ok()` converts `Result<T, E>` to `Option<T>`; `.err()` to `Option<E>`.

### `Option<T>`

- `?` on `Option` in `Option`-returning fn is stable.
- `let Some(x) = opt else { return ... };` — let-else, stable 1.65.
- Or-chain with `or_else`, `and_then` for lazy fallbacks.

### Error enum design

```rust
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("connection failed")]
    Connect(#[source] std::io::Error),

    #[error("query error: {0}")]
    Query(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

### `#[non_exhaustive]`

- On pub enum — downstream match must include `_ =>`. Avoids breaking changes when you add variants.
- Same on struct — downstream can't construct or exhaustively destructure.

### Error aggregation vs wrapping

- Aggregate: a single `enum CrateError` covers all failure modes.
- Wrap: a per-module error type, with the top-level crate error aggregating via `#[from]`.
- Wrapping is more scalable; aggregation is fine for small crates.

### Source chain

- `impl Error for MyErr { fn source(&self) -> Option<&(dyn Error + 'static)> { ... } }` — carries causal chain.
- `anyhow` walks the chain via `{:#}` format.
- Display is user-facing; Debug is developer-facing. Good Error types differ between them.

### Panic configuration

- `[profile.release] panic = "abort"` — no unwinding, smaller binaries, no cross-FFI concerns, but no `catch_unwind`.
- `"unwind"` default allows `std::panic::catch_unwind` boundaries (web handlers, plugin isolation).
- `#[panic_handler]` required on `no_std` binaries.

### `catch_unwind`

- Wrap calls to untrusted code (plugin, FFI) to turn panic into error. Requires `UnwindSafe`. Don't use as general error handling.

### Returning `impl IntoIterator<Item = Result<_, _>>`

- Stream of results; caller decides how to aggregate (`collect::<Result<Vec<_>, _>>()` short-circuits; `.filter_map(Result::ok)` ignores errors).

### Panicking helpers

- `todo!()` — typed `!`, compiles, panics with "not yet implemented".
- `unimplemented!()` — historical; still useful.
- `unreachable!()` — strong claim; LLVM can optimise based on it in release (via optimizer, not semantically).
- `assert!` / `debug_assert!` — latter elided in release.

### Library-policy pattern

- Functions that panic → document "# Panics" section.
- Functions that allocate a lot → document "# Performance".

### Error design rules of thumb

- Typed errors at API boundaries; opaque within implementation.
- Don't leak internal dependencies (wrapping `reqwest::Error` directly couples you to reqwest's major).
- Include enough context that logs alone can diagnose: request ID, key, method name.

---

## 07-async-concurrency — Effective Rust + perf-book + best practice (1.75+)

### Async core concepts

- `async fn` desugars into a state machine implementing `Future`. Each `.await` is a suspension point.
- `Future::poll` is called by an executor; returns `Poll::Pending` or `Poll::Ready(T)`.
- A `Waker` is how the future says "I'm ready now"; the executor schedules `poll` again.
- No runtime is built in — you pick `tokio`, `async-std` (deprecated), `smol`, `embassy` (embedded), `glommio` (thread-per-core io_uring), etc.

### `async fn` in traits (stable 1.75+)

- Traits can have `async fn` directly: no more `#[async_trait]` macro.
- Caveat: returned future is not `Send` by default; for `Send` bounds or `dyn`, still need workarounds (`trait_variant`, `dynosaur`).
- `trait_variant::make(Send)` generates a send-bound variant automatically.

### Async closures (stable 1.85)

- `async || { ... }` returns an opaque future; captures can be borrowed across awaits.
- `AsyncFn`, `AsyncFnMut`, `AsyncFnOnce` traits parallel `Fn` family.

### `impl Future<Output = T>` in traits (RPITIT)

- Stable since 1.75. Implementations must match the signature; opaque type hides concrete future.

### `Pin<P>` and self-referential futures

- State machines generated by `async fn` can be self-referential (future's local stores a borrow into future's state).
- `Pin<&mut T>` guarantees `T` won't move — so the self-ref stays valid.
- `Unpin` opt-outs this requirement. Most types are `Unpin`; generators and async fns are not (unless wrapped in `Box::pin`).
- `tokio::pin!(fut)` pins on stack; `Box::pin(fut)` allocates.

### Cancellation safety

- A `.await` that is aborted (via `select!`, drop) must leave state consistent.
- `tokio::fs::File::write` is not cancellation safe; `tokio::io::AsyncWriteExt::write_all` is partially.
- Rule: any function whose progress would be lost on cancel is "not cancel-safe"; document it.

### `select!`

- Polls multiple futures; first to complete wins; others are dropped.
- With `biased;` keyword, priority is fixed (top-down); otherwise random.

### Spawn patterns

- `tokio::spawn(async move { ... })` — separate task, independent lifecycle. Join handle.
- `tokio::task::spawn_blocking(move || { ... })` — offload CPU-bound work to a blocking pool.
- `tokio::task::block_in_place` — allow blocking inside current task (multi-threaded runtime only).

### Shared state

- `Arc<Mutex<T>>` OK if locks are short.
- `Arc<RwLock<T>>` for read-mostly.
- `tokio::sync::Mutex` is async (can await). `std::sync::Mutex` is blocking; OK inside tasks if critical section is short and doesn't await.
- For actor-style: `tokio::sync::mpsc` + single owner task.

### Concurrency primitives

- `tokio::sync::mpsc::{channel, unbounded_channel}` — multi-producer, single-consumer.
- `tokio::sync::broadcast` — multi-producer, multi-consumer, cloning.
- `tokio::sync::watch` — rolling latest value; good for config.
- `tokio::sync::oneshot` — single value, one-time signal.
- `tokio::sync::Notify` — cheap wakeup primitive.
- `tokio::sync::Semaphore` — bounded concurrency.
- `std::sync::OnceLock` / `std::sync::LazyLock` (stable 1.80) — lock-free lazy init; replace `lazy_static!` / `once_cell::sync::Lazy`.

### Stream processing

- `futures::Stream` is async Iterator: `.next().await -> Option<T>`.
- `tokio_stream::StreamExt` adds combinators.
- Backpressure naturally via bounded channels.
- `futures::stream::iter(v).for_each_concurrent(N, f)` fans out with concurrency cap.

### Structured concurrency

- Tasks spawned outside a scope leak — spawn inside a `JoinSet` or scoped runtime:

```rust
let mut set = tokio::task::JoinSet::new();
for url in urls { set.spawn(fetch(url)); }
while let Some(res) = set.join_next().await { ... }
```

- `tokio::task::JoinSet` aborts remaining on drop, providing structured cancellation.

### Common async anti-patterns

- **Blocking inside an async task**: any `std::thread::sleep`, `std::fs`, mutex held across await on contested locks. Use `tokio::time::sleep`, `tokio::fs`, etc.
- **Long-running CPU work inside async fn**: starves other tasks on that executor thread. Use `spawn_blocking`.
- **Forgetting to `.await`**: warning `unused_must_use`; also `clippy::async_yields_async`.
- **Sharing non-`Send` futures across threads with a multi-threaded runtime**: compile error; use `tokio::task::LocalSet` + `spawn_local` when you need `!Send` futures.

### Runtime choice cheatsheet

- `tokio` — default; large ecosystem; multi-threaded + current-thread runtime.
- `smol` — small, simple; minimal.
- `async-executor` + `async-io` — modular.
- `embassy` — `no_std` embedded; static allocation.
- `glommio` — thread-per-core io_uring; Linux only.
- `monoio` — similar; thread-per-core io_uring.

### Performance tuning (perf-book)

- `tokio::runtime::Builder::new_multi_thread().worker_threads(n)` — tune to cores.
- LIFO slot optimization for same-socket work.
- Avoid tiny tasks; each has overhead. Batch.
- Use `tokio-console` for async profiling.
- Profile with `tokio-metrics` and `tracing` + `tracing-flame`.

### `Send` and `Sync` considerations

- Async fns returning `impl Future` are `Send` iff every local held across `.await` is `Send`.
- `Rc<T>` makes a future `!Send`; use `Arc<T>` if task moves.
- `MutexGuard` across `.await` = `!Send` for `std::sync::Mutex` on some types; prefer `tokio::sync::Mutex` if guard crosses await.

---

## 09-performance — The main perf-book payload

### Compile times (perf-book "Compile Times")

#### Check debug builds, not release, during edit-compile-run

- `cargo check` type-checks only (~1/3 of build time); add `cargo-watch` for continuous.
- `cargo build --timings` generates HTML dependency-graph with per-crate timing.

#### Reduce compilation units

- Split heavy generic code into small crates so monomorphisation stays bounded.
- `[profile.dev] incremental = true` (default). Set `codegen-units = 256` for dev to maximise parallel codegen.
- `[profile.release] codegen-units = 16` default; lowering (even to 1) improves runtime perf at cost of compile time.

#### Use a faster linker

- Switch to `lld` (LLVM) or `mold` (Linux):

```toml
# .cargo/config.toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

- Rust 1.90+: `lld` is default on `x86_64-unknown-linux-gnu`.
- macOS: `ld64` (default) is fine; `lld` can work; the cranelift backend experiment reduces compile times further.

#### Cranelift backend

- `cargo +nightly build -Zcodegen-backend=cranelift` — faster debug compiles, ~OK runtime. Stabilising incrementally.

#### Disable unused crate features

- `default-features = false` on dependencies. Audit with `cargo tree --duplicates` and `cargo-feature-set`.
- `cargo-udeps` (nightly) / `cargo-machete` (stable) — find unused dependencies.

#### Shrink the dependency graph

- Each crate = cold-cache rebuild. Fewer deps = faster CI. Consider writing the 10-line utility rather than pulling a crate.

#### Workspace layout

- Share a single `Cargo.lock` across workspace.
- Shared `target/` saves rebuilds across projects; across workspaces use `sccache` or `CARGO_TARGET_DIR=/shared`.

#### `rustup`-level speedups

- `--target` tuning; install only targets you need.
- Use `rustc --emit=metadata` via `cargo check` for type-check-only.

#### Heavy derive macros are costly

- `serde_derive`, `clap_derive` add compile time — often unavoidable. For your own derives, keep code generation small.

#### Compile-time benchmarking

- `cargo +nightly rustc -- -Zself-profile` — produces profile data readable by `measureme` / `summarize`.
- `perf stat -r 3 cargo build` for wall-clock.
- `cargo bench` for the runtime side.

### Benchmarking (perf-book "Benchmarking")

#### `criterion` crate

- Statistical measurement; detects noise; auto-generates HTML reports.

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
fn bench_fib(c: &mut Criterion) {
    c.bench_function("fib 20", |b| b.iter(|| fib(black_box(20))));
}
criterion_group!(benches, bench_fib);
criterion_main!(benches);
```

- `--save-baseline main` then `--baseline main` on PR branch for regression detection.

#### `divan` crate

- Newer, simpler, `#[divan::bench]` attribute, sub-ns precision.
- Supports generics benchmarks per-type.

#### Ground rules

- Release build (`-O` at minimum, typically full `--release`).
- Wrap inputs in `black_box(x)` to prevent LLVM from optimising them into constants.
- Run on dedicated CPU cores (`taskset -c 2 ./bench`).
- Disable CPU frequency scaling (`cpupower frequency-set -g performance`) or acknowledge the noise.
- Pin threads for multi-threaded benches.
- Run each bench multiple times; Criterion handles statistical significance.

#### Micro- vs macro-benchmarks

- Micro: single function; fast feedback; risk: doesn't reflect real call patterns.
- Macro: end-to-end; more realistic; slower iteration.

#### Common pitfalls

- Dead-code elimination of inputs / results — use `black_box`.
- Micro-benchmarks of trivially-inlineable code measure the wrapper, not the work.
- JIT effects → Rust has none; but thermal throttling, turbo boost, shared caches still apply.

### Build configuration (perf-book)

#### `Cargo.toml` release tuning

```toml
[profile.release]
opt-level = 3            # default; 's' or 'z' for size
codegen-units = 1        # better inlining, slower build
lto = "fat"              # or "thin" (faster build, 90% of gains)
panic = "abort"          # smaller, faster, loses catch_unwind
strip = "symbols"        # strip binary
debug = false            # no debug info (use `1` for line tables only)
overflow-checks = false  # default in release; enable in dev
```

- `cargo-bloat` to see what's in the binary.
- `cargo build --release -Z build-std=std,panic_abort -Z build-std-features="panic_immediate_abort"` (nightly) — rebuild std with your opt flags.

#### Profile-guided optimization (PGO)

- `cargo-pgo` / rustc `-Cprofile-generate=` then `-Cprofile-use=`. Gains 5–20% in hot loops.
- Similar: BOLT post-link optimisation.

#### CPU target

- `RUSTFLAGS='-C target-cpu=native'` enables AVX2/AVX512 on modern x86. Non-portable binary.
- `target-cpu=x86-64-v3` for AMD Zen2+/Haswell+ binary that's still portable on modern hardware.
- `target-feature=+crt-static` on MUSL.

#### Debug info

- `debug = 1` in release provides line-tables-only (for profiling with perf / instruments) at ~0% runtime cost.

### Linting (perf-book + ER)

#### `cargo clippy`

- Hundreds of lints across groups: `correctness` (bugs), `perf`, `style`, `complexity`, `suspicious`, `pedantic`, `nursery`, `cargo`.
- Recommended in CI:

```toml
# Cargo.toml or .cargo/config.toml
[workspace.lints.clippy]
perf = "warn"
pedantic = "warn"
complexity = "warn"
```

- Top perf lints: `needless_collect`, `iter_cloned_collect`, `manual_memcpy`, `redundant_clone`, `inefficient_to_string`, `large_types_passed_by_value`, `redundant_allocation`, `map_clone`, `single_char_pattern`, `slow_vector_initialization`.

#### `rustc` lints

- `unused_must_use` — catches forgotten `.await`, `.send()`, `Result` discards.
- `rust_2018_idioms`, `rust_2024_compatibility` to migrate.
- `unsafe_code = "forbid"` on safe crates.

#### `#[deny(...)]` in CI

- `unused`, `future_incompatible`, `nonstandard_style`, `rust_2018_idioms`, `clippy::unwrap_used`, `clippy::expect_used` (less strict).

### Profiling (perf-book "Profiling")

#### What to profile: runtime, memory, cache, compile time

- `perf record -g ./target/release/mybin` then `perf report` / flamegraph via `inferno`.
- `cargo-flamegraph` wrapper: `cargo flamegraph --bin myapp`.
- Requires `debug = 1` in release for meaningful symbols.

#### Tools

- **perf** (Linux): CPU time, cache misses, branch misses.
- **Instruments** (macOS): Time Profiler, Allocations, Leaks, Activity Monitor.
- **Intel VTune**: microarchitecture-level.
- **samply**: cross-platform sampling profiler; produces Firefox Profiler format.
- **AMD uProf**: AMD-specific.
- **Hotspot**: KDE perf UI.
- **callgrind / cachegrind** (valgrind suite): deterministic instruction count, cache simulation.
- **heaptrack**: allocation profiler.
- **DHAT** (valgrind): allocation hotspots with call-tree.
- `**dhat-rs`** crate: Rust-in-process DHAT.
- `**jemalloc**` with `MALLOC_CONF=stats_print:true` for alloc stats.
- `**tracy**` + `tracing-tracy`: nanosecond frame profiler.
- `**tokio-console**`: async task inspection.
- `**cargo-instruments**` (macOS): wraps Instruments.

#### Microarchitectural metrics

- IPC (instructions per cycle): < 1 → memory-bound; > 2 → CPU-bound.
- Cache miss ratio: L1d, LLC.
- Branch misprediction: bad on modern CPUs because speculation penalty is high.

#### Workflow

1. `cargo flamegraph` — where does time go at function granularity?
2. Drill into hot function with `cargo-show-asm` or Godbolt.
3. Try a targeted change.
4. Re-run criterion benchmarks to confirm improvement.
5. Commit; repeat.

### Inlining (perf-book)

- `#[inline]` — suggest across crate boundaries; required for cross-crate inlining of generic / short fns.
- `#[inline(always)]` — force; use sparingly (micro-optimised hot helpers).
- `#[inline(never)]` — forbid; useful for improving benchmark isolation or reducing I-cache pressure.
- LTO (thin or fat) enables cross-crate inlining without per-fn `#[inline]`.
- Monomorphisation already inlines non-recursive functions when small.
- Observe effect with `cargo-show-asm` — compare before/after.

### Hashing (perf-book)

Already summarised under collections (03-idioms). Additional:

- `#[derive(Hash)]` hashes each field in declaration order. Order affects hash distribution only, not correctness.
- `Hash` implementors must be consistent with `PartialEq` (`a == b ⇒ hash(a) == hash(b)`). Hand-written impls often break this.
- `BuildHasherDefault<T>` to inject a non-default hasher into `HashMap<K,V,S>`.

### Heap allocations (perf-book)

Already under idioms. Extras:

- `Box::new_uninit_slice(n)` — single alloc then fill; avoids double-init.
- `Vec::reserve(n)` reserves exactly one alloc to hold existing + n more.
- `Vec::split_off(mid)` — reuses tail allocation.
- Small-vec optimisation in `SmallVec` / `smartstring` amortises zero allocs across small cases.
- Custom allocator: `#[global_allocator]` with `mimalloc`, `jemallocator`, `tcmalloc`, `snmalloc`. 10–30% on alloc-heavy workloads.

### Type sizes (perf-book)

Already covered. Extras:

- Use `std::mem::size_of::<T>()` at const-time:

```rust
const _: () = assert!(size_of::<Msg>() <= 32);
```

- Static assertions via `static_assertions` crate or stable `const _:() =`.

### Box, Vec, SmallVec (perf-book)

Already covered.

### Strings and Collections (perf-book)

Already covered. Extras:

- `BTreeMap::range(a..b)` — ordered range; near O(log n) + k; HashMap cannot.
- `HashMap::drain()` — move all entries out, reuse capacity.
- `HashSet::is_subset` / `is_superset` — specialised.

### Iterators (perf-book)

Already covered. Extras:

- `Iterator::fuse()` — turns `None → Some → None` pattern into stable `None`s.
- `Iterator::peekable()` — look-ahead without consumption; cheap.
- Parallel via `rayon`: replace `iter()` with `par_iter()` globally.

### Bounds checks (perf-book)

Already covered. Additional:

- `v.iter().enumerate()` gives `(idx, &T)` without manual indexing.
- `for (i, x) in v.iter().enumerate()` avoids `v[i]` subsequent check.
- `unsafe { *v.get_unchecked(i) }` — only after invariant proof; document safety rationale.

### I/O (perf-book)

Already covered. Extras:

- `std::io::copy(&mut reader, &mut writer)` — kernel-level `sendfile` on Linux when possible (1.75 optimised).
- `std::io::Read::read_exact` vs `read` — former fills buffer or errors.
- `mmap` / `memmap2` for huge read-only data (parsers, dictionaries).

### Logging (perf-book)

Already covered.

### Parallelism (perf-book)

Already covered. Additional:

- `rayon::prelude::*`, then `v.par_iter().filter(...).sum()`.
- `rayon::join(|| a(), || b())` — fork-join.
- `crossbeam::channel` — bounded / unbounded, MPMC.
- `std::sync::Barrier` for stage synchronisation.

### Machine code (perf-book)

Already covered.

### Additional perf-book topics

#### Unsafe and micro-optimisations

- `core::intrinsics::unlikely / likely` (nightly) — branch prediction hints.
- On stable: `#[cold]` on unlikely fn; helps code-placement.

#### SIMD

- `std::simd` (unstable): portable SIMD types `f32x4`, `u8x16`.
- Stable: `core::arch::x86_64::_mm_*` intrinsics — platform-specific.
- `cargo bench` to validate; auto-vectoriser can match hand SIMD in many simple loops.

#### Const evaluation

- `const fn` computes at compile time; no runtime cost.
- `const { ... }` blocks (stable 1.79+) — compile-time constant expressions inside fn bodies.

#### Binary size

- `cargo bloat --release --crates` → per-crate contribution.
- `-C opt-level="z"` + `panic = "abort"` + LTO + `strip`.
- `no_std` + `alloc` for embedded.

#### Cold paths

- `#[cold]` attribute; placed on error paths so CPU prefetcher ignores them.

---

## 10-testing-and-tooling — ER deps/tooling + perf-book profiling

### Effective Rust items on dependencies

**Take care when choosing a crate**

- Prefer widely used, well-maintained crates. Check: download count, last release, issue response time, code of conduct, number of maintainers.
- `cargo-crev`, `cargo-vet` for reviews and trust chains.
- `cargo-audit` — flag known RUSTSEC vulnerabilities.
- `cargo-deny` — license compliance, duplicate detection, version gating, vendor allow-list.

**Version pinning**

- `Cargo.toml` accepts ranges (`"1.2"` means `>=1.2, <2.0`). `Cargo.lock` pins exact in binaries (checked in) but not libraries (not checked in historically; 1.84+ recommends checking in for libs too).
- `cargo update -p foo --precise 1.2.3` to set specific.

**Feature flags**

- Additive only! Features should never remove items. Downstream may turn on any combo.
- `default-features = false` to opt out.
- `resolver = "2"` (default in 2021 edition) isolates dev-dep features from build / bin feature sets.

**Minimal Supported Rust Version (MSRV)**

- Declare via `rust-version = "1.75"` in `Cargo.toml`.
- `cargo-msrv` finds the minimum version that compiles your crate.
- Semver expectation: raising MSRV is at least a minor bump.

### Tooling (ER + ecosystem)

**Format**: `cargo fmt`. Config in `rustfmt.toml` — e.g., `max_width = 100`, `imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`, `reorder_imports = true`.

**Lint**: `cargo clippy --all-targets --all-features -- -D warnings`. CI-mandatory.

**Docs**: `cargo doc --open`. Doc tests run via `cargo test --doc`.

- `#![warn(missing_docs)]` for library crates.
- `//!` module-level; `///` item-level; `#[doc(hidden)]` hide from docs; `#[doc(cfg(...))]` gate for docs.rs.
- `cargo-readme` generates README from lib.rs.

**Docs.rs**:

```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
targets = ["x86_64-unknown-linux-gnu"]
```

**Test**: `cargo test` runs unit, integration, and doc tests.

- Unit: `#[cfg(test)] mod tests { ... }` inside src files.
- Integration: `tests/*.rs` — treated as external crate; tests only pub API.
- Doc tests:  `` fences in ///; verified by `cargo test`.
- `#[should_panic(expected = "msg")]` — asserts panic and message.
- `#[ignore]` for slow tests; run with `cargo test -- --ignored`.
- `cargo nextest` — parallel test runner, faster, better output.

**Property testing**: `proptest`, `quickcheck` — shrink on failure.

- `proptest! { #[test] fn sort_is_idempotent(v in vec(any::<u32>(), 0..100)) { ... } }`.

**Fuzzing**: `cargo fuzz` + libFuzzer; `afl.rs`; `honggfuzz-rs`.

**Snapshot testing**: `insta` crate — capture outputs, review diffs.

**Mocking**: `mockall` (auto-derive on traits), `faux`, `double`.

**Continuous integration**

- Toolchain via `rust-toolchain.toml`: `[toolchain] channel = "1.85.0"`.
- GitHub Actions: matrix over OS, stable / beta / nightly, MSRV.
- Cache `target/` keyed on Cargo.lock hash.
- `cargo-deny`, `cargo-audit`, `cargo-outdated` in CI.

**Release management**

- `cargo-release` automates version bumps and publishing workflows.
- Semver: breaking = major; additive = minor; fix = patch.
- `#[non_exhaustive]` on public enums / structs to leave room for extension.

### Perf-book profiling tools

Already listed under 09-performance → Profiling. Additional CI coverage:

- `**iai`** / `**iai-callgrind**` — deterministic cycle counts (no wall-clock variance); good for CI.
- `**codspeed**` — continuous benchmarking SaaS; runs Criterion in isolated cpuset.
- `**bencher.dev**` — CI perf tracking, regression alerts.

### Test structure rules of thumb

- Each test is independent; no shared state.
- `#[test]` functions should be < 100 lines; extract helpers.
- Use `Default::default()` builders in tests; typed-state "happy path" factories.
- Tests touching filesystem / network → mark `#[ignore]` or gate on env var; use `tempfile` crate.
- `serial_test::serial` for tests that must run serially (shared resource).

---

## 12-modern-rust — Rust 1.75+ features

### 1.75 (Dec 2023)

- `**async fn` in traits**: stable.
- **RPITIT**: return-position `impl Trait` in traits.
- `**Symmetric` `Hash`** / consistent trait methods.
- `Ranged` ptr alignment, pointer byte offsets.

### 1.77

- `async fn` in trait bounds stable.
- C-string literals: `c"hello"` → `&CStr`.
- `offset_of!` macro stable.

### 1.78

- `#[diagnostic::on_unimplemented]` / `#[diagnostic::do_not_recommend]` — trait diagnostic hints.
- Deterministic realloc in LazyCell / LazyLock.

### 1.79

- `inline const` expressions in pattern contexts and fn bodies:

```rust
let arr = [const { Mutex::new(0) }; 16];
```

- Bounds in associated type position ("associated type bounds").

### 1.80

- `std::sync::LazyLock` + `LazyCell` stable — replaces `once_cell::sync::Lazy` / `Lazy` entirely.
- `Option::take_if`.
- Inferred types in `impl Trait`-return generics.

### 1.81

- `#[expect(lint_name)]` — required-positive lint suppression (errors if no warning occurs).
- `std::panic::PanicHookInfo` replaces `PanicInfo` in hooks.

### 1.82

- `&raw const EXPR` / `&raw mut EXPR` — create raw pointers without creating references (critical for packed / alignment-unsafe fields).
- `use<'a, T>` captures syntax for `impl Trait` in return position.
- CFI / KCFI control-flow-integrity support.

### 1.83

- Const control-flow expansion — `if`, `match`, `while`, etc. in const fn.
- `ErrorKind::InvalidFilename`.

### 1.84

- Precise capturing (`use<>` on trait methods).
- `Ipv4Addr` / `Ipv6Addr` consts expansion.
- `pin!` no longer requires `std`.

### 1.85 (Rust 2024 edition)

- `**async` closures** (`async || { ... }`), `AsyncFn` traits.
- **Let chains** in 2024 edition: `if let Some(x) = a && let Some(y) = b { ... }`.
- `use<..>` in trait fns.
- `gen` blocks / `gen fn` (iterator DSL) — stabilising in 1.85+.
- Rust 2024 edition: new prelude adds `Future` / `IntoFuture`; `static mut` requires `unsafe`; RPIT captures all in-scope generics and lifetimes by default; `cargo` default resolver = "3".

### 1.88

- Let chains all editions.
- Naked functions stable (`#[unsafe(naked)]`).

### 1.90

- `lld` default on `x86_64-unknown-linux-gnu`.

### 1.93

- `MaybeUninit` new slice helpers: `write_copy_of_slice`, `assume_init_drop`, etc.
- `String::into_raw_parts` / `Vec::into_raw_parts` stable.
- `unchecked_neg`, `unchecked_shl`, `unchecked_shr` for `*i8`/`*u8`/... family.
- `asm_cfg` attributes for conditional asm.

### 2024 edition specifics

- New prelude: `Future`, `IntoFuture`.
- `static mut` usage outside `unsafe` blocks is now an error — replace with `LazyLock` / `OnceLock`.
- Default resolver `"3"` — smarter feature unification.
- Temporary scopes in `if let ... else` chains adjusted to avoid surprising extensions.
- RPIT capture: captures all in-scope generics / lifetimes by default; use `use<...>` to narrow.

### Modern-Rust idioms checklist

- Replace `lazy_static!` / `once_cell::sync::Lazy` with `std::sync::LazyLock` (1.80).
- Replace `async_trait` with native `async fn` in traits (1.75) if `Send` / `dyn` unnecessary.
- Replace `std::ptr::null_mut()` + `&mut `* with `&raw mut` (1.82).
- Replace `macro_rules!` count tricks with `${count($x)}` metavar (unstable; roadmap).
- Use let-else for early returns on `None` / pattern mismatch.
- Use let-chains (1.88) for composed conditions instead of nested `if let`.

---

## Macros — detailed coverage from TLBORM

### Declarative macros (`macro_rules!`)

#### Basics

- Match tree: one or more rules, each `(matcher) => (body)`.
- Matchers consume token trees; bodies emit token trees.
- First matching rule wins (top-down).

#### Fragment specifiers and follow-sets

- `item`, `block`, `stmt`, `pat`, `pat_param` (2021+ pat restricted to no `|` at top), `expr`, `ty`, `ident`, `path`, `tt`, `meta`, `lifetime`, `literal`, `vis`.
- Each fragment class has a "follow set" — tokens allowed immediately after it:
  - `expr` / `stmt`: follow with `=>`, `,`, `;`.
  - `pat` / `pat_param`: follow with `=>`, `,`, `=`, `if`, `in`.
  - `ty`: follow with `,`, `=>`, `:`, `=`, `>`, `;`, `|`, `{`, `[`.
  - Others have broader follows.
- Violating follow-set is a compile-time error with explicit message.

#### Repetitions

- `$( ... )*` — zero or more.
- `$( ... )+` — one or more.
- `$( ... )?` — zero or one (since 1.32).
- Separator: `$( ... ),*`.
- Nested repetitions allowed; each `$x` must be bound at same depth and iterated together.

#### Metavariables in output

```rust
macro_rules! vec_of_strings {
    ( $($s:expr),* $(,)? ) => {{
        let mut v = ::std::vec::Vec::new();
        $( v.push(::std::string::ToString::to_string(&$s)); )*
        v
    }};
}
```

#### Hygiene (macro_rules!)

- Identifier-level hygiene: local bindings introduced in macro don't shadow caller's; caller's don't shadow macro's.
- Path-level non-hygiene: `Vec` in macro body resolves in the call site's namespace — so macros should use absolute paths (`::std::vec::Vec`).
- `$crate`: absolute path to the crate that defined the macro; essential for exported macros.

#### Exporting and importing

- `#[macro_export]` makes macro available at crate root after `extern crate` / in 2018+ via normal `use` path: `use my_crate::my_macro;`.
- `#[macro_use] extern crate foo;` (2015 edition) imports all macros — deprecated style.
- Re-export: `pub use crate::my_macro;`.
- `macro_rules!` at module scope: private by default.

#### Scoping and ordering

- `macro_rules!` definitions are textual; must be defined before use within the same file.
- Cross-module: use `pub use` or `#[macro_export]`.

#### Debugging

- `trace_macros!(true);` (nightly) — prints expansion steps.
- `cargo expand` (cargo-expand crate) — shows fully expanded source.
- `log_syntax!(...)` (nightly) — print token-level debug.

#### TT Muncher

Canonical pattern: peel off one token at a time, recurse with the tail.

```rust
macro_rules! as_expr { ($e:expr) => { $e }; }
macro_rules! count_tts {
    () => { 0 };
    ($one:tt) => { 1 };
    ($one:tt $two:tt) => { 2 };
    // exponential blowup avoided by recursion:
    ($($pairs:tt $_p:tt)*) => { count_tts!($($pairs)*) << 1 };
    ($odd:tt $($pairs:tt $_p:tt)*) => { (count_tts!($($pairs)*) << 1) | 1 };
}
```

- Guard against `recursion_limit = "128"` default — use `#![recursion_limit = "256"]` or binary-recursion pattern.

#### Push-down Accumulator

Carry partial result through recursive calls:

```rust
macro_rules! reverse {
    (@r [$($r:tt)*]) => { [$($r)*] };
    (@r [$($r:tt)*] $h:tt $($t:tt)*) => { reverse!(@r [$h $($r)*] $($t)*) };
    ($($t:tt)*) => { reverse!(@r [] $($t)*) };
}
```

#### Internal rules

Prefix unreachable-from-outside rules with `@name`. Convention: `@`-prefixed tokens won't collide because `@` can't appear in user expressions at that position.

#### Callback pattern

Macro accepts a callback macro path + extra args:

```rust
macro_rules! call_with_one {
    ($callback:ident ! ( $($args:tt)* )) => {
        $callback!($($args)* 1);
    };
}
macro_rules! show { ($x:expr) => { println!("{}", $x) } }
// usage:
// call_with_one!(show!());
```

#### Counting tricks

- Slice length trick: `<[()]>::len(&[$(replace_expr!($xs, ())),*])` — evaluates at compile time (arguably).
- `${count($x)}` — unstable; preferred future.

#### Fragment-classifying helpers

- `as_expr!` / `as_ident!` idioms to force a fragment classification.

#### Pitfalls and gotchas

- `$e:expr` can't be followed by `;` in certain contexts (edition-dependent).
- `macro_rules!` doesn't truly distinguish calls vs pattern matching — test carefully.
- Repetition drift: in `$( ... )*`, all metavariables must come from same repetition depth.
- `#[allow(...)]` in macro body doesn't always propagate — use `#[allow_internal_unstable]` / `#[macro_use]` carefully.

### Procedural macros

#### Crate layout

```toml
# proc-macro crate's Cargo.toml
[lib]
proc-macro = true

[dependencies]
syn = { version = "2", features = ["full", "extra-traits"] }
quote = "1"
proc-macro2 = "1"
```

- Only proc-macro crates can export `proc_macro` functions; can't export other items.
- Splitting: a proc-macro crate calls into a normal library crate for parse/emit logic — enables unit testing on the underlying functions.

#### Function-like proc macros

```rust
use proc_macro::TokenStream;
use quote::quote;

#[proc_macro]
pub fn make_answer(_input: TokenStream) -> TokenStream {
    quote! { fn answer() -> u32 { 42 } }.into()
}
```

Usage:

```rust
use my_macros::make_answer;
make_answer!();
fn main() { println!("{}", answer()); }
```

#### Derive macros

```rust
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(HelloMacro)]
pub fn hello_macro_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    quote! {
        impl HelloMacro for #name {
            fn hello() { println!("Hello from {}!", stringify!(#name)); }
        }
    }.into()
}
```

#### Attribute macros

```rust
#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream { ... }
```

Invocation: `#[route(GET, "/users")] fn list_users() { ... }`.

#### `syn` common usage

- `syn::parse_macro_input!(input as T)` — panics on parse error with compile diagnostic.
- `syn::Item` / `ItemFn` / `ItemStruct` / `Expr` / `Type` / `Path` etc. — typed AST.
- `DeriveInput { ident, generics, data, attrs, vis }` — what derive macros start with.
- `Data::Struct(DataStruct { fields, .. })` — destructure to enumerate fields.
- `Fields::Named(FieldsNamed { named })` vs `Fields::Unnamed(...)` vs `Fields::Unit`.
- `Generics::split_for_impl()` → `(impl_generics, ty_generics, where_clause)` for clean `impl`.
- `Attribute::parse_args_with(parser)` for `#[my(key = "value")]`.
- `syn::parse::Parse` / `ParseStream` for custom input grammars; `syn::Token![,]` / `Token![if]` macros for token assertion.

#### `quote!` essentials

- Interpolation via `#var`; for vectors: `#(#list),*`.
- `quote_spanned! { span => ... }` uses a specific span for emitted tokens.
- `ToTokens` trait: types appearing inside `#x` must implement it; `syn::Ident`, `syn::Type`, `proc_macro2::Literal` all do.
- Avoid `quote!` for enormous outputs (binary size + compile time); split into helper fns.

#### Error handling in proc macros

```rust
use syn::Error;
Error::new(field.span(), "unsupported field kind")
    .to_compile_error()
    .into()
```

- Prefer reporting errors on the *specific* span in user code for good diagnostics.
- Alternatively `syn::Result<TokenStream>` and `manyhow`/`proc-macro-error2` crates for richer error machinery.

#### Testing proc macros

- Logic crate (not the proc-macro crate) gets unit tests.
- Integration: `trybuild` crate — compiles `.rs` examples, asserts pass / fail & diagnostics match snapshot.
- `macrotest` — expansion snapshots.

#### Hygiene (proc macros)

- Two spans: `Span::call_site()` — tokens look like they came from call site (classic, non-hygienic); `Span::mixed_site()` — like `macro_rules!` hygiene.
- `quote!` default: `call_site`. Introduce local identifiers with `proc_macro2::Ident::new_raw("x", Span::mixed_site())` when you need hygienic locals.

#### `proc-macro2`

- Mirror of `proc-macro` crate usable outside proc-macro contexts (tests, logic crates, non-proc-macro libs). `From<proc_macro::TokenStream>` / `Into` at the boundary.

#### Common proc-macro patterns

- **Parse, transform, emit**: most common. `parse_macro_input!` → manipulate → `quote!`.
- **Delegation** to a helper crate so expansion logic is testable without building a proc-macro.
- **Helper attributes**: `#[proc_macro_derive(Foo, attributes(foo))]` adds `#[foo(...)]` recognised attributes on fields.

#### Performance tips

- Proc macros run on every compile; keep them fast.
- Cache static data; avoid filesystem I/O at macro expansion time.
- `syn` feature "full" is heavy — disable if you only parse simple grammars.

### `macro` (Macros 2.0) — decl_macro

- `macro foo($x:expr) { ... }` — future successor to `macro_rules!`.
- Hygiene: fully hygienic at paths and idents.
- Visibility: `pub macro foo { ... }` — uses normal item visibility.
- Status: unstable (feature: `decl_macro`). Not yet stable as of 1.93.
- Use `macro_rules!` for production; watch `macro` for future migration.

---

## Cross-reference index by topic

- **Zero-cost abstractions**: ER 9 (iterators), 12 (impl Trait), 13 (generics vs dyn). perf-book: bounds checks, inlining.
- **Compile time vs run time**: perf-book all of "Compile Times" + "Build Configuration"; ER dependencies chapter.
- **Memory**: perf-book Heap Allocations + Type Sizes + Box + Vec + Strings. ER 8 (pointer types), 10 (Drop).
- **Concurrency**: ER 17, 23 (async). perf-book Parallelism.
- **Error handling**: ER 4, 7, 18 (panic). Entire section 06 here.
- **Macros**: section at end; TLBORM entire book.
- **Tooling**: ER dependencies + tooling items. perf-book profiling.

---

## Code-snippet library (copy-paste for LLM prompting)

### Bench harness template (Criterion)

```rust
// benches/main.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort");
    for size in [100, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            let mut v: Vec<u32> = (0..n as u32).rev().collect();
            b.iter(|| {
                let mut copy = black_box(v.clone());
                copy.sort();
                black_box(copy);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_sort);
criterion_main!(benches);
```

### Lint config baseline

```toml
# Cargo.toml (workspace root)
[workspace.lints.rust]
unsafe_code = "warn"
unused_must_use = "deny"
nonstandard_style = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
# opt-outs
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
```

### Release profile template

```toml
[profile.release]
opt-level = 3
codegen-units = 1
lto = "fat"
panic = "abort"
strip = "symbols"
debug = "line-tables-only"
```

### .cargo/config.toml for faster linking

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold", "-C", "target-cpu=x86-64-v3"]

[target.x86_64-apple-darwin]
rustflags = ["-C", "target-cpu=native"]
```

### Newtype + From / Display

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct UserId(u64);

impl UserId {
    pub const fn new(n: u64) -> Self { Self(n) }
    pub const fn get(self) -> u64 { self.0 }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "uid:{}", self.0)
    }
}

impl From<UserId> for u64 { fn from(x: UserId) -> u64 { x.0 } }
```

### Typestate builder

```rust
use std::marker::PhantomData;

pub struct Missing;
pub struct Set<T>(T);

pub struct ReqBuilder<U = Missing, M = Missing> {
    url: U,
    method: M,
}
impl ReqBuilder { pub fn new() -> Self { Self { url: Missing, method: Missing } } }
impl<M> ReqBuilder<Missing, M> {
    pub fn url(self, u: impl Into<String>) -> ReqBuilder<Set<String>, M> {
        ReqBuilder { url: Set(u.into()), method: self.method }
    }
}
impl<U> ReqBuilder<U, Missing> {
    pub fn method(self, m: &'static str) -> ReqBuilder<U, Set<&'static str>> {
        ReqBuilder { url: self.url, method: Set(m) }
    }
}
impl ReqBuilder<Set<String>, Set<&'static str>> {
    pub fn build(self) -> Req { Req { url: self.url.0, method: self.method.0 } }
}
pub struct Req { url: String, method: &'static str }
```

### Async with structured concurrency

```rust
use tokio::task::JoinSet;

async fn fetch_all(urls: Vec<String>) -> Vec<Result<Body, Error>> {
    let mut set = JoinSet::new();
    for u in urls { set.spawn(fetch_one(u)); }
    let mut out = Vec::new();
    while let Some(res) = set.join_next().await {
        out.push(res.unwrap_or_else(|je| Err(Error::Join(je))));
    }
    out
}
```

### Custom global allocator

```rust
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

### Smallvec for avoiding heap in common case

```rust
use smallvec::{smallvec, SmallVec};

fn tokenize(s: &str) -> SmallVec<[&str; 4]> {
    let mut out: SmallVec<[&str; 4]> = smallvec![];
    out.extend(s.split_whitespace());
    out
}
```

### Declarative macro with internal rules

```rust
#[macro_export]
macro_rules! hash_set {
    (@inner $set:ident ; ) => {};
    (@inner $set:ident ; $head:expr $(, $($tail:tt)*)?) => {
        $set.insert($head);
        hash_set!(@inner $set ; $($($tail)*)?);
    };
    ( $( $x:expr ),* $(,)? ) => {{
        let mut set = ::std::collections::HashSet::new();
        $( set.insert($x); )*
        set
    }};
}
```

### Proc-macro derive skeleton

```rust
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

#[proc_macro_derive(FieldNames)]
pub fn derive_field_names(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (ig, tg, wc) = ast.generics.split_for_impl();

    let fields = match ast.data {
        Data::Struct(s) => match s.fields {
            Fields::Named(f) => f.named.into_iter().map(|f| f.ident.unwrap()).collect::<Vec<_>>(),
            _ => return syn::Error::new_spanned(name, "only named fields").to_compile_error().into(),
        },
        _ => return syn::Error::new_spanned(name, "only structs").to_compile_error().into(),
    };
    let fields_str = fields.iter().map(|i| i.to_string());

    let expanded = quote! {
        impl #ig #name #tg #wc {
            pub fn field_names() -> &'static [&'static str] {
                &[ #(#fields_str),* ]
            }
        }
    };
    expanded.into()
}
```

### Attribute macro skeleton

```rust
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn timed(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let vis = &input.vis;
    let sig = &input.sig;
    let name = &sig.ident;
    let name_str = name.to_string();
    let block = &input.block;

    quote! {
        #vis #sig {
            let __start = ::std::time::Instant::now();
            let __out = (|| #block)();
            ::tracing::info!(fn_name = #name_str, elapsed_us = __start.elapsed().as_micros() as u64, "timed");
            __out
        }
    }.into()
}
```

---

## Quick-reference cheat sheet (one-liners for LLM context)

- `Arc<T>` shares across threads; `Rc<T>` same-thread; `Cell<T>` interior mut for Copy; `RefCell<T>` runtime-borrow-check; `Mutex<T>` thread-safe interior mut.
- `impl Trait` in args = generic; in return = opaque type.
- `dyn Trait` = dynamic dispatch via vtable; object-safe traits only.
- `#[non_exhaustive]` keeps enums / structs future-proof.
- `#[inline]` signals cross-crate inlining; `#[cold]` marks unlikely.
- `cargo clippy -- -D warnings -W clippy::pedantic` for a rigorous audit.
- Release: LTO + codegen-units=1 + panic=abort + strip for smallest/fastest.
- `LazyLock` (1.80) replaces `lazy_static!` / `once_cell::sync::Lazy`.
- `let Some(x) = opt else { return };` (1.65) — early return on None.
- `if let ... && let ...` let-chains (1.88) compose multiple conditions.
- `#[derive(thiserror::Error)]` for typed lib errors; `anyhow::Error` + `.context()` for apps.
- `SmallVec<[T; N]>` avoids heap for common small cases.
- `FxHashMap` / `AHashMap` faster than default SipHash when DoS not a concern.
- `rayon::par_iter()` drops in for data parallelism.
- `tokio::task::JoinSet` for structured spawn / join.
- `tokio::pin!(fut)` pins on stack; `Box::pin(fut)` heap.
- `.iter().map().collect::<Result<Vec<_>, _>>()` short-circuits on first error.
- `BufReader` / `BufWriter` always for file I/O; call `flush()` before drop.
- `cargo-flamegraph` for CPU; `heaptrack` / `dhat-rs` for allocations.
- `criterion` for benches; `divan` newer alternative; `iai-callgrind` for deterministic CI benches.
- `$crate::` in exported macros for portable paths; `::std::vec::Vec` for hygienic type references.
- `syn` + `quote` + `proc-macro2` = standard proc-macro toolchain.
- `trybuild` tests expected compile failures with diagnostics.
- Rust 2024 edition: `static mut` requires `unsafe`; new `Future`/`IntoFuture` in prelude; RPIT captures all in-scope.
- `&raw const / mut EXPR` (1.82) creates raw pointers without reference temporaries.
- `use<'a, T>` (1.82) narrows `impl Trait` lifetime / generic capture.

---

## Supplementary idioms from Effective Rust

### ER Item 21 — Understand what cargo does on your behalf

- `cargo build` = compile + link. `cargo check` = type-check only. `cargo test` = build + run tests. `cargo doc` = rustdoc. `cargo bench` = benchmarking.
- `cargo +nightly` uses nightly toolchain; `rust-toolchain.toml` pins.
- `cargo tree -i $pkg` shows who depends on it; `cargo tree -d` shows duplicates.
- Overrides: `[patch.crates-io] foo = { path = "../foo" }` for local development.

### ER Item 22 — Minimise visibility

- Default private; `pub` opt-in. Use `pub(crate)`, `pub(super)`, `pub(in path)` to expose gradually.
- Prefer re-exports over deeply nested pub paths: `pub use self::inner::Thing;`.

### ER Item 23 — Avoid wildcard imports

- `use foo::*;` hides which names come from which module; harms IDE navigation and churn.
- Exceptions: preludes (`use tokio::prelude::*;`), test modules.

### ER Item 24 — Re-export dependencies whose types appear in your API

```rust
// If your API returns `bytes::Bytes`, re-export:
pub use bytes;
```

- Prevents downstream from having to separately depend on matching version.

### ER Item 25 — Manage your dependency graph

- `cargo tree --duplicates` to find diamond dependency skew.
- Pin indirect deps via `[patch.crates-io]` or direct dep declaration.
- Use `cargo update -p foo` sparingly.

### ER Item 26 — Be wary of feature creep

- Every feature flag doubles the build matrix. Keep them orthogonal.

### ER Item 27 — Document public interfaces

- Each pub fn / type: `///` summary, `# Examples`, `# Errors`, `# Panics`, `# Safety` (for unsafe), `# Performance`.
- Run `cargo test --doc` to keep examples compiling.

### ER Item 28 — Use macros judiciously

- Prefer: functions > generic functions > declarative macros > proc macros.
- Macros obscure go-to-definition, complicate IDE support, slow compile.
- Use macros when there's no alternative (DSL, variadic, trait implementation over closed set).

### ER Item 29 — Listen to the Compiler

- `rustc` / Clippy diagnostics often suggest the fix inline. Apply via `cargo fix` / editor action.
- "Consider borrowing here" → usually right.
- "Cannot borrow as mutable" → reorganise, don't `clone()`.

### ER Item 30 — Write more than unit tests

- Integration tests for public API surface.
- Property tests (proptest / quickcheck) for commutativity, idempotency.
- Fuzz tests for parsers and untrusted input.
- Doc tests validate examples.
- Snapshot tests (insta) for serialized outputs.

### ER Item 31 — Take care with build scripts

- `build.rs` runs at build time; each dependency change triggers re-run.
- Use `cargo:rerun-if-changed=path` / `cargo:rerun-if-env-changed=VAR` to limit re-runs.
- Avoid network calls; determinism is important.
- Prefer `vergen` / `built` crates for standard info (git hash, build time).

### ER Item 32 — Control what crosses the FFI boundary

- `extern "C"` with `#[repr(C)]` structs.
- `*const T` / `*mut T` not `&T` / `&mut T` (no guaranteed layout on `&`).
- `CString` / `CStr` for NUL-terminated strings; never `String`.
- Panics cross FFI = UB unless you use `#[unwind(...)]` annotations and matching ABI.
- `bindgen` generates Rust from C; `cbindgen` generates C from Rust.

### ER Item 33 — Prefer `async`/`await` over manual `Future` implementation

- Writing `Future::poll` by hand is easy to get wrong (missing wake, accidental re-entrancy).
- When you must: use `std::task::ready!(cx.poll())` to propagate Pending.
- Consider `futures::future::poll_fn(|cx| ...)` for small ad-hoc futures.

### ER Item 34 — Beware of reflection / dynamic downcasts

- `Any::downcast_ref::<T>()` works on concrete types only; does not introspect fields.
- Used for plugin systems, error chains; otherwise rare.

### ER Item 35 — Prefer composition over inheritance

- Rust has no inheritance. Embed types, forward methods (via `Deref` sparingly or explicit delegation).
- Traits provide shared behaviour; avoid "god traits" — split responsibilities.

---

## More perf-book topics

### Inlining and abstraction cost

- Closures implemented as unique nameless types implementing `Fn*`; dispatched statically unless boxed.
- `Box<dyn Fn>` introduces vtable lookup per call; `Arc<dyn Fn + Send + Sync>` same.
- Sometimes a `fn` pointer is cheaper than `Box<dyn Fn>` (no alloc, smaller size, still one indirect call).

### Debug-profile pitfalls

- `opt-level = 0` on dev; `cargo bench` uses release by default.
- `debug_assert!` elided in release.
- `cfg(debug_assertions)` gate to elide expensive invariant checks in release.

### Const-generics perf

- `const N: usize` generics let you stack-allocate fixed-size arrays where previously you'd need heap.
- `[T; N]` and `[T; N]`-parameterised types may specialise / inline better than `Vec<T>`.

### Branching

- Sort hot branches first. Use `match` over `if let` chains for compiler-selected jump tables.
- `#[likely]` / `#[unlikely]` via `std::intrinsics::likely` (nightly) or `cold_path()` / `hot_path()` idioms.

### Layout optimisations

- `#[repr(C)]` stable layout (FFI).
- `#[repr(packed)]` removes padding; every field access becomes potentially unaligned; often slower or UB.
- `#[repr(align(N))]` align whole type to N.
- `#[repr(transparent)]` single-field wrapper; ABI identical.

### Allocation patterns

- Object pooling: `object_pool`, `sharded-slab`.
- Slab allocation: `slab` crate for dense index-keyed storage.
- Bump allocator: `bumpalo` for arenas (compilers, AST trees).
- Thread-local arenas: `typed-arena::Arena` (single-thread).

### Performance mindset

- Cache misses dominate modern perf: data layout > algorithm at small N.
- Favour arrays-of-structs-of-arrays (SoA) over arrays-of-structs (AoS) when iterating one field.
- `petgraph::CSR` (compressed sparse row) for graph traversal.

### Compile-time tricks

- `const fn` for pre-computed tables.
- `include_bytes!` / `include_str!` for static assets.
- `build.rs` + `OUT_DIR` for generated lookups.

---

## More declarative macro pitfalls (TLBORM)

### Fragment Interactions

- A matcher `$e:expr $d:tt` may appear to work but `$e:expr` is greedy and will swallow up to the follow-set boundary.
- `$($x:pat),+` pre-2021: `pat` allows `|`; may conflict with outer `|`. Since 2021: `pat_param` excludes `|`.
- Nested macros: inside `macro_rules!`, another `macro_rules!` uses `$$` for outer `$`-references if needed (unstable).

### Recursive limits

- Default `recursion_limit = 128`. Raise via `#![recursion_limit = "1024"]`.
- Prefer tree-recursive / binary-recursive counters over linear TT-muncher when counting many tokens.

### Hygiene with paths

- `macro_rules!` paths are *not* hygienic. A macro that emits `Vec::new()` will call whatever `Vec` is in scope at the call site.
- Use `::std::vec::Vec::new()` or `$crate::inner::Helper`.

### `$crate` mechanics

- Exported macros (`#[macro_export]`) substitute `$crate` with the absolute path to the defining crate (e.g., `::my_crate`).
- Inside non-exported macros, `$crate` refers to the current crate — useful for making the macro insensitive to module nesting.

### Exporting with re-exports

```rust
// lib.rs
mod macros;
pub use crate::macros::*;  // re-export definitions if needed
#[doc(inline)]
pub use my_macro;  // bring macro to crate root
```

### Interaction with editions

- Edition 2018+: macros are imported via `use` paths.
- Edition 2015: `#[macro_use]` on `extern crate`.
- Edition 2021+: `pat` → `pat_param` / `pat` distinction.

### Variable-length arg macros (printf-style)

```rust
macro_rules! dbg_all {
    ( $( $x:expr ),* $(,)? ) => {{
        $( eprintln!("{} = {:?}", stringify!($x), $x); )*
    }};
}
```

### Token tree opaqueness

- `tt` captures one balanced token tree (a single token, or `(...)`, `[...]`, `{...}`).
- Once captured, you can't parse its interior with `macro_rules!` — it's a black box. You can forward it to another macro that parses its own fragments.

### Conditional compilation inside macros

- `#[cfg(feature = "x")]` inside macro body: the gate is applied to the expanded tokens, not to macro rules.
- Use `#[cfg_attr(feature = "x", derive(Debug))]` style for attribute conditional.

### Macro 1.0 vs Macro 2.0

- `macro_rules!` = Macro 1.0; still the stable path.
- `macro name` (future Macro 2.0): fully hygienic, path-scoped; unstable. Likely years away from full stability.

---

## More proc-macro depth

### `syn` version pinning

- `syn 2.x` (current) has stricter parsing and new types — upgrade paths from 1.x require noticeable work.
- `syn = { version = "2", features = ["full", "extra-traits"] }` — "full" parses full item syntax; "extra-traits" adds `Debug`, `PartialEq` for AST types (vital for writing tests).

### `quote` tricks

- `quote! { fn #name() {} }` — interpolate one `Ident`.
- `quote! { #(#items)* }` — repeat a `Vec<T>` where `T: ToTokens`.
- `quote! { #(#pairs),* }` — separator-joined.
- `quote_spanned! { ident.span() => ... }` — custom spans for errors.
- `TokenStream::extend(...)` — build tokens procedurally when `quote!` DSL isn't flexible enough.

### Debug expansions

- Write expansion to file: use `syn::__private::print_punct`, `cargo-expand`, or `eprintln!("{}", ts)` inside the macro (visible during cargo build).

### Crates for robust proc macros

- `proc-macro-error2`: better error reporting (replaces abandoned `proc-macro-error`).
- `manyhow`: anyhow-style error handling in proc macros.
- `darling`: declarative input parsing — strongly typed AST for `#[my_attr(...)]` attributes.
  ```rust
  #[derive(FromDeriveInput)]
  #[darling(attributes(my))]
  struct Opts { name: Option<String>, skip: bool }
  ```
- `synstructure`: simplifies writing derive macros that iterate over variants / fields.
- `prettyplease`: format `TokenStream` for snapshot testing.

### Hygiene spans in detail

- `Span::call_site()`: emitted tokens behave as if written at the call site. Most `quote!` output uses this by default.
- `Span::mixed_site()`: new identifiers are hygienic (local to the macro); existing paths resolve at call site. Closer to `macro_rules!` semantics.
- `Span::def_site()` (nightly): tokens behave as if written at definition site — full hygiene.
- Use `Span::mixed_site()` for macro-introduced temp variables that shouldn't shadow caller's names.

### Proc-macro pitfalls

- Expansion must be valid Rust at *every* use site; test diverse inputs.
- Generic bounds: `impl_generics` carries where-clauses; never hand-roll.
- Attributes on individual fields: iterate `DataStruct::fields.iter()` and parse `field.attrs`.
- Handling lifetime parameters: `split_for_impl` does this for you.
- Enum variants: each variant has own `fields` (`Fields::Named`, `Fields::Unnamed`, `Fields::Unit`).

### Example: custom derive that handles generics correctly

```rust
#[proc_macro_derive(Named)]
pub fn named(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let name = &ast.ident;
    let (ig, tg, wc) = ast.generics.split_for_impl();

    quote! {
        impl #ig ::my_crate::Named for #name #tg #wc {
            fn name(&self) -> &'static str { stringify!(#name) }
        }
    }.into()
}
```

### Performance of proc macros

- `cargo build --timings` shows per-crate times; watch for slow expansions.
- Share a proc-macro crate across a workspace — linker cost amortised.
- Prefer compile-time computation via `const fn` + traits over proc-macro expansion when feasible.

### Testing snapshots with `trybuild`

```rust
// tests/compile_fail.rs
#[test] fn ui() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
```

- Each `.rs` has a sibling `.stderr` snapshot; run `TRYBUILD=overwrite cargo test` to update.

---

## Topical deep-dives

### Atomics and memory ordering (not in Effective Rust, but adjacent)

- `std::sync::atomic::{AtomicUsize, AtomicBool, AtomicPtr, ...}`.
- Orderings (weakest to strongest): `Relaxed`, `Release` / `Acquire`, `AcqRel`, `SeqCst`.
- Lock-free counter: `Relaxed` fetch_add.
- Publish-subscribe: writer uses `Release`; reader uses `Acquire`.
- Strongest and simplest: `SeqCst`; use if unsure, but it's slower.

### UnsafeCell

- `UnsafeCell<T>` is the *only* way to get interior mutability legally; `Cell` / `RefCell` / `Mutex` all wrap it.
- Rust assumes `&T` → no mutation; `UnsafeCell` tells the compiler to disable that optimisation for this value.

### `Pin` mechanics

- `Pin<P>` promises that the pointee will not move until dropped (excluding `Unpin` types).
- `Box::pin(v)` — stable alloc-based pin.
- `std::pin::pin!(v)` — stack pin, no alloc (1.68+).
- `tokio::pin!(v)` — older equivalent in the tokio runtime.
- You pin a future before polling.
- `Pin::new_unchecked` — unsafe; used when implementing pin projections manually. Use `pin-project` / `pin-project-lite` crates.

### `async fn` desugaring

```rust
// Original:
async fn foo(x: u32) -> u32 { bar(x).await + 1 }

// Conceptually becomes:
fn foo(x: u32) -> impl Future<Output = u32> {
    async move { bar(x).await + 1 }
}

// Which is a state-machine struct with `impl Future`
```

- Locals live in the future struct; each `.await` is a state.
- `Send`-ness is determined by whether all held locals across `.await` are `Send`.

### `#[must_use]`

- On types: using the type without consuming triggers a warning.
- On functions: discarding the return triggers a warning.
- Idiomatic on `Result`, builders, iterators, locks (guards).

### `#[track_caller]`

- On a function: the `panic!()` location reported is the caller's, not the function itself.
- Use on helper functions like `assert_eq!`, unwrap wrappers.

### `#[must_use]` + `#[track_caller]` combo on unwrap wrappers

```rust
#[track_caller]
pub fn unwrap_or_bug<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    r.expect("internal invariant")
}
```

### Error ergonomics

- `anyhow::Result<T>` in binaries / scripts.
- `thiserror` + enum in libraries.
- Both interop: `anyhow::Error: From<E>` for any `E: Error + Send + Sync + 'static`.
- Add context: `reqwest::get(url).with_context(|| format!("fetching {url}"))`.
- Display error chain: `format!("{err:#}")` with anyhow.

### Small-object optimisation

- `SmallVec<[T; 4]>` — up to 4 on stack.
- `smartstring::SmartString` / `CompactString` — up to ~23 bytes on stack.
- `tinyvec`: all-safe smallvec alt.

### `From<String> for Cow<'static, str>`

```rust
fn f(s: impl Into<Cow<'static, str>>) { ... }
```

- Accepts `&'static str`, `String`, `Cow<_, _>` transparently.

### Reading large files

- `std::io::BufReader::new(File::open(p)?)` — buffered. Use `.lines()` for line iteration.
- `memmap2::Mmap::map(&file)?` → `&[u8]`. Zero-copy; OS-paged. Best for random access.

### Writing async-safe locks

```rust
// Prefer:
let guard = tokio::sync::Mutex::lock(&m).await;
some_async_op(&*guard).await;

// Avoid std::sync::Mutex across await:
// let g = std::sync::Mutex::lock(&m).unwrap();
// some_async_op(&*g).await;  // !Send; blocks runtime
```

---

## Closing cross-topic patterns

### "Make illegal states unrepresentable" checklist

- Sum types for mutually-exclusive states.
- Phantom types for compile-time state tracking.
- NonZeroU32 / NonZeroI64 etc. for "non-zero" invariants.
- `#[non_exhaustive]` to prevent exhaustive match at the boundary.
- Newtypes for unit-carrying values (milliseconds vs microseconds).
- `Result<T, Infallible>` for types that can't fail.

### "Zero-cost" sanity checks

- Measure before optimising: `cargo bench`, `cargo flamegraph`.
- Any optimisation adding complexity without a ≥ 10% observed improvement is suspect.
- Prefer wider data types and iterator chains for LLVM auto-vectorisation.

### "Own less" principle

- Borrow over clone; `&T` over `Arc<T>` until you cross a thread.
- `Cow<'a, T>` as a zero-cost fallback.
- Lifetimes on structs are fine for small-scope struct types; avoid for long-lived, serialised types.

### "Batch and reuse" principle

- Reuse `String` / `Vec` buffers across iterations (`buf.clear()`).
- Pre-allocate capacities whenever size is predictable.
- Combine I/O into batches (`write_all` a large buffer vs many small `write`).

### "Use the type system first, runtime checks second" principle

- Typed builders > runtime assertions.
- `NonEmpty<T>` newtype > "check vec is non-empty at call".
- `struct Positive(u64)` > `fn positive_check(n: u64) -> Result<u64, _>`.

---

## End notes / gaps vs source material

- Book references covered by internalised knowledge; absolute fidelity to individual item numbering may shift over time as the authors renumber.
- Several crate versions (syn 2.x, serde 1.x, anyhow 1.x, tokio 1.x) are current as of April 2026 with minor revisions; the patterns above are version-stable.
- New Rust stabilisations arrive regularly — cross-check against current `releases.rs` before committing to a feature.
- For proc-macro development, the combo `syn 2 + quote 1 + proc-macro2 1 + darling 0.20 + trybuild 1` is the de-facto standard.

---

## Appendix A — Complete Effective Rust item ledger

> ER is organised into six chapters: "Types", "Concepts", "Dependencies", "Tooling", "Asynchronous Rust", "Beyond Standard Rust". Roughly 35 items total. Below is every item with a compact rule + rationale.

### Chapter 1 — Types

- **Item 1. Use the type system to express your data structures.** Sum types encode mutually exclusive states; avoid `Option<...>` companion fields.
- **Item 2. Use the type system to express common behaviour.** Traits = interfaces; small focused traits beat big "god" traits.
- **Item 3. Prefer `Option` and `Result` transforms over explicit `match`.** Combinators read better and compose; reach for `match` only for multi-arm logic.
- **Item 4. Prefer idiomatic error types.** Enum + `thiserror` in libraries; `anyhow` in apps. Use `#[source]` and `#[from]` to preserve causal chains.
- **Item 5. Understand lifetimes.** Lifetimes are static scope-validity proofs. `'static` = *may* live forever (owned types satisfy it). NLL + two-phase borrows unlock common patterns.
- **Item 6. Avoid matching on `Option` and `Result`.** Revisits Item 3; reinforces combinator-first.
- **Item 7. Use builders for complex types.** Fluent APIs substitute for missing named args; consider typestate for required fields.
- **Item 8. Familiarise yourself with reference types.** `Box`/`Rc`/`Arc`/`Cell`/`RefCell`/`Mutex`/`RwLock`/`Pin`/`Cow` — each serves distinct ownership / mutability needs.
- **Item 9. Consider iterator transforms over explicit loops.** Expresses intent, eliminates bounds checks, auto-vectorises.
- **Item 10. Implement the Drop trait for RAII.** Define cleanup once, let the compiler enforce scope.

### Chapter 2 — Concepts

- **Item 11. Implement the Default trait where sensible.** Required by many APIs; derivable when all fields default.
- **Item 12. Understand the uses of `impl Trait`.** Arg vs return positions; RPITIT; edition 2024 capture semantics.
- **Item 13. Use generics and trait objects for polymorphism.** Compile-time vs runtime; monomorphisation cost vs object safety.
- **Item 14. Use built-in traits to tag types.** `Copy`, `Send`, `Sync`, `Sized`, `Unpin` communicate capability.
- **Item 15. Understand the trait system's coherence rules.** Orphan rule enforces clear ownership of trait impls.
- **Item 16. Avoid writing `unsafe`.** Unsafe unlocks five superpowers; encapsulate behind safe APIs; document safety invariants.
- **Item 17. Be wary of shared-state parallelism.** Prefer message passing; mutex chains are deadlock-prone.
- **Item 18. Don't panic.** Library APIs return `Result`. Panic is for bug detection, not control flow.
- **Item 19. Avoid reflection.** Rust has `Any` but no real introspection; use trait-driven metaprogramming.
- **Item 20. Avoid the temptation to over-optimise.** Measure first; idiomatic code usually fast enough.

### Chapter 3 — Dependencies

- **Item 21. Understand what Cargo does.** Build / check / test / doc / bench; lock files; resolver versions; targeted builds.
- **Item 22. Minimise visibility.** `pub(crate)` by default; broaden deliberately.
- **Item 23. Avoid wildcard imports.** Reserve for preludes / tests.
- **Item 24. Re-export dependencies in your API surface.** Prevents downstream version skew.
- **Item 25. Manage your dependency graph.** Audit duplicates; pin indirect deps; use `cargo-tree` / `cargo-deny` / `cargo-audit`.
- **Item 26. Be wary of feature creep.** Features must be additive and orthogonal.
- **Item 27. Document public interfaces.** Use `# Examples`, `# Errors`, `# Panics`, `# Safety`, `# Performance` sections.

### Chapter 4 — Tooling

- **Item 28. Use macros judiciously.** Functions → generics → decl macros → proc macros. Prefer earliest that works.
- **Item 29. Listen to the compiler.** Diagnostics often suggest the fix; apply via `cargo fix`.
- **Item 30. Write more than unit tests.** Integration, property, fuzz, snapshot, doc tests.
- **Item 31. Take care with build scripts.** Keep deterministic; gate `rerun-if-changed`; avoid network.
- **Item 32. Control what crosses the FFI boundary.** `#[repr(C)]`, raw pointers, nul-terminated strings, catch panics at boundary.

### Chapter 5 — Asynchronous Rust

- **Item 33. Prefer `async`/`await` over manual Future implementation.** Macro-generated state machines are correct; hand-rolled are fragile.
- **Item 34. Share and synchronise data for async Rust.** Know when to use `tokio::sync::`* vs `std::sync::*`; avoid holding `std::sync::MutexGuard` across `.await`.
- **Item 35. Consider letting the compiler infer types.** Explicit type annotations hinder refactoring; use elision where idiomatic.

### Chapter 6 — Beyond Standard Rust (final items)

- **Item 36. Control what crosses FFI boundaries.** (Sometimes numbered 32 depending on edition — ER restructures occasionally.)
- **Item 37. Prefer Rust to script wrappers.** Use `build.rs` for minor tasks; a full Rust crate for non-trivial ones.

*(Precise numbering can drift as the book is updated; the rules above are stable.)*

---

## Appendix B — Expanded perf-book topic inventory

### Type sizes drill-down

```rust
// Problem: enum with one huge variant.
enum Event {
    Tick,                       // tag + 0
    Payload([u8; 1024]),        // tag + 1024
}
// sizeof::<Event>() ≈ 1028 (plus align padding)

// Fix: box the heavy variant
enum EventBoxed {
    Tick,
    Payload(Box<[u8; 1024]>),
}
// sizeof::<EventBoxed>() == 16 (ptr + tag + padding)
```

- Check with `cargo +nightly rustc -- -Zprint-type-sizes > sizes.txt`.
- Hunt for large enums or structs with unused padding.

### Padding-aware field ordering

```rust
// Before: 24 bytes due to alignment
struct Bad {
    a: u8,   // 1
    // 7 padding
    b: u64,  // 8
    c: u8,   // 1
    // 7 padding
}

// Rust auto-reorders unless `#[repr(C)]`; but with repr(C) you pick:
#[repr(C)]
struct Good {
    b: u64,  // 8
    a: u8,   // 1
    c: u8,   // 1
    // 6 padding
}  // 16 bytes
```

### Small-String optimisations

- `smartstring::SmartString<LazyCompact>` — 24B inline, spills to heap.
- `compact_str::CompactString` — similar.
- `smol_str::SmolStr` — interned-style, Arc-backed.
- Use when many short ids / keys.

### Vec vs Box<[T]>

- `Vec<T>` = ptr + len + cap (3 `usize`).
- `Box<[T]>` = ptr + len (2 `usize`).
- After construction, use `v.into_boxed_slice()` to save 8 bytes per slice.

### Capacity management

- `Vec::with_capacity(n)` then `push` N times: 1 alloc.
- Build iteratively then `shrink_to_fit()` trims wasted cap.
- `Vec::try_reserve` (stable) fallible alloc for untrusted sizes.

### Enum discriminant elision (niche optimisation)

- `Option<&T>`, `Option<Box<T>>`, `Option<NonZeroU32>`, `Option<bool>`, `Option<char>` all use the niche.
- Multi-level `Option<Option<NonZeroU32>>` can still elide.
- User types: `NonZeroU8`, `NonZeroUsize`, `NonNull<T>` expose niches.

### Inlining details

- `#[inline]` advisory cross-crate; necessary for generic small helpers.
- `#[inline(always)]` forces; LLVM may refuse if inlining is impossible (recursive / complex).
- `#[inline(never)]` for benchmarks, cold paths, and IR clarity.
- `#[cold]` on error paths hints code placement (moves to end of fn).

### Dead code elimination in benches

- LLVM sees bench bodies returning known constants and folds entire work.
- `std::hint::black_box(x)` is the stable way to stop DCE.
- `criterion::black_box` forwards to `std::hint::black_box`.

### Profile-guided optimisation (PGO)

Steps:

1. `RUSTFLAGS="-Cprofile-generate=/tmp/pgo" cargo build --release`
2. Run representative workload — writes `.profraw` files to `/tmp/pgo`.
3. `llvm-profdata merge -o /tmp/pgo/merged.profdata /tmp/pgo`
4. `RUSTFLAGS="-Cprofile-use=/tmp/pgo/merged.profdata" cargo build --release`

- `cargo-pgo` crate automates.

### BOLT (post-link optimisation)

- Binary-level block reordering, ICache warming.
- `llvm-bolt` with `perf.data`; complementary to PGO.

### Link-Time Optimisation (LTO)

- `lto = "off"` (default dev), `"thin"` (fast), `"fat"` (max). `thin` gets ~80% of `fat` gains in much less time.
- Enables cross-crate inlining.

### Small hash collections

- `indexmap::IndexMap`: preserves insertion order; swap-remove O(1).
- `smallvec::SmallVec<[(K,V); N]>` + linear scan: for tiny maps (≤ 8 entries), beats HashMap on allocs and cache.
- `hashbrown::HashMap`: SIMD-accelerated; the std `HashMap` since 1.36 is hashbrown.

### Lock-free / concurrent collections

- `dashmap::DashMap` — sharded RwLock HashMap, great for read-heavy.
- `flurry::HashMap` — Java-ConcurrentHashMap-style.
- `crossbeam::skiplist::SkipMap` — concurrent ordered.
- `evmap` (eventual-consistency, mostly deprecated).

### Custom hashers

- `BuildHasherDefault<rustc_hash::FxHasher>`: fast, non-cryptographic.
- `ahash::RandomState`: fast + secure-ish.
- `fnv::FnvBuildHasher`: tiny-key specialised.
- Benchmark with your real keys; default often good enough.

### Parallelism patterns

- Fork-join with `rayon::join(f, g)`.
- Pipelines: producer-consumer channels with `crossbeam::channel` (unbounded/bounded).
- Data-parallel: `par_iter().map(...).sum()`.
- Scoped threads: `std::thread::scope(|s| { s.spawn(...); })` — borrowed refs OK.
- Work-queue patterns with `deadqueue` or `async-channel`.

### SIMD and auto-vec

- Compiler auto-vectorises simple loops with contiguous access + no aliasing.
- `target-feature=+avx2` expands what's allowed.
- Manual SIMD via `std::simd` (unstable) or `core::arch` intrinsics.
- Validate with `cargo-show-asm --rust` and look for `%ymm` / `%zmm` registers.

### Binary size reduction

- `strip` in release profile.
- `opt-level = "z"` (size) or `"s"` (balanced size/speed).
- `panic = "abort"` eliminates unwind landing pads.
- `codegen-units = 1` + `lto = "fat"` improves dead-code removal.
- `xz`-packed binaries with `upx`.
- `build-std` (nightly) rebuilds `std` with your flags.

### No-std and embedded

- `#![no_std]` at crate root; use `core` instead of `std`.
- `#![no_main]` for embedded entry points.
- Heap (`alloc` crate) optional via `#![feature(alloc)]` (or stable with `alloc` crate).
- `panic-halt` / `panic-semihosting` crates provide `#[panic_handler]`.
- `defmt` for efficient logging over RTT/serial.

### Heap profiling in detail

- `**dhat-rs**`: in-process tracking; drop-in for one-run profiling.
  ```rust
  #[global_allocator]
  static ALLOC: dhat::Alloc = dhat::Alloc;

  fn main() {
      let _profiler = dhat::Profiler::new_heap();
      run_workload();
  }
  ```
- **heaptrack** (external): records alloc stacks; `heaptrack_gui` to inspect.
- **valgrind --tool=dhat**: deterministic; slow; thorough.
- **jemalloc + `MALLOC_CONF=prof:true`**: production-safe profiling.

### Continuous regression detection

- `criterion` `--save-baseline` + `--baseline` for local diffing.
- `codspeed.io` integrates Criterion with CI, pins to cpuset.
- `bencher.dev` similar.
- `iai-callgrind` gives cycle-accurate counts (no wall-clock noise) — good for flaky CI environments.

### Compile-time wins worth knowing

- `cargo check` (not `build`) during edit-loop.
- `cargo build --profile dev-opt` (define a custom `dev-opt` profile for mid-opt dev builds).
- `CARGO_TARGET_DIR=/nvme/target` — put target on fast disk.
- `sccache` for cross-machine rebuild caching (CI).
- Split binary into lib + bin — the lib builds once, bin relinks.
- `thin` LTO in release: 4× faster link than `fat`, ~90% of gains.

---

## Appendix C — Macro cookbook (TLBORM patterns)

### Repetition over key-value pairs

```rust
macro_rules! config {
    ( $( $k:ident : $v:expr ),* $(,)? ) => {{
        let mut m = ::std::collections::HashMap::new();
        $( m.insert(stringify!($k).to_string(), $v); )*
        m
    }};
}
// usage: config! { host: "127.0.0.1", port: 8080 }
```

### Optional arguments via `$()?`

```rust
macro_rules! log {
    ($lvl:ident, $msg:expr $(, $($arg:tt)*)?) => {
        ::tracing::$lvl!($msg $(, $($arg)*)?)
    };
}
// log!(info, "hello");
// log!(info, "x = {}", 1);
```

### Nested repetitions

```rust
macro_rules! nested {
    ( $( $outer:ident : [ $( $inner:expr ),* ] ),* $(,)? ) => {{
        vec![ $( ($outer, vec![ $( $inner ),* ]) ),* ]
    }};
}
```

### Converting between fragment classes

```rust
// Re-parse an `expr` fragment as a `path` in some cases (rare).
macro_rules! as_expr { ($e:expr) => { $e } }
macro_rules! as_path { ($p:path) => { $p } }
```

### Counting via recursion

```rust
macro_rules! count {
    () => { 0usize };
    ($_head:expr $(, $tail:expr)*) => { 1 + count!($($tail),*) };
}
// const LEN: usize = count!(a, b, c);  // 3
```

### Counting without recursion (slice trick)

```rust
macro_rules! count_slice {
    ( $( $item:expr ),* $(,)? ) => {
        <[()]>::len(&[ $( { let _ = $item; () } ),* ])
    };
}
```

### Early-return macro (match-like)

```rust
macro_rules! unwrap_or_return {
    ($opt:expr, $ret:expr) => {
        match $opt {
            Some(v) => v,
            None => return $ret,
        }
    };
}
```

### Cleaner assertion with track_caller

```rust
#[macro_export]
macro_rules! assert_eq_tol {
    ($a:expr, $b:expr, $tol:expr $(,)?) => {{
        let a = $a;
        let b = $b;
        let tol = $tol;
        if (a - b).abs() > tol {
            panic!("|{a} - {b}| > {tol}");
        }
    }};
}
```

### DSL-style macro

```rust
macro_rules! html {
    ( $tag:ident { $($inner:tt)* } ) => {{
        format!("<{0}>{1}</{0}>", stringify!($tag), html!(@children $($inner)*))
    }};
    (@children) => { String::new() };
    (@children $head:tt $($tail:tt)*) => {{
        let mut s = html!(@one $head);
        s.push_str(&html!(@children $($tail)*));
        s
    }};
    (@one $text:literal) => { $text.to_string() };
    (@one $tag:ident { $($inner:tt)* }) => { html!($tag { $($inner)* }) };
}
```

### `matches!` macro (built-in since 1.42)

```rust
let ok = matches!(val, Some(x) if x > 0);
```

### `pin_mut!` and `pin!`

- `std::pin::pin!(x)` — stack-pin (1.68+).
- `tokio::pin!(x)` — older equivalent.
- `futures::pin_mut!(x)` — yet another; drop when migrating.

---

## Appendix D — Effective Rust anti-example gallery

### Anti: Over-specific lifetimes

```rust
// ❌ Tied inputs to outputs unnecessarily
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { ... }

// ✓ Independent lifetimes
fn longest<'a, 'b: 'a>(x: &'a str, y: &'b str) -> &'a str { ... }
```

### Anti: Unnecessary `'static`

```rust
// ❌ Forces callers to pass owned or leaked data
fn f(s: &'static str) { ... }

// ✓ Accepts any borrow
fn f(s: &str) { ... }
```

### Anti: `String` argument when `&str` suffices

```rust
// ❌ Forces caller to allocate
fn greet(name: String) { println!("{}", name); }

// ✓
fn greet(name: &str) { println!("{}", name); }
```

### Anti: `Vec<u8>` when `&[u8]` works

```rust
// ❌
fn process(data: Vec<u8>) { ... }

// ✓
fn process(data: &[u8]) { ... }
```

### Anti: `Box<dyn Trait>` when generic suffices

```rust
// ❌ Heap alloc + vtable on each call
fn apply(f: Box<dyn Fn(i32) -> i32>) { ... }

// ✓ Monomorphised, zero dispatch
fn apply(f: impl Fn(i32) -> i32) { ... }
```

### Anti: Mutex guard across await

```rust
// ❌ Holds std::sync::MutexGuard across .await; marks future !Send;
// also blocks the runtime thread.
async fn bad(m: &std::sync::Mutex<T>) {
    let g = m.lock().unwrap();
    some_async_op(&*g).await;
}

// ✓ Drop guard before await, or use async mutex
async fn good(m: &tokio::sync::Mutex<T>) {
    let g = m.lock().await;
    some_async_op(&*g).await;
}
```

### Anti: `clone()` to satisfy borrow checker

```rust
// ❌ Solves symptom, hides structural issue
for key in map.keys().cloned().collect::<Vec<_>>() {
    if map.get(&key).unwrap() == ... { map.remove(&key); }
}

// ✓ Use retain
map.retain(|_, v| v != &target);
```

### Anti: Nested `Option<Option<T>>`

```rust
// ❌ Ambiguous meaning
fn find() -> Option<Option<Config>> { ... }

// ✓ Three-state enum
enum FindResult { Absent, Present(Option<Config>) }
```

### Anti: Panic on flag parsing

```rust
// ❌ Library
fn parse_port(s: &str) -> u16 { s.parse().unwrap() }

// ✓
fn parse_port(s: &str) -> Result<u16, std::num::ParseIntError> { s.parse() }
```

---

## Appendix E — Perf-book benchmarking playbook

### Canonical Criterion layout

```
my_crate/
├── Cargo.toml           # [dev-dependencies] criterion = "0.5"
├── src/lib.rs
├── benches/
│   ├── main.rs          # criterion_main! entry
│   └── algorithms.rs    # grouped benches per module
```

```toml
[[bench]]
name = "main"
harness = false   # disable default libtest harness
```

### Criterion configuration

```rust
fn custom_cfg() -> Criterion {
    Criterion::default()
        .sample_size(100)
        .measurement_time(std::time::Duration::from_secs(10))
        .warm_up_time(std::time::Duration::from_secs(3))
        .noise_threshold(0.02)
}
criterion_group! { name = benches; config = custom_cfg(); targets = bench_fn }
```

### Compare alternatives

```rust
c.bench_function("hash_fx", |b| b.iter(|| { let m = FxHashMap::<i64,i64>::default(); /*...*/ black_box(m); }));
c.bench_function("hash_default", |b| b.iter(|| { let m = HashMap::<i64,i64>::new(); /*...*/ black_box(m); }));
```

### Throughput measurement

```rust
group.throughput(criterion::Throughput::Bytes(buf.len() as u64));
group.bench_function("parse", |b| b.iter(|| parser::parse(black_box(&buf))));
```

### Async benches

```rust
c.bench_function("async_op", |b| {
    let rt = tokio::runtime::Runtime::new().unwrap();
    b.to_async(&rt).iter(|| async { do_work().await });
});
```

---

## Appendix F — CI perf gates

- Criterion + `cargo bench -- --save-baseline main` in main branch CI; `--baseline main` on PR branch.
- `codspeed-criterion-compat` integrates with CodSpeed for isolated benchmarks.
- `iai-callgrind` runs in Docker with reproducible instruction counts — good for slow-budget CIs.
- Regression threshold: ≥ 3% → alert; often 5% to absorb noise.

---

## Appendix G — Proc-macro anatomy diagram (textual)

```
user code with `#[derive(Foo)]`
    │
    ▼
rustc sees `derive(Foo)` attribute
    │
    ▼
invokes `foo_derive` proc-macro crate
    │
    ▼
proc-macro fn receives `TokenStream` (proc_macro::)
    │                         │
    ▼                         ▼
    parse via syn        (optional: parse attrs via darling)
    │
    ▼
    syn::DeriveInput { ident, generics, data, attrs, vis }
    │
    ▼
    logic (pure fn on AST)  ← testable in normal crate
    │
    ▼
    emit with quote! → proc_macro2::TokenStream
    │
    ▼
    .into() → proc_macro::TokenStream
    │
    ▼
rustc injects expanded tokens as siblings of the annotated item
```

---

## Appendix H — Miscellaneous items not otherwise shown

### `#[repr(...)]` for performance

- `#[repr(C)]` — stable layout, FFI; same field order.
- `#[repr(transparent)]` — single-field struct; same ABI as inner; pass as `T` to FFI receiving `Inner`.
- `#[repr(packed(1))]` — forbids padding; most field accesses then unaligned → slower; deref risks UB. Rare.
- `#[repr(align(N))]` — increase alignment; help with cacheline isolation / SIMD.

### Cacheline isolation

```rust
#[repr(align(64))]
struct PaddedAtomic(AtomicUsize);
```

- Avoids false sharing between threads hammering adjacent counters.

### `core::sync::atomic::compiler_fence`

- Barrier only for compiler reordering; doesn't emit CPU fences.
- Use `std::sync::atomic::fence(Ordering::SeqCst)` for full barrier.

### `std::hint`

- `black_box(x)` — stop DCE (bench-critical).
- `spin_loop()` — CPU hint; shorter backoff than OS sleep.
- `unreachable_unchecked()` — unsafe; strong invariant.

### `once_cell` vs `std::sync::OnceLock` vs `std::sync::LazyLock`

- Pre-1.70: use `once_cell::sync::OnceCell`, `once_cell::sync::Lazy`.
- 1.70+: `std::sync::OnceLock<T>` — replaces OnceCell.
- 1.80+: `std::sync::LazyLock<T>` — replaces `Lazy`; `LazyLock::new(|| ...)`.
- `lazy_static!` macro is now obsolete; it was pre-const-fn workaround.

### `format_args!` and no-alloc formatting

```rust
use std::io::Write;
let mut buf = [0u8; 64];
let n = { let mut w = &mut buf[..]; write!(w, "{}:{}", x, y).unwrap(); 64 - w.len() };
let s: &str = std::str::from_utf8(&buf[..n]).unwrap();
```

### `write!` / `writeln!` into `String`

- `use std::fmt::Write as _;` for `write!` into `String`.
- `use std::io::Write as _;` for `write!` into `Vec<u8>` / `File`.

### `#[must_use]` on Result

- All `Result` is `#[must_use]` by default; ignoring `.send()` etc. triggers warning.

### `tracing` vs `log`

- `log` crate: facade; `env_logger` consumes. Flat lines.
- `tracing`: async-aware spans, structured fields, attachment to tasks.
- Both supported by major libraries via features.

---

## Appendix I — Summary code-recipe index

1. Struct → typestate builder: see `TReq` example.
2. Recursive enum with `Box`: see `enum List { Cons(T, Box<Self>), Nil }` in idioms section.
3. Proc-macro derive skeleton: Appendix G + code block.
4. Attribute macro skeleton: `#[timed]` example.
5. Structured async concurrency: `JoinSet` example.
6. PGO workflow: 4-step block.
7. `Criterion` bench layout: Appendix E.
8. Cacheline-isolated atomics: `#[repr(align(64))]`.
9. Newtype + From + Display: full idiomatic id pattern.
10. Error type with thiserror + source chain: DbError example.

---

## Supplement J — Book-accurate dense notes (Perf Book + ER + TLBORM)

*Use with **Canonical Effective Rust item list** above. Source: `print.html` aggregates from the three sites.*

### 09-performance — The Rust Performance Book (full chapter themes)

**Introduction**

- Audience: intermediate/advanced; beginners should skip (noise vs. learning ROI).
- Breadth over depth; validate every change with measurement.

**Benchmarking**

- Workloads: prefer real inputs; microbenches/stress tests in moderation.
- Harnesses: built-in `bench` (nightly/unstable), **Criterion**, **Divan**, **Hyperfine** (CLI wall-time), **Bencher** (CI continuous benchmarking), custom (e.g. rustc-perf).
- Metrics: wall-time can have high variance (layout randomization); cycles/instructions sometimes stabler. Summarizing across workloads has no single best method.
- Mediocre benchmarking beats none; improve methodology as you learn the program.

**Build configuration — runtime speed**

- `**--release**`: dev vs release often 10–100×; `Finished dev [unoptimized + debuginfo]` vs `Finished release [optimized]`.
- `**codegen-units = 1**`: more optimization, slower compile.
- **LTO**: `lto = false` ⇒ thin *local* LTO (default with opt); `lto = "thin"`; `lto = "fat"` strongest; `lto = "off"` disables cross-crate LTO (fastest compile, worse runtime/size).
- **Allocators**: `#[global_allocator]` + **tikv-jemallocator** (Linux/mac; THP via `MALLOC_CONF` / `_RJEM_MALLOC_CONF`), **mimalloc** (cross-platform).
- `**RUSTFLAGS="-C target-cpu=native"**` or `config.toml` `[build] rustflags` — SIMD/vectorization; compare `rustc --print cfg` vs with native.
- **PGO**: compile → run representative workload → recompile with profile; **cargo-pgo**; not applicable to `cargo install` from crates.io in the usual workflow.

**Build configuration — binary size**

- `opt-level = "z"` (smallest; may slow); `opt-level = "s"` slightly less aggressive, slightly more inlining/vectorization than `"z"`.
- `panic = "abort"` if unwinding unused — smaller/faster slightly.
- `strip = "symbols"` — smaller binary; worse backtraces/profiling. (Rust 1.77+: strip in release profiles by default for local builds per book.)
- See **min-sized-rust** repo for advanced techniques.

**Build configuration — compile time**

- Faster **linker**: **lld** (Linux default since **Rust 1.90** per book); **mold** / **wild** on Linux; macOS system linker already fast. `RUSTFLAGS="-C link-arg=-fuse-ld=lld"` (or mold). No real downside if it works.
- `**[profile.dev] debug = false**` — large dev compile win; use `debug = "line-tables-only"` if you need traces without full DWARF.
- **Nightly parallel front-end**: `RUSTFLAGS="-Zthreads=8"` (often best at 8); up to ~50% compile win, memory cost; no effect on generated code quality.
- **Cranelift** (`-Zcodegen-backend=cranelift`): faster compile, worse runtime — dev builds only.
- **Custom profiles** between dev and release for daily dev when release too slow.

**Summary block (book)**

- Max speed: consider `codegen-units=1`, `lto="fat"`, alt allocator, `panic="abort"`, benchmark each.
- Min size: `opt-level="z"`, same LTO/codegen ideas, strip, etc.
- `target-cpu=native`, PGO when distribution allows.
- Always use faster linker where supported; **cargo-wizard** for guided profiles.

**Linting**

- **Clippy** perf group; rest of perf book omits what Clippy already catches.
- `**disallowed_types**` in `clippy.toml` when banning `HashMap`/`HashSet` after switching hashers.

**Profiling**

- Linux: **perf** + Hotspot / Firefox Profiler; **samply** (cross-platform, Firefox Profiler).
- macOS: Instruments; Windows/Linux: VTune, AMD μProf.
- **flamegraph** crate: perf/DTrace → flame graph.
- Valgrind: **Cachegrind**, **Callgrind**, **DHAT**; **dhat-rs** (cross-platform, instrument code).
- **Iai-Callgrind**: `cargo bench` + Valgrind family.
- **heaptrack**, **bytehound** (Linux heap).
- **counts**: ad-hoc `eprintln!` + frequency post-process.
- **Coz** + **coz-rs**: causal profiling (Linux).

**Profiling — debug info**

- `[profile.release] debug = "line-tables-only"` for symbolized stacks on optimized builds.
- Stdlib lacks debuginfo in distributed builds; full fidelity needs custom toolchain build or **build-std** (limitations on source paths for some tools).

**Profiling — frame pointers / symbols**

- `-C force-frame-pointers=yes` for better stacks.
- **rustfilt** demangling; try **v0** mangling: `-C symbol-mangling-version=v0`.

**Inlining**

- Attributes: none / `#[inline]` / `#[inline(always)]` / `#[inline(never)]` — hints only; `(always)` almost always inlines.
- Non-transitive: callee and caller both need hints if you want a fused blob at a hot callsite.
- Split hot/cold paths: `#[inline(always)]` thin wrapper + `#[inline(never)]` cold shell calling the hot inline.
- **Outlining**: `#[cold]` on rare helper for better hot-path codegen.

**Hashing**

- Default SipHash-1-3: collision-resistant, slower on small keys.
- Alternatives: **rustc-hash** (FxHashMap), **fnv**, **ahash** (AES); measure; **nohash_hasher** when keys are already well-distributed.
- Clippy `disallowed_types` to prevent accidental `std::collections::HashMap` after migration.
- `**#[derive(Hash)]**`: per-field hashing; **zerocopy** / **bytemuck** byte-hash derives can win for packed POD types — measure.

**Heap allocations**

- DHAT: allocation sites, lifetimes, memcpy hot spots; rustc rule-of-thumb: ~10 allocs per million instructions ≈ ~1% win when removed.
- **Box**: simple; box enum variants to shrink outer type.
- **Rc/Arc**: avoid if rarely shared — extra heap + refcount traffic; `clone` is cheap (no alloc).
- **Vec**: growth 0→4→8→…; `with_capacity` / `reserve` when distribution known; **SmallVec**, **ArrayVec** for short fixed-ish lengths; **ThinVec** for often-empty in hot structs.
- **String**: `format!` allocates; `format_args!`, lazy crates; **smartstring** / **smallstr** patterns.
- **clone** / **clone_from** / **to_owned**: `clone_from` reuses capacity; remove stale hot `clone`s after refactors.
- **Cow** for borrowed-or-owned without extra alloc when static/`&str` path exists.
- Reuse **workhorse** `Vec`/`String` across loop iterations (`clear()` retains capacity).
- **BufRead::lines`** allocates per line; **`read_line`into reused`String`** does not.

**Type sizes**

- Types **> 128 bytes** copied with `memcpy` in generated code — shrink hot structs if `memcpy` is hot.
- `**-Zprint-type-sizes**` (nightly) or **top-type-sizes** crate.
- Box large variants; smaller integer indices (`u32` vs `usize`) when range allows; `Vec::into_boxed_slice` to drop capacity word.

**Standard library types (selected)**

- `vec![0; n]` for zero-filled vec — OS may assist.
- `swap_remove` vs `remove` (order vs speed).
- `Option::ok_or_else` not `ok_or` when error is expensive to construct.
- `parking_lot` vs std locks — **measure**; std improved on some platforms.

**Iterators**

- Avoid `collect` then re-iterate; return `impl Iterator` when possible (lifetime blog: Katona).
- `extend` vs collect + `append`.
- Implement `size_hint` / `ExactSizeIterator::len` when possible for smarter `collect`/`extend`.
- `chain` can be slower than one iterator — hot path care.
- `filter_map` often beats `filter`+`map`.
- `chunks_exact` when length divisible; else `chunks_exact` + `remainder`.
- `iter().copied()` for small `Copy` items — can help LLVM.

**Bounds checks**

- Prefer iteration over indexing; slice the `Vec` then index slice; assertions on index ranges; **Bounds Check Cookbook** (Shnatsel).
- Last resort: `get_unchecked` with proof.

**I/O**

- Lock `stdout` once for many `println!`; use `writeln!(lock, ...)`.
- **BufReader** / **BufWriter** for small repeated reads/writes; explicit `flush()` to surface errors.

**Logging / debugging**

- No heavy work when logging disabled; `debug_assert!` vs `assert!` in hot paths.

**Wrapper types**

- Multiple `Arc<Mutex<...>>` fields accessed together → merge into one mutex wrapping a tuple.

**Machine code**

- godbolt.org, **cargo-show-asm**, `core::arch` intrinsics.

**Parallelism**

- **rayon**, **crossbeam**; **Rust Atomics and Locks**; SIMD blog (2025 overview in book pointer).

**General tips**

- Optimize **hot** code only; algorithm/data structure beats micro-ops; cache + branch prediction awareness; many small wins compound; eliminate calls vs speed up callee; lazy computation; fast paths for 0/1/2-sized collections; compress repetitive domains; measure case frequencies; cache in front of hot lookups; comment *why* odd structure (e.g. “99% len ≤ 1”).

**Compile times (code changes)**

- `cargo build --timings` — Gantt chart of crate parallelism.
- `-Zmacro-stats` (nightly) — proc-macro vs `macro_rules!` expansion weight; **cargo-expand** to inspect.
- **cargo llvm-lines** — which generic instantiations explode LLVM IR; fix by shrinking generics or extracting non-generic inner `fn` (pattern from `std::fs::read` in book).
- Replace hot `map`/`map_err` chains with `match` if monomorphisation explodes (small compile win).

---

### 02-language-rules / 06-error-handling — Effective Rust Items 5–8, 10–16 (accurate titles)

**Item 5 — Type conversions**

- Integral narrowing/widening is explicit (`try_into`, `into`); no silent C-style promotion.
- `char` vs `u32` vs UTF-8 bytes are distinct types; conversions expose failure (`Option` / `unsafe`).

**Item 6 — Newtype**

- Zero-cost distinction; use for units, IDs, bool-like enums vs raw bools.

**Item 7 — Builders**

- Three styles: consuming, `&mut self`, typestate; `..Default::default()` + field update; `#[default]` on enum variants (Rust 1.62+ for `Default` on enums with marked variant).

**Item 8 — References and pointers**

- `Deref` coercion chains; `AsRef` in APIs; `Rc`/`Arc` + `make_mut` clone-on-write.

**Item 10 — Standard traits (not Drop)**

- `Clone` vs `Copy` — `Copy` forbids custom `clone()` on copy; marker semantics.
- `PartialEq`/`Eq`/`Hash` agreement; manual `Hash` if manual `Eq`.
- `PartialOrd`/`Ord` coherence with `Eq`; float NaN breaks total order — `PartialOrd` only.
- `Debug` vs `Display`; `derive` gets Debug, not Display.
- Operator traits (`std::ops`) — implement coherent sets; moves consume non-`Copy` operands.

**Item 11 — `Drop` / RAII**

- `drop(&mut self)` only; explicit destructor call forbidden — use `std::mem::drop` for eager drop.
- Resource release cannot propagate errors through `Drop` — use `fn release(self) -> Result` if needed.

**Item 12 — Generics vs trait objects**

- Vtable vs monomorphisation; `dyn Trait` sizing (`Sized` pitfalls); async traits often need `async_trait` or RPIT in trait (edition-dependent).

**Item 13 — Default trait methods**

- Extension without breaking impls; careful semver when adding defaulted methods (name clashes).

**Item 14 — Lifetimes**

- Elision rules (book): one input → output borrows same; multiple inputs + `self` → `self` wins; else name lifetimes explicitly.
- Two equal input lifetimes `'a` on parameters ⇒ output must live within **both** — intersection/subset intuition.
- `'static` for string literals / promoted `const` (exceptions: types with `Drop` or interior mutability may not promote).

**Item 15 — Borrow checker**

- Two-phase borrows; reborrow rules; `MutexGuard` scoped with extra blocks.

**Item 16 — `unsafe**`

- Prefer std/ecosystem wrappers; safety comments; `unsafe_op_in_unsafe_fn`; **Miri** tests.

---

### 07-async-concurrency / 17 — ER Item 17 (shared state) highlights

- `Send`/`Sync` auto-traits; `Rc`/`RefCell` not `Send`/`Sync` as `Arc`/`Mutex` are.
- `Arc<Mutex<T>>` + `spawn` `'static` → clone `Arc` into each thread.
- Mutex poisoning: `lock().unwrap()` common; understand panic while holding lock.
- Deadlocks still possible — lock ordering; try `try_lock`, timeouts, design away from nested locks.

---

### 10-testing-and-tooling — ER Items 27–32 (short)

**Item 27 — Docs**

- `///` + examples that compile (`rustdoc --test`); intra-doc links; README via `readme` key.

**Item 28 — Macros judiciously**

- Not functions: hygiene, control-flow injection (`return`/`?` inside expansion), double evaluation of `$e`.
- Fix double eval: bind `let x = $e;` once; or restrict fragment kind to `ident` if appropriate.
- Prefer `format_args!` for format-like macros.
- Proc-macros separate crate; token low-level — prefer `macro_rules!` when pattern matching suffices.
- `cargo-expand` for debugging; rustfmt limitations inside macro bodies.

**Item 29 — Clippy**

- Deny warnings in CI so new lints surface; pedantic/nursery opt-in per crate.

**Item 30 — Beyond unit tests**

- Integration tests in `tests/`; doc tests; property testing (`proptest`); fuzz (`cargo-fuzz`).

**Item 31 — Tooling ecosystem**

- `rustfmt`, `cargo fmt`; `cargo deny` / `cargo audit`; IDE rust-analyzer.

**Item 32 — CI**

- `cargo clippy -- -D warnings`; `cargo test`; MSRV matrix; caching (`sccache`, Swatinem/rust-cache).

---

### 12-modern-rust / FFI — ER Items 33–35 + compiler versions

**Item 33 — `no_std**`

- `#![no_std]` + `alloc` if heap needed; `core` only for truly embedded; feature-gate std in libraries.

**Item 34 — FFI boundaries**

- Minimize `unsafe` surface; document lifetimes across FFI; align/repr.

**Item 35 — bindgen**

- Auto-bind headers vs hand-written — fewer ABI mistakes.

**Version anchors (from sources in this cluster)**

- **1.77** — release strip defaults (perf book).
- **1.80** — `LazyLock` in std (appendix in file).
- **1.90** — lld default linker Linux (perf book).
- **1.75+** — RPIT in traits / API evolution (Effective Rust discusses; verify project MSRV).

---

### 04-design-patterns / macros — TLBORM mechanics (declarative + proc)

**Syntax extensions & parsing**

- Macro input to `$name!` is **one** non-leaf token tree `(...)`, `[...]`, `{...}` — parser stores opaque TTs; can be non-Rust internally until expanded.
- Invocation positions: expression, statement, pattern, type, item — **not** arbitrary identifier positions or match arms.

**Expansion**

- Result must parse as complete AST fragment for context; **no** “incremental invalid” output — drives **push-down** pattern (accumulate tokens until final emit).
- Recursive expansion until fixed point; **recursion limit** default **128** — `#![recursion_limit = "..."]` crate-wide; hurts compile time.

**Hygiene (`macro_rules!`)**

- Mixed-site: emitted `let` bindings don’t capture caller locals; paths resolve blend of def/call site — use `$crate::path` for crate-root items.

**Metavariable expressions (unstable `macro_metavar_expr`)**

- `${count(ident)}`, `${index()}`, `${len()}`, `${ignore(ident)}`, `$$` for literal `$`.

**Callbacks (TLBORM)**

- Inner macro expansion order prevents “macro A expands to tokens that invoke B” in the naive way — use **callback passing**: `call_with_larch!(recognize_tree)` passes macro *identifier* to dispatch.

**TT muncher costs**

- Matching tail repeatedly ⇒ **O(n²)** macro matching work; prefer many small invocations vs one huge DSL; put frequent rules first; see `quote` crate comment for avoiding quadratic push-down in advanced cases.

**Internal rules**

- `@phase` prefixes; put internal matchers before general `$($tts:tt)*` to avoid bad backtracking; internal rules add failed match attempts — cost.

**Proc-macros**

- `#[proc_macro]` / `#[proc_macro_derive(Name, attributes(...))]` / `#[proc_macro_attribute]`; panic → compiler error; infinite loop hangs rustc.
- Attribute macro: **replace** whole item; derive macro: **append** items; helper attrs must be listed in `proc_macro_derive`.
- **syn** feature-gate heavy parsers; **quote** returns `proc_macro2::TokenStream`.
- **Spans**: `call_site` (unhygienic), `mixed_site`, `def_site` (unstable) — control whether identifiers bind inside/outside macro.

**Macros 2.0 (`macro` / `decl_macro`, nightly)**

- Proper item visibility (`pub`); **definition-site hygiene** — example in book: `macro_rules!` can emit `struct Foo` usable as `Foo::new()` at call site; `macro` definition-site hygiene breaks that unless explicitly addressed; **not stable** as of book chapter.

---

### 05-anti-patterns — quick hits

- **ER Item 18**: `unwrap` in libraries; `panic!` for recoverable errors.
- **ER Item 19**: simulate reflection with `Any` chains — prefer traits.
- **ER Item 20**: change algorithms before `unsafe`; profile first.
- **ER Item 23**: `use crate::*` + new trait impl from dependency → method ambiguity (disambiguate with UFCS).
- **ER Item 24**: expose `dep::TransitiveType` via `pub use` when same crate version must be used at call site.

---

## Supplement K — TLBORM deep cuts (fragment specifiers + metavariable expansion)

*Source: [Fragment Specifiers](https://veykril.github.io/tlborm/decl-macros/minutiae/fragment-specifiers.html), [Metavariables and Expansion Redux](https://veykril.github.io/tlborm/decl-macros/minutiae/metavar-and-expansion.html) (The Little Book of Rust Macros). Rust 1.60+ lists **14** fragment specifiers.*

### 04-design-patterns / 05-anti-patterns — fragment kinds and opacity

- **Opaque capture rule:** matching with anything other than `ident`, `lifetime`, or `tt` stores an **opaque AST blob** — you **cannot** re-match it in a nested macro with finer fragment types. Only **paste** it wholesale into output.
- **The 14 specifiers:** `block`, `expr`, `ident`, `item`, `lifetime`, `literal`, `meta`, `pat`, `pat_param`, `path`, `stmt`, `tt`, `ty`, `vis`.
- `**tt` / `ident` / `lifetime`:** preserve inspectable token structure → enables TT munchers, push-down, callback forwarding.

### Per-fragment notes (dense)


| Spec        | Matches                                                | Pitfall / detail                                                                                                                                                                                                  |
| ----------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `block`     | `{ ... }` block expression                             | —                                                                                                                                                                                                                 |
| `expr`      | any expression                                         | —                                                                                                                                                                                                                 |
| `ident`     | identifier or keyword                                  | `_` is a **pattern**, not `ident`                                                                                                                                                                                 |
| `item`      | any item (incl. `pub use ...`)                         | not a path *to* an item                                                                                                                                                                                           |
| `lifetime`  | `'a`, `'static`, `'`_, labels                          | similar to `ident` + `'`                                                                                                                                                                                          |
| `literal`   | numeric/string/char/bool literals                      | —                                                                                                                                                                                                                 |
| `meta`      | attribute payload (`path`, `path = "lit"`, `path(tt)`) | `///` / `//!` desugar to `doc` attributes — matchable as meta                                                                                                                                                     |
| `pat`       | pattern, **incl. top-level or-patterns** (2021+)       | **cannot** be followed by `|` at top level (follow-set restriction)                                                                                                                                               |
| `pat_param` | pattern **without** top-level `|`                      | use when you need `|` *after* the capture in the matcher                                                                                                                                                          |
| `path`      | type-style paths, `Fn() -> ()` forms                   | —                                                                                                                                                                                                                 |
| `stmt`      | statement **without** trailing `;` (usually)           | **subtle:** semicolon-only statements; item `struct Foo;` includes `;` in capture; expression statements re-emit `;` unless expr is block/control-flow — affects **repetition counts** when expanding `$($stmt)*` |
| `tt`        | one token tree                                         | most flexible; use for munchers                                                                                                                                                                                   |
| `ty`        | type expression                                        | —                                                                                                                                                                                                                 |
| `vis`       | visibility, possibly **empty**                         | see quirks below                                                                                                                                                                                                  |


### `vis` quirks (book examples)

- Empty visibility: repeat as `$( $vis:vis , )*` — comma allows “nothing” between commas.
- `**$vis:vis` alone** cannot match zero tokens → `non_optional_vis!()` fails; must have *optional* repetition or other grammar.
- `**$(pub)? $ident:ident` is ambiguous** — `pub` is also a valid `ident` → **local ambiguity error**.
- **Empty `vis` + opacity:** matching empty with `$vis:vis` still creates a capture; if passed to another macro as `$tt:tt`, you get an **empty `tt`** — recursive matching may hit `($tt:tt)` with “nothing”, **not** the same as matching `()` tokens (see book’s `it_is_opaque!` example).

### Metavariables and Expansion Redux — matching semantics

- **No backtracking:** once the parser commits to a metavariable arm, it **does not** try the next rule on parse failure — it can **abort compilation** with a syntax error. **Rule ordering:** **most specific → least specific** (book: `dead_rule` with `expr` first makes `ident +` rule unreachable in practice).
- **Follow restrictions (Rust 1.46 table in book):** e.g. `expr`/`stmt` may only be followed by `=>`, `,`, or `;`; `pat` by `=>`, `,`, `=`, `if`, `in` (edition note: before 2021, `pat` could be followed by `\|`); `ty`/`path` have longer follow sets including `where`, `as`, etc.
- **Repetitions:** if `*`/`+` repeats, the **inner** fragment must be able to **follow itself**; if `?`/`*` allows zero, what comes **after** the repetition must be legal after **zero** instances.
- **No lookahead:** `$( $i:ident )* $i2:ident` + input `an_identifier` → **ambiguity error** (cannot see closing `)` ahead).
- **Substitution is not token-preserving:** `expr`/`meta`/etc. capture **AST**. When pasted into another macro, **you cannot destructure** the original tokens again.

### AST capture trap (`capture_then_match_tokens` / `capture_then_what_is`)

- Direct `match_tokens!(...)` on `tt` sees structure (`+`, parens).
- `**capture_then_match_tokens!($e:expr)`** passes an **opaque expression** → inner macro only hits the catch-all `$($other:tt)`* branch — **three times “got something else”**.
- Same for `#[$m:meta]` forwarded to `what_is!(#[$m])` vs direct `what_is!(#[no_mangle])` — specialized attribute arms match **only** when tokens weren’t pre-parsed as opaque `meta`.

**LLM takeaway:** if you need to inspect or re-parse macro input, capture with `**tt` slices** (or `ident`/`lifetime`) at the boundary; use `expr`/`ty` only when output is **paste-only**.

### Doc links (stable paths on veykril.github.io)

- Patterns index: `decl-macros/patterns/` — tt-muncher, push-down-acc, internal rules.
- Minutiae: `decl-macros/minutiae/` — fragment-specifiers, metavar-and-expansion, debugging.

---

## Supplement L — TLBORM building blocks + Effective Rust Items 21–26 (compressed)

### 04-design-patterns — AST coercion (TLBORM *Building Blocks*)

- **Problem:** after nested `tt` substitution, the outer parser may see an undifferentiated **token lump** where it expected a grammar slot → parse **gives up** instead of re-lexing intelligently.
- **Fix:** force re-classification by wrapping in a **no-op macro** that captures with the target fragment and re-emits:

```rust
macro_rules! as_expr { ($e:expr) => { $e } }
macro_rules! as_item { ($i:item) => { $i } }
macro_rules! as_pat  { ($p:pat)  => { $p } }
macro_rules! as_stmt { ($s:stmt) => { $s } }
macro_rules! as_ty   { ($t:ty)   => { $t } }
// Example: as_stmt!(let as_pat!(_): as_ty!(_) = as_expr!(42));
```

- **Use with** push-down accumulation: final accumulated `tt` sequence is **coerced** to `expr` / `item` / … so the crate author’s intent matches what the compiler parses.
- **Critical note (book):** which `as_*` exist is determined by **what macros are allowed to expand to**, not by what they can **match** — symmetry is not guaranteed a priori.

*Source:* [https://veykril.github.io/tlborm/decl-macros/building-blocks/ast-coercion.html](https://veykril.github.io/tlborm/decl-macros/building-blocks/ast-coercion.html)

### 04-design-patterns — Counting token trees (`count_tts` patterns)


| Technique                   | Idea                                                                                                                    | Scale / cost                                                                                                                       |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| **Repetition + replace**    | `0usize $(+ replace_expr!($tts 1usize))*`                                                                               | **~500 `tt`** → compiler builds **deep unbalanced `+` AST** — can **crash** rustc                                                  |
| **Linear recursion**        | `1usize + count_tts!($($tail)*)`                                                                                        | Hits `**recursion_limit`** quickly; use **typed literals** (`usize`) — rustc 1.2+ had bad inference on huge untyped int lit chains |
| **Chunked recursion**       | Match 20 / 10 / 5 / 1 `tt` per step                                                                                     | Book example reaches **~1,200 tokens** before breakage                                                                             |
| **Slice length**            | `<[()]>::len(&[$(replace_expr!($tts ())),*])`                                                                           | Tested **≥10,000** tokens                                                                                                          |
| **Const generic (1.51+)**   | `count_helper([$(replace_expr!($smth ())),*])` where `const fn count_helper<const N: usize>(_: [(); N]) -> usize { N }` | Similar scale; works in **const** contexts                                                                                         |
| **Enum counting**           | Distinct `ident` list → `enum Idents { ... }`; `last as u32 + 1`                                                        | Only **valid idents**, **no duplicates**                                                                                           |
| **Bit-twiddling / halving** | Even: `count!($($a:tt $b:tt)*) => count!($($a)*) << 1`; odd: peel one + `| 1`                                           | **AST depth ~O(log n)** — still can hit recursion limit eventually; “YatoRust” method                                              |


**LLM rule:** prefer **array/slice length** or **const-generic length** for large `tt` counts; avoid long `**a+a+a+…`** chains.

*Source:* [https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html](https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html)

### 04-design-patterns — Abacus counters (advanced)

- Represent integer **n** as **n copies** of a token (`+` / `-`) in a **group**; manipulate with **push-down** + recursion: increment/decrement = add/remove token; zero-test = match `()`.
- Book notes simpler closed form when you only need **numeric result** and not matching intermediate values: e.g. `0 $(+ abacus!($moves))`* over unary moves.
- Full **abacus** example in book — use when you need **compare-to-fixed-value** and incremental moves (DSLs, protocol stacks).

*Source:* [https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html#abacus-counters](https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html#abacus-counters)

### 04-design-patterns — “Parsing Rust” (subset matchers)

- Declarative macros can **partially** parse `fn` / `struct` / `enum` items for codegen; book **explicitly ignores generics** in examples — real crates need **proc-macros + `syn`** for full grammar.
- **Function matcher sketch:** `$(#[$meta:meta])* $vis:vis fn $name:ident ( $( $arg:ident : $arg_ty:ty ),* $(,)? ) $( -> $ret:ty )? { $($body:tt)* }` — reproduces a subset with attrs/vis/sig/body.
- **Takeaway:** `macro_rules!` item matchers are for **controlled DSLs**; don’t try to be `rustc`.

*Source:* [https://veykril.github.io/tlborm/decl-macros/building-blocks/](https://veykril.github.io/tlborm/decl-macros/building-blocks/) (chapter *Parsing Rust* in print book)

---

### Dependencies / semver / features — Effective Rust Items 21–26

**Item 21 — Semantic versioning (Cargo)**  

- **0.y.z** in Cargo: leftmost **non-zero** acts like major → `0.2` vs `0.3` can both exist; `0.1.2` vs `0.1.4` cannot (same 0.x line).  
- **Breaking without API change:** behavior change can violate Hyrum’s Law users — consider **type-level break** (new major) if semantics shifted.  
- **Enum variant / struct field:** adding variant often **major** unless `#[non_exhaustive]`; public tuple struct with all-public fields: adding field is **major**.  
- **Tooling:** `cargo-semver-checks` for mechanical API diff.  
- **MSRV:** ecosystem often treats compiler bump as **non-breaking** — document policy.

**Item 22 — Minimize visibility**  

- **Default private** per module; `pub` only at API edge.  
- `**pub(in path)`** — e.g. `pub(in crate::iter)` for std-style internal modules re-exported at parent.  
- **Reversing visibility is breaking** (major); widening from private→public is usually minor.  
- Aligns with Rust API guidelines: **private struct fields** for future-proofing.

**Item 23 — Avoid wildcard imports**  

- `use dep::*` — dep’s **minor** add can introduce **trait methods** clashing with yours → UFCS disambiguation hell.  
- Exceptions: `use super::*` in tests; curated `**prelude`**.  
- If you must glob external crates: **pin exact version** in `Cargo.toml`.

**Item 24 — Re-export deps whose types leak**  

- If your signature mentions `rand::Rng` v0.7 but caller uses 0.8 `ThreadRng`, trait impl errors are **opaque**.  
- Fix: `pub use rand;` (or re-export types) so callers use **your** path (`mycrate::rand::...`).

**Item 25 — Dependency graph**  

- **Flat `crates.io` namespace**; hyphen `some-crate` → `some_crate` in code.  
- **Multiple versions** allowed only when **semver-incompatible** ranges — but **FFI/static C** code: **one definition rule** → cannot link two copies of same C sym.  
- **Resolver** picks newest in **intersection** of ranges; record in **Cargo.lock**.  
- **Lockfile policy:** binaries **commit lock**; libraries **usually don’t publish lock** (but local/CI may keep one for deterministic dev).  
- **Feature unification:** union of all requested features across graph — drives Item 26.  
- **Tools:** `cargo tree`, `cargo tree --duplicates`, `cargo tree --invert`, `cargo tree --edges features`, **cargo-deny** (license/advisories/bans/duplicates), **cargo-udeps** (unused deps).  
- **Supply chain:** `build.rs` + **proc-macros execute at compile time** — compromised crate can run arbitrary code in CI.

**Item 26 — Feature creep**  

- `**optional = true` dependency** ⇒ implicit **feature name = dep name** (unless `dep:` syntax in features table — see Cargo docs).  
- **Features must be additive** — don’t pair mutually exclusive flags; unification can enable **both**.  
- **Avoid `#[cfg(feature)]` on public struct fields** or public trait methods — downstream **cannot know** if another path enabled the feature; use **private fields + constructors**, or **separate types**, or **defaulted trait methods** (Item 13) with care.  
- **N independent features ⇒ 2^N** combinations — CI should cover **matrix** or accept risk.

---

### Cross-links

- **ER Item 28** + **Perf Book compile-time** + **TLBORM counting/munchers** — same theme: **macro power vs build-time blowups**.
- **ER Item 25–26** + **workspace `Cargo.toml`** — feature unification is why **default features** and **minimal features** matter for compile time **and** binary size.

---

## Supplement M — TLBORM expanded reference (syntax extensions → patterns → tooling)

*Primary source: [The Little Book of Rust Macros](https://veykril.github.io/tlborm/) (Veykril et al.; fork of Daniel Keep’s book). Below is a structured digest for LLM retrieval; examples follow the book’s intent.*

### Part 1 — Pipeline: tokens, AST, token trees, macros

**Tokenization (lexing)**  

- Source → stream of tokens: identifiers, literals, keywords (incl. **reserved** e.g. `yield`, `macro` — lexer knows them though not all are active syntax), symbols.  
- **Multi-char ops are single tokens:** `::` is one token, not two `:`.  
- `**self`** is both identifier and keyword (special cases later in macros).  
- Rust macros are **not** like C preprocessor at lex time — expansion is **after** AST construction.

**Parsing**  

- Token stream → **AST** (expression structure). At parse time, names are **not** resolved (no “what is `a`?” yet).

**Token trees (TT)**  

- TT sit **between** flat tokens and full AST: most tokens are **leaves**; **grouping** `(...)`, `[...]`, `{...}` are **interior nodes** with nested TT.  
- **Same text** can yield different TT forest vs AST shape — e.g. `a + b + (c + d[0]) + e` has **seven root-level TT**; the AST is one tree. Macro authors must keep both mental models.  
- **Invariant:** no unpaired delimiters; no illegally nested groups in TT.

*Chapter:* [Source Analysis](https://veykril.github.io/tlborm/syntax-extensions/source-analysis.html)

---

### Part 2 — Macros in the AST: four surface forms, invocation sites, `!` input

**Four syntax-extension shapes**  

1. `#[$arg]` — attributes (`#[derive]`, …).
2. `#![$arg]` — inner attributes.
3. `$name! $arg` — **function-like** (`println!`, proc-macros, `macro_rules!`-defined).
4. `$name! $arg0 $arg1` — **only** `macro_rules!` itself uses this second-arg form.

**Function-like `!` invocation**  

- Parser does **not** parse inside the delimiters as Rust upfront — argument is **one non-leaf token tree** `{...}`, `(...)`, or `[...]`. Hence DSLs like `bitflags! { ... }` / `lazy_static! { ... }` — contents can even be **invalid Rust** until macro expands.  
- `format!` is `macro_rules!`; `**format_args!`** is a **compiler builtin** (not mbe).  
- Attributes’ inner `meta` is path + optional `=` literal or **token tree** — still TT-based.

**Where `!` macros may appear (whitelist)**  

- **Pattern, statement, expression, item (incl. `impl` item), type.**  
- **Cannot** appear as: bare **identifier**, **match arm**, **struct field**, etc. — **no** exceptions in the book’s wording.

*Chapter:* [Macros in the AST](https://veykril.github.io/tlborm/syntax-extensions/macros-in-ast.html) — *if 404 in some builds, same text exists in [print.html](https://veykril.github.io/tlborm/print.html#macros-in-the-ast).*

---

### Part 3 — Expansion mechanics

- After AST exists, **before** name resolution / type checking, compiler **walks AST**, finds syntax extensions, **replaces** invocation node with expanded AST.  
- Expansion is **structural (AST)**, not textual paste — so **no** “half expression” leakage; context forces a full **expression**, **pattern**, **type**, **item(s)**, or **stmt(s)**.  
- Nested macros: expand outer → inner may still contain macros → **multi-pass** until fixed point.  
- **Recursion limit default 128** expansion steps — `#![recursion_limit = "..."]` crate-wide; raising it can **hurt compile time**.

*Chapter:* [Expansion](https://veykril.github.io/tlborm/syntax-extensions/expansion.html)

---

### Part 4 — Hygiene (overview + `macro_rules!` + `$crate`)

**General idea**  

- **Create** vs **use** identifiers: `struct Foo` / `let foo` create; `Foo` in type position / `foo` in expr use.  
- Hygienic if: created names **don’t leak** to caller; uses **don’t bind** to caller’s names unintentionally.

`**macro_rules!` = mixed / partial hygiene**  

- Hygienic for **locals, labels**, and `**$crate`**.  
- Implemented via invisible **syntax context** on identifiers — **same spelling ≠ same identifier** if contexts differ.  
- Fix for “pass name in”: take `**$a:ident`** and emit `**let $a = ...**` so caller’s ident **shares** context with the expression that uses it.

`**$crate`**  

- Expands to **absolute path to defining crate** — needed so `#[macro_export]` macros can call **sibling macros/items** without relying on caller’s imports.  
- Use `**$crate::module::item`** for non-macro items; for **nested modules**, qualify fully (book shows `#[macro_export]` + `inner::foo`).  
- Pre-1.30, `**$crate::OtherMacro!`** for macros was **not** reliable with `macro_rules!` name resolution; **1.31+** / 2018 edition improved namespacing.

*Chapters:* [syntax-extensions/hygiene](https://veykril.github.io/tlborm/syntax-extensions/hygiene.html), [decl-macros/minutiae/hygiene](https://veykril.github.io/tlborm/decl-macros/minutiae/hygiene.html)

---

### Part 5 — Scoping (`macro_rules!`): textual vs path

**Textual scope (unqualified `foo!`)**  

- Macros visible **after** definition in source order — **not** hoisted like `fn`.  
- **Exception:** unlike other items, `macro_rules!` is visible in **nested modules** **below** in the same crate hierarchy (book: `X!()` works in `mod a`, `b`, `c` after one top-level `macro_rules! X`).  
- **Does not leak** out of the **lexical** block/module where defined unless `#[macro_use]` / export.  
- **Shadowing:** redefining `macro_rules! X` replaces; inner modules can shadow outer.  
- **Macros expanding to macros:** resolution of `Y!` inside `X!`’s body happens at **expansion** time — order of `macro_rules! Y` vs call sites matters (book’s `X!` → `Y!` examples).

`**#[macro_use] mod`**  

- Lifts macros defined in that module so **later** sibling modules see them — **order of `mod` declarations matters** (`#[macro_use] mod defines_macros` **before** `mod uses_macros`).

`**#[macro_use] extern crate`**  

- Behaves like **hoisting** imported macros to top of module — **different** from file `mod` behavior.

**Path-based scope**  

- `**#[macro_export]`** puts macro in **crate root** namespace (ignores module `vis` on the defining `mod`).  
- **Rust 2018+:** `use crate::my_macro` / `use dep::macro_name` for external crates — **namespaced** imports.  
- **Within same crate**, book still notes: macros in submodules may need `**#[macro_use]` on child module** to use in order-sensitive setups (historical pattern: **macros at top of `lib.rs`** before other `mod`s).

*Chapter:* [Scoping](https://veykril.github.io/tlborm/decl-macros/minutiae/scoping.html)

---

### Part 6 — Import / export across editions

**2015**  

- `#[macro_use]` on `mod` / `extern crate`; `**#[macro_use(foo, bar)] extern crate*`* to import **only** listed macros (reduce pollution, enable local overrides).  
- `**#[macro_use]` on extern crate only from crate root.**  
- `**#[macro_export]`** ignores normal visibility.

**2018+**  

- `**use some_crate::some_macro;`** — ergonomics like normal items.  
- **Caveat (book):** for macros **defined in the same crate**, you may still need `**#[macro_use]` on the defining submodule** — not fully unified with regular item paths in all cases.

`**$crate` + renames**  

- Crates can be **renamed** in `Cargo.toml` — `$crate` tracks the **actual** crate id.

*Chapter:* [Import and Export](https://veykril.github.io/tlborm/decl-macros/minutiae/import-export.html)

---

### Part 7 — Patterns: TT muncher, push-down, internal rules

**TT muncher**  

- Recursive macro; each step **consumes** a prefix of `$($tail:tt)*`, emits side effects, recurses.  
- **Only** `tt` repetition can **losslessly** hold arbitrary remainder.  
- **Cannot** match unbalanced groups.  
- **Quadratic** compile cost if one-tt-per-step (see Supplement L / book).  
- **Mitigations:** chunk rules, multiple top-level invocations instead of one giant DSL, **rule order** = most frequent first, prefer `*`/`+` repetitions when possible; **quote** crate has advanced **non-quadratic** patterns ([link in book](https://github.com/dtolnay/quote/blob/31c3be473d0457e29c4f47ab9cff73498ac804a7/src/lib.rs#L664-L746)).

*Chapter:* [Incremental TT Munchers](https://veykril.github.io/tlborm/decl-macros/patterns/tt-muncher.html)

**Push-down accumulation**  

- Cannot expand to **partial** syntax — forbidden. Push-down carries **state** `(input..., acc...)` until a rule emits a **complete** fragment; accumulator kept as `**$($body:tt)*`**.  
- **Quadratic** in acc length; combined with muncher → **doubly** bad. Put **accumulator at end** of matchers so failed rules don’t scan long acc first.

*Chapter:* [Push-down Accumulation](https://veykril.github.io/tlborm/decl-macros/patterns/push-down-acc.html)

**Internal rules**  

- Prefix like `@as_expr` to dispatch **inside** one `macro_rules!` instead of many exported helpers — fixes 2015 **global namespace pollution**; still useful for structure.  
- `**@`**: historically unused as prefix operator (GC pointer era); now conventional; **internal rules first** so matcher doesn’t treat `@foo` as beginning of wrong production.  
- **Cost:** more rules → more failed match attempts → slower compile.

*Chapter:* [Internal Rules](https://veykril.github.io/tlborm/decl-macros/patterns/internal-rules.html)

---

### Part 8 — Procedural macros: `Span` hygiene modes

Each token carries a **Span** (source region + expansion info). For identifiers, span controls **name resolution boundaries**:


| Constructor (conceptual)  | Behavior                                                          |
| ------------------------- | ----------------------------------------------------------------- |
| `**def_site`** (unstable) | True **def-site** hygiene — isolated from outer binders.          |
| `**mixed_site`**          | Same **mixed** behavior as `**macro_rules!`**.                    |
| `**call_site**`           | **Unhygienic** — behaves like user wrote identifier at call site. |


*Chapter:* [proc-macros/hygiene](https://veykril.github.io/tlborm/proc-macros/hygiene.html) (see also `proc_macro::Span` docs)

---

### Part 9 — Debugging declarative macros

- `**#![feature(trace_macros)]` + `trace_macros!(true)`** — compiler prints each **macro_rules** expansion step (nightly). CLI: `**-Z trace-macros`**.  
- `**log_syntax!**` (nightly feature) — dump tokens passed to it; good for **targeted** spelunking inside munchers.  
- **Classic:** `rustc +nightly -Zunpretty=expanded` / `**cargo expand`**.  
- **macro_railroad** ([lukaslueg/macro_railroad](https://github.com/lukaslueg/macro_railroad)) — **syntax diagrams** / automata view of `macro_rules!` grammars.

*Chapter:* [decl-macros/minutiae/debugging](https://veykril.github.io/tlborm/decl-macros/minutiae/debugging.html) — see also [syntax-extensions/debugging](https://veykril.github.io/tlborm/syntax-extensions/debugging.html) for general `-Zunpretty`.

---

### Part 10 — Cross-reference index (TLBORM → this file)


| TLBORM topic                                     | Prior supplements               |
| ------------------------------------------------ | ------------------------------- |
| Fragment specifiers + follow-sets + opacity      | **Supplement K**                |
| Metavariables / no backtrack / AST capture traps | **Supplement K**                |
| AST coercion `as_`*                              | **Supplement L**                |
| Counting / abacus                                | **Supplement L**                |
| Building blocks index                            | **Supplements J–L**, appendices |

---

## Supplement N — TLBORM topics not spelled out elsewhere + honesty checklist

*Purpose: explicit “what we did / did not fully transcribe” so LLM consumers know where to open the book.*

### Still in TLBORM but only skimmed or absent above

**1. «Macros, A Practical Introduction» (recurrence → `Iterator`)**  
- Long **worked example** (design invocation syntax → `macro_rules!` → expansion).  
- **Retrieval:** the *pattern* is “design macro API first, then rules”; full code walkthrough lives only in [print.html](https://veykril.github.io/tlborm/print.html) — not duplicated here (would be hundreds of lines).

**2. Non-Identifier Identifiers (`self`)**  
- `self` is a **keyword** but can match **`$i:ident`** in some invocations; hygiene + “`mut` must follow a named binding” interact badly with `make_mutable!(self)`.  
- Macros **cannot** see the method receiver’s `self` unless passed through parameters — the book stresses **keyword `self` has hygiene-like behavior** in expansions.  
- *Chapter:* [Non-Identifier Identifiers](https://veykril.github.io/tlborm/decl-macros/minutiae/non-identifier-identifiers.html)

**3. Turing completeness + Tag Systems**  
- **Proof sketch:** `macro_rules!` can simulate a Tag System (m>1) ⇒ Turing-complete expansion ⇒ **Rice-style undecidability** (equivalence of two macros, redundancy of arms, etc.).  
- **Engineering implication:** no general algorithm to prove two macros equivalent; compiler must **cap** expansion.  
- *Chapter:* [Turing Completeness](https://veykril.github.io/tlborm/decl-macros/turing-complete.html)

**4. Repetition replacement**  
- Discard captured `ty` list but repeat **`Default::default()`** N times via `replace_expr!($t $sub)` — classic for **>12 tuple** default tuple / arity codegen.  
- *Chapter:* [Repetition Replacement](https://veykril.github.io/tlborm/decl-macros/patterns/repetition-replacement.html)

**5. TT bundling**  
- Pack many forwarded params into **one grouped `tt`** (e.g. `(a: $a:ident, b: $b:ident)`) so recursive rules only pass **`$ab:tt`**.  
- Terminal rules **destructure** the bundle.  
- *Chapter:* [TT Bundling](https://veykril.github.io/tlborm/decl-macros/patterns/tt-bundling.html)

**6. Enum item matcher**  
- **TT muncher + internal rules** to walk `enum` variants (tuple / struct / unit) with attributes and visibility; book **visits** tokens rather than full re-emit (would need heavy push-down).  
- Shows realistic **partial** parsing of Rust items in `macro_rules!`.  
- *Chapter:* [Enum](https://veykril.github.io/tlborm/decl-macros/building-blocks/enum-item-matcher.html)

**7. Metavariable expressions (unstable)**  
- `${count(ident)}`, `${index()}`, `${len()}`, `${ignore(ident)}`, `$$` escape — RFC / feature `macro_metavar_expr`.  
- *Chapter:* [Metavariable Expressions](https://veykril.github.io/tlborm/decl-macros/minutiae/metavar-expr.html)

**8. Procedural macros — “practical” half**  
- Book marks **WIP**; **methodical** intro (three kinds, panic vs `compile_error!`, syn/quote) is covered in **Supplement J** + **M**.  
- For **derive** helpers, attribute replacement semantics, see [proc-macros](https://veykril.github.io/tlborm/proc-macros/) chapters on crates.io.

**9. Glossary**  
- End of book — terms like “syntax extension”, “function-like macro” — **Supplement M** already defines most.

---

### Effective Rust — structural caveat (unchanged)

- В начале файла **`### ER Item N`** в нескольких местах **не совпадают** с официальной таблицей 1–35 — исправление только через **каноническую таблицу** + **Supplements J–L/N**.  
- **Items 27–35** (доки, тесты, CI, `no_std`, FFI, bindgen) — покрыты **короче**, чем «Types/Traits» блоки.

### The Rust Performance Book

- Основные главы сведены в **Supplement J**; отдельно **не** перечислены все внешние PR/issue ссылки из оригинала — они в онлайн-книге.

### When to open the primary sources

| Need | Open |
|------|------|
| Full macro walkthrough (recurrence example) | TLBORM *Practical Introduction* |
| `self` + macro edge cases | TLBORM *Non-Identifier Identifiers* |
| Proof / undecidability story | TLBORM *Turing Completeness* |
| Latest `decl_macro` / rustc tracking | [rust#39412](https://github.com/rust-lang/rust/issues/39412) + nightly docs |
| Exact semver tables | Effective Rust Item 21 + Cargo book |

--- end of cluster-04 notes ---