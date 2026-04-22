# Cluster 01: The Book, Rust by Example, Std, Style Guide, Edition Guide, Error Codes

Dense expert notes from official Rust documentation on doc.rust-lang.org: *The Rust Programming Language* (Book; front matter assumes Rust 1.90+ and `edition = "2024"`), *Rust by Example*, `std`/`alloc` API docs, *The Rust Reference* (namespaces, selected items), *The Rust Style Guide* (nightly), *The Rust Edition Guide* (2021 & 2024 pages), and the *Error codes index* (`rustc --explain`, `error_codes/index.html`). Later parts (IV–VIII) add cell/Arc, Pin/Drop, iterators/coercions, I/O, `MaybeUninit`, task API, sync primitives, `alloc`/`no_std`, and extended error excerpts. Tagged bullets; `[source: …]` points to the official page or Book chapter.

---

## [TAG: 01-meta-principles] Philosophy and framing
- Ownership is Rust's rule system for heap correctness; drop at scope end (RAII). No GC. [source: Book ch.4]
- Stack vs heap: stack LIFO fixed-size; heap for unknown/dynamic size; pointer to heap is fixed-size on stack. [source: Book ch.4]
- Rust never silently deep-copies; `clone()` is explicit cost signal. [source: Book ch.4]
- `std` is default-available; prelude re-exports common traits/types. [source: std crate root]
- Methods on `String`/`Vec` often forward to `str`/`[T]` via deref coercion. [source: std crate root]

## [TAG: 02-language-rules] Ownership — rules
- Each value has exactly one owner; ownership transfers on move; drop runs once. [source: Book ch.4]
- Assigning a new value to an existing `let mut` drops the previous value immediately. [source: Book ch.4]
- Function arguments and returns use the same move/copy rules as assignment. [source: Book ch.4]
- `Copy` + `Drop` cannot both be implemented for a type. [source: Book ch.4]
- Tuples/arrays are `Copy` iff all elements are `Copy`. [source: Book ch.4]

## [TAG: 02-language-rules] Borrowing
- Either one `&mut T` or any number of `&T`, not both overlapping. [source: Book ch.4.2]
- References must always be valid; dangling references are compile errors. [source: Book ch.4.2]

## [TAG: 02-language-rules] Lifetimes — semantics
- Lifetime parameters relate reference lifetimes in signatures; they do not change runtime extent. [source: Book ch.10.3]
- `longest<'a>(x: &'a str, y: &'a str) -> &'a str`: returned borrow valid for overlap of input borrows. [source: Book ch.10.3]
- Compiler rejects code where returned reference could outlive a shorter input (E0597 pattern). [source: Book ch.10.3]
- If implementation always returns one parameter's borrow, other parameters need not share lifetime (Book `longest` variant). [source: Book ch.10.3]
- Returning reference to local data is rejected (E0515); return owned value instead. [source: Book ch.10.3]

## [TAG: 02-language-rules] Lifetime elision — E0106
- E0106: lifetime missing where required (structs holding references, type aliases to references, some signatures). [source: error_codes/E0106]
- Elided output lifetime requires either exactly one input lifetime, or `&self`/`&mut self` method with multiple inputs. [source: E0106]
- Examples of elision failure: `fn f() -> &str`, `fn g(x: &str, y: &str) -> &str` without named lifetimes. [source: E0106]

## [TAG: 02-language-rules] Trait not implemented — E0277
- E0277: type does not implement a trait required in context. [source: E0277]
- Generic functions must declare bounds on type parameters; rustc does not infer from call sites only. [source: E0277]

## [TAG: 02-language-rules] Type mismatch — E0308
- E0308: expected type ≠ found type (calls, `if` conditions, `let` annotations). [source: E0308]

## [TAG: 02-language-rules] Use after move — E0382
- E0382: value used after move (non-`Copy`). [source: E0382]
- Fix: borrow, `clone()`, or interior/shared ownership patterns (`Rc`/`RefCell` in diagnostic). [source: E0382]

## [TAG: 02-language-rules] Conflicting borrows — E0502
- E0502: borrow conflicts with different mutability. [source: E0502]
- Reorder so the conflicting borrow ends before the other begins. [source: E0502]

## [TAG: 02-language-rules] Assign while borrowed — E0506
- E0506: assign to owner while reference exists. [source: E0506]
- Narrow borrow scope or move borrow into a function. [source: E0506]

## [TAG: 02-language-rules] Dangling return — E0515
- E0515: return reference to local/temporary that does not outlive function. [source: E0515]
- Return owned values / iterators owning data. [source: E0515]

## [TAG: 02-language-rules] Value does not live long enough — E0597
- E0597: storage dropped while borrow still required. [source: E0597]

## [TAG: 02-language-rules] Unstable feature — E0658
- E0658: unstable feature; nightly + `#![feature(...)]` per docs, or avoid. [source: E0658]

## [TAG: 02-language-rules] Temporary dropped while borrowed — E0716
- E0716: statement-scoped temporary ends before derived reference. [source: E0716]
- Use `let` binding with adequate scope; `let r = &foo()` can extend temporary to block end in simple cases. [source: E0716]

## [TAG: 02-language-rules] Send / Sync (auto traits)
- `Send`: safe to transfer ownership across threads; `Rc` is `!Send` (non-atomic refcount). [source: std::marker::Send]
- `Arc` uses atomics; `Send` when `T: Send` (std docs narrative). [source: std::marker::Send]
- `Sync` iff `&T` is `Send` — safe to share immutable refs across threads. [source: std::marker::Sync]
- `Cell`/`RefCell` are not `Sync` (unsynchronized interior mutability). [source: std::marker::Sync]
- Interior mutability must use `UnsafeCell`; transmuting `&T` to `&mut T` is UB. [source: std::marker::Sync]

## [TAG: 03-idioms] Generics and performance
- Trait bounds express capabilities: `T: PartialOrd` enables `>` in generic `largest`. [source: Book ch.10]
- Monomorphization specializes generics per concrete type at compile time. [source: Book ch.10]
- `impl<T> Point<T>` vs `impl Point<f32>` — second adds methods only for `f32` instance. [source: Book ch.10]

## [TAG: 03-idioms] Closures
- `unwrap_or_else` closure: `FnOnce() -> T` bound. [source: Book ch.13]
- Inference locks closure input/output types at first call site. [source: Book ch.13]
- Capture modes: `&`, `&mut`, or `move` (threads). [source: Book ch.13]
- `Fn` / `FnMut` / `FnOnce` classification per body (move out -> `FnOnce`). [source: Book ch.13]
- `sort_by_key` needs `FnMut` because closure runs per element. [source: Book ch.13]

## [TAG: 03-idioms] Iterator — trait surface
- Core: associated type `Item`; required `next`. [source: std::iter::Iterator]
- Adapters compose lazily; `collect`/`fold`/etc. consume.
- Book minigrep: `lines().filter(...).collect()` replaces explicit `mut` vec — less mutable state. [source: Book ch.13]

## [TAG: 03-idioms] Iterator — provided methods (inventory)
- Listed names appear as `fn` items on `Iterator` in std docs snapshot. Prefer rustdoc for exact bounds/changed signatures. [source: doc.rust-lang.org/std/iter/trait.Iterator.html]
- `next_chunk`
- `size_hint`
- `count`
- `last`
- `advance_by`
- `nth`
- `step_by`
- `chain`
- `zip`
- `intersperse`
- `intersperse_with`
- `map`
- `for_each`
- `filter`
- `filter_map`
- `enumerate`
- `peekable`
- `skip_while`
- `take_while`
- `map_while`
- `skip`
- `take`
- `scan`
- `flat_map`
- `flatten`
- `map_windows`
- `fuse`
- `inspect`
- `by_ref`
- `collect`
- `try_collect`
- `collect_into`
- `partition`
- `partition_in_place`
- `is_partitioned`
- `try_fold`
- `try_for_each`
- `fold`
- `reduce`
- `try_reduce`
- `all`
- `any`
- `find`
- `find_map`
- `try_find`
- `position`
- `rposition`
- `max`
- `min`
- `max_by_key`
- `max_by`
- `min_by_key`
- `min_by`
- `rev`
- `unzip`
- `copied`
- `cloned`
- `cycle`
- `array_chunks`
- `sum`
- `product`

### `Iterator::next_chunk`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::size_hint`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::count`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::last`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::advance_by`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::nth`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::step_by`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::chain`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::zip`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::intersperse`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::intersperse_with`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::map`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::for_each`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::filter`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::filter_map`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::enumerate`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::peekable`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::skip_while`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::take_while`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::map_while`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::skip`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::take`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::scan`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::flat_map`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::flatten`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::map_windows`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::fuse`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::inspect`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::by_ref`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::collect`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::try_collect`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::collect_into`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::partition`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::partition_in_place`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::is_partitioned`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::try_fold`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::try_for_each`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::fold`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::reduce`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::try_reduce`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::all`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::any`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::find`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::find_map`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::try_find`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::position`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::rposition`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::max`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::min`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::max_by_key`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::max_by`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::min_by_key`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::min_by`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::rev`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::unzip`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::copied`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::cloned`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::cycle`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::array_chunks`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::sum`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

### `Iterator::product`
- [TAG: 03-idioms] Listed as provided method on `Iterator` in std; verify unstable/const status in your toolchain's rustdoc. [source: std::iter::Iterator]
- [TAG: 09-performance] Adapter chains typically compile to a single loop when inlined; inspect LLVM IR only if profiling shows hot path. [source: Book ch13 performance discussion]
- [TAG: 05-anti-patterns] Avoid `collect`ing huge intermediate vectors if the next step could stream (`impl Iterator` return). [source: Book minigrep iterator refactor]
- [TAG: 03-idioms] Prefer `by_ref()` when you need to iterate multiple times without consuming the underlying iterator. [source: std::iter::Iterator]
- [TAG: 06-error-handling] For `try_*` variants, respect `Try` short-circuit semantics — same as `?` in loops. [source: std::iter::Iterator]
- [TAG: 02-language-rules] Double-ended iterators enable `rev()`; not all iterators are `DoubleEndedIterator`. [source: std::iter::Iterator]
- [TAG: 04-design-patterns] `fuse()` can make malformed iterators safer after manual `next` misuse; rare in production code. [source: std::iter::Iterator]
- [TAG: 09-performance] `size_hint` informs `Vec` reserve in `collect`; trust lower bound only for optimization hints. [source: std::iter::Iterator]
- [TAG: 03-idioms] Combine with `Enumerate` when indexing matters; avoid manual `idx` counters when `enumerate()` suffices. [source: Book iterator style]

## [TAG: 03-idioms] Cow
- `Cow<'a, B>`: `Borrowed` vs `Owned`; `Deref` for read; `to_mut` clones on demand. [source: std::borrow::Cow]
- `Rc::make_mut` / `Arc::make_mut` also provide COW patterns. [source: std::borrow::Cow]

## [TAG: 04-design-patterns] Trait objects
- `Vec<Box<dyn Trait>>` allows heterogeneous implementors; generics + bounds require homogeneous `T`. [source: Book ch.18]
- Dynamic dispatch costs vtable lookup; static dispatch via generics can inline. [source: Book ch.18]
- Trait objects cannot carry extra fields; only behavior. [source: Book ch.18]

## [TAG: 04-design-patterns] Encapsulation
- Private fields + public API methods maintain invariants (`AveragedCollection`). [source: Book ch.18]
- Inheritance for code reuse -> default trait methods; for polymorphism -> trait objects/generics. [source: Book ch.18]

## [TAG: 05-anti-patterns] Anti-patterns (compiler errors as guidance)
- Relying on elided lifetimes where relationships are non-obvious — use named lifetimes. [source: Book ch.10 + E0106]
- Closure that moves value into `sort_by_key` — incompatible with `FnMut`. [source: Book ch.13]

## [TAG: 06-error-handling] Error handling
- `Result` for recoverable failure; `panic!` for unrecoverable/bug. [source: Book ch.9]
- `?` propagates errors in compatible `Try` contexts.
- `Box<dyn Error>` in examples for heterogeneous error sources. [source: Book ch.12]
- `unwrap_or_else` vs `panic!` for CLI UX (minigrep). [source: Book ch.12]

## [TAG: 07-async-concurrency] Async / threads (Book)
- Threads vs async tasks: choose based on workload; often combine (Book narrative). [source: Book ch.17]
- Work stealing runtimes may move tasks across threads. [source: Book ch.17]

## [TAG: 08-unsafe-and-ffi] Unsafe + FFI
- Five unsafe capabilities listed in Book: raw deref, unsafe fn, static mut, unsafe trait, union fields. [source: Book ch.20]
- `unsafe` does not turn off borrow checking on references. [source: Book ch.20]
- `split_at_mut` pattern: prove disjointness manually inside `unsafe` using raw parts. [source: Book ch.20]
- FFI: `unsafe extern "C"` blocks; can mark individual imports as `safe fn` when sound. [source: Book ch.20]
- Exporting to C: `extern "C"` + `#[unsafe(no_mangle)]` and collision caveats. [source: Book ch.20]
- Mutable static requires `unsafe` and documented synchronization obligations. [source: Book ch.20]
- `unsafe impl Send/Sync` for types with raw pointers — manual proof obligation. [source: Book ch.20]
- Miri for UB detection at runtime. [source: Book ch.20]

## [TAG: 09-performance] mem module
- `size_of`, `align_of`, `align_of_val` — layout.
- `MaybeUninit`, `ManuallyDrop`, `forget` — control drop/initialization.
- `transmute` unsafe — layout constraints.
- [source: std::mem module index]

## [TAG: 10-testing-and-tooling] Testing
- `#[test]`, `#[cfg(test)]`, `assert!` family. [source: Book ch.11]
- `cargo test` runs unit + integration; doc tests compile ` ``` ` examples. [source: Book ch.11]
- Nightly benchmark tests mentioned in Book. [source: Book ch.11]

## [TAG: 10-testing-and-tooling] Style Guide (formatting defaults)
- Spaces not tabs; 4-space indent; 100 column max width; trailing commas in broken lists. [source: Style Guide intro]
- Prefer block indent over visual indent for arguments. [source: Style Guide]
- Single `derive` attribute; merged derives preserve trait order. [source: Style Guide]
- `extern` items: always specify ABI (`extern "C"`). [source: Style Guide items]
- Imports version-sorted within groups; `self`/`super` first; globs last. [source: Style Guide items]

## [TAG: 03-idioms] Rust by Example
- RBE states scopes govern validity of borrows, drop, and variable creation/destruction. [source: RBE scope.html]

## [TAG: 02-language-rules] Std operational caveat
- Behavior before/after `main` for `std` is best-effort; test per platform if relied upon. [source: std root]

## [TAG: 12-modern-rust] Edition 2021 — prelude
- Prelude adds `TryInto`, `TryFrom`, `FromIterator` — may break method resolution vs custom traits.
- Migrate with `cargo fix --edition` and `rust_2021_prelude_collisions` lint.
- Disambiguate with UFCS or receiver adjustments (incl. `dyn Trait` + prelude clash example). [source: Edition Guide rust-2021/prelude]

## [TAG: 12-modern-rust] Edition 2024 — unsafe_fn lint
- `unsafe_op_in_unsafe_fn` warns if unsafe operations in `unsafe fn` lack inner `unsafe {}` block.
- Motivation: separate caller-unsafe from proof-local unsafe.
- Migrate via `cargo fix --edition` or crate-level `#![warn(unsafe_op_in_unsafe_fn)]`. [source: Edition Guide rust-2024/unsafe-op-in-unsafe-fn]
- Rust 2024 index cites RFC #3501 and release 1.85.0. [source: Edition Guide rust-2024/index]

## [TAG: 11-ecosystem-crate-picks] Named crates/tools in fetched docs
- Book async examples use `trpl` teaching crate. [source: Book]
- Miri installed via rustup nightly component. [source: Book ch.20]

## [TAG: 03-idioms] Iterator — usage notes (idiomatic combinations)
- `map` + `collect` transforms container; preserve error handling with `Result` inside `map` + `collect::<Result<_,_>>()` or `try_fold` / `try_for_each` when using fallible steps. [source: std::iter::Iterator + Book error propagation patterns]
- `filter` + `map` often replaced by `filter_map` when mapping to `Option`. [source: Iterator trait]
- `flat_map`/`flatten` for nested iterable items; watch allocation if inner iterators allocate. [source: Iterator trait]
- `fuse` after `None` sticky — useful for iterators that may violate fused assumption after manual `next` misuse (rare). [source: Iterator trait]
- `inspect` side effects — debugging only; do not rely on for correctness. [source: Iterator trait]

## [TAG: 06-error-handling] Guess / invariant pattern
- `Guess::new` may `panic!` on invalid range — document panic conditions in public API. [source: Book ch.9]

---

## Part II — Chapter digests (Book + RBE + std tour)

## [TAG: 02-language-rules] Book ch4 — References: rules and errors (detailed)
- Creating a reference borrows without taking ownership; pointee is not dropped when reference goes out of scope. [source: Book ch4.2]
- References are immutable by default; `push_str` on `&String` fails (E0596) — need `&mut String`. [source: Book ch4.2]
- At most one active `&mut` to a value; two `&mut` simultaneously is E0499. [source: Book ch4.2]
- Data race definition (three conditions): multiple pointers access same data, at least one writes, no synchronization — UB; Rust rejects at compile time. [source: Book ch4.2]
- Scopes can be nested so `&mut` borrows are sequential, not simultaneous. [source: Book ch4.2]
- Cannot have `&mut` while any `&` exists to same data (E0502 example in Book). [source: Book ch4.2]
- NLL: reference scope ends at last use, not necessarily end of block — allows `println!` then `&mut` after immutable refs. [source: Book ch4.2]
- Dangling references prevented: returning `&String` to local is invalid; compiler suggests owned `String` or lifetimes. [source: Book ch4.3]
- Two reference rules recap: exclusive mut *or* many shared immut; references must always be valid. [source: Book ch4.3]

## [TAG: 06-error-handling] Rust by Example — error handling
- Explicit `panic` mainly for tests and unrecoverable errors; `unimplemented!` preferred for unfinished code in prototypes. [source: RBE error.html]
- `Option` for optional values / absence not an error; `unwrap` ok for prototypes; `expect` better for message. [source: RBE error.html]
- Use `Result` when caller must handle failure; avoid `unwrap` outside tests/prototypes. [source: RBE error.html]

## [TAG: 10-testing-and-tooling] Rust by Example — testing
- Rust embeds tests in the language; three styles (see RBE sub-pages); additional test dependencies supported. [source: RBE testing.html]
- Cross-link: Book ch11 for testing mechanics. [source: RBE testing.html]

## [TAG: 02-language-rules] Rust by Example — traits
- Trait methods use `Self` as the implementing type; associated functions can return `Self`. [source: RBE trait.html]
- Traits may provide default methods; impls can override. [source: RBE trait.html]

## [TAG: 08-unsafe-and-ffi] Rust by Example — unsafe
- Minimize unsafe; Book is canonical reference (RBE links to Book unsafe chapter). [source: RBE unsafe.html]
- Unsafe used for: raw pointer deref, calling `unsafe` fn/FFI, `static mut`, `unsafe trait` impl (RBE lists four primary items — compare to Book's five-item list for drift). [source: RBE unsafe.html]
- Dereference raw pointers only inside `unsafe` blocks. [source: RBE unsafe.html]
- `slice::from_raw_parts` requires valid pointer + correct type + length invariants — otherwise UB. [source: RBE unsafe.html]

## [TAG: 10-testing-and-tooling] Rust by Example — macros
- Macros expand to ASTs (not string preprocessing), reducing precedence surprises vs C macros. [source: RBE macros.html]
- `macro_rules!` defines declarative macros; useful for DRY across types and variadic-like APIs (`println!`). [source: RBE macros.html]


## [TAG: 04-design-patterns] Smart pointers — chapter overview (Book ch.15 intro)
- Smart pointers: structs acting like pointers with extra metadata; often own pointee (vs references that borrow). [source: Book ch15-00]
- Common std smart pointers: `Box<T>`, `Rc<T>`, `Ref`/`RefMut` via `RefCell<T>`; chapter also covers interior mutability and reference cycles. [source: Book ch15-00]
- Smart pointers typically implement `Deref` (use like references) and `Drop` (cleanup). [source: Book ch15-00]

## [TAG: 04-design-patterns] Box<T> (Book ch15-01)
- `Box::new` stores payload on heap; stack holds fixed-size pointer. [source: Book ch15-01]
- Primary uses: (1) DST/indirection for unknown compile-time size contexts, (2) move large data without copying payload (only pointer moves on stack), (3) trait objects (`Box<dyn Trait>`) per cross-reference to ch18. [source: Book ch15-01]
- Recursive types: direct `enum List { Cons(i32, List) }` is infinite size (E0072); indirection via `Box<List>` (or `Rc`, `&`) breaks cycle so size is known. [source: Book ch15-01]
- `Cons(i32, Box<List>)` size is `i32` + pointer — compiler can compute enum size as max variant size. [source: Book ch15-01]

## [TAG: 04-design-patterns] Rc<T> (Book ch15-04)
- `Rc` enables multiple ownership via refcount; dropped when count hits zero. [source: Book ch15-04]
- Single-threaded only; multithreaded refcount is `Arc` (ch16). [source: Book ch15-04]
- Sharing graph-like structure: `Box` cannot share one subtree between two lists (move/E0382); `Rc::clone` increments count, not deep copy — conventionally call `Rc::clone(&a)` to signal cheap refcount increment. [source: Book ch15-04]
- `Rc` provides immutable shared access; mutation layered via `RefCell` later in chapter (interior mutability). [source: Book ch15-04]
- `Rc::strong_count` observes refcount; drops decrement automatically via `Drop`. [source: Book ch15-04]

## [TAG: 07-async-concurrency] Fearless concurrency — chapter overview (Book ch16 intro)
- Ownership + types catch many concurrency mistakes at compile time (nickname: fearless concurrency). [source: Book ch16-00]
- Chapter covers threads, message passing, shared state, `Send`/`Sync`. [source: Book ch16-00]

## [TAG: 03-idioms] Patterns — chapter overview (Book ch19 intro)
- Patterns match structure: literals, destructuring, variables, wildcards, placeholders. [source: Book ch19-00]
- Used with `match` and other pattern positions to control flow by shape. [source: Book ch19-00]

## [TAG: 03-idioms] Containers — std overview
- `Option`/`Result` + `Iterator` are core abstract types for optionality, errors, and loops. [source: std root]
- Contiguous memory triad: `Vec<T>`, `[T; N]`, `[T]` slice; slices always behind pointer (`&[T]`, `&mut [T]`, `Box<[T]>`). [source: std root]
- `String` owns UTF-8 buffer; `str` is UTF-8 slice; `format!` builds `String`; `FromStr` parses. [source: std root]
- Sharing: `Rc`/`Arc`; mutation through shared ownership often pairs `Cell`/`RefCell` (single-threaded) or `Mutex` (concurrent) per std overview narrative. [source: std root]
- `collections` module: `HashMap`, sets, linked list, etc. [source: std root]

## [TAG: 07-async-concurrency] Platform / I/O modules (std overview)
- `thread` for OS threads; `sync` includes `atomic`, `mpmc`, `mpsc` channels. [source: std root]

## [TAG: 06-error-handling] Error E0106 — expanded fix checklist
- Official summary: see rustc --explain E0106 and error index page. [source: error_codes/E0106.html]
- Missing lifetime in type position or ambiguous output borrow.
- Fix structs/enums: add `'a` to wrapper and fields.
- Elision failure: add `'a` to function when multiple input lifetimes and return borrows from inputs.

## [TAG: 06-error-handling] Error E0277 — expanded fix checklist
- Official summary: see rustc --explain E0277 and error index page. [source: error_codes/E0277.html]
- Trait bound not satisfied.
- Implement trait for type or add `where T: Trait` to generic definition.

## [TAG: 06-error-handling] Error E0308 — expanded fix checklist
- Official summary: see rustc --explain E0308 and error index page. [source: error_codes/E0308.html]
- Types disagree — follow compiler expected/found annotations.

## [TAG: 06-error-handling] Error E0382 — expanded fix checklist
- Official summary: see rustc --explain E0382 and error index page. [source: error_codes/E0382.html]
- Moved value — use references, `clone`, or shared ownership per error text.

## [TAG: 06-error-handling] Error E0502 — expanded fix checklist
- Official summary: see rustc --explain E0502 and error index page. [source: error_codes/E0502.html]
- Mutability conflict — sequence borrows so exclusive `&mut` does not overlap incompatible `&`.

## [TAG: 06-error-handling] Error E0506 — expanded fix checklist
- Official summary: see rustc --explain E0506 and error index page. [source: error_codes/E0506.html]
- Assign to owner while borrowed — shrink borrow scope.

## [TAG: 06-error-handling] Error E0515 — expanded fix checklist
- Official summary: see rustc --explain E0515 and error index page. [source: error_codes/E0515.html]
- Return reference to local — return owned data.

## [TAG: 06-error-handling] Error E0597 — expanded fix checklist
- Official summary: see rustc --explain E0597 and error index page. [source: error_codes/E0597.html]
- Borrow outlives borrowed value — extend storage lifetime.

## [TAG: 06-error-handling] Error E0658 — expanded fix checklist
- Official summary: see rustc --explain E0658 and error index page. [source: error_codes/E0658.html]
- Unstable — nightly + feature gate or remove usage.

## [TAG: 06-error-handling] Error E0716 — expanded fix checklist
- Official summary: see rustc --explain E0716 and error index page. [source: error_codes/E0716.html]
- Temporary dropped too early — bind to `let` or rely on block-extended temp rules from diagnostics.

## [TAG: 01-meta-principles] Book ch01–ch03 — checkpoint
- Installation, Hello World, Cargo, guessing game: Cargo runs `main`, crates are units of compilation. [source: Book chapter ch01–ch03]

## [TAG: 02-language-rules] Book ch03 — checkpoint
- Variables: `let` immutable by default; `mut` for rebinding mutation; shadowing allows type change. [source: Book chapter ch03]

## [TAG: 03-idioms] Book ch03 — checkpoint
- Scalar types: integers, floats, bool, char; compound: tuple, array fixed `[T;N]`. [source: Book chapter ch03]

## [TAG: 02-language-rules] Book ch04 — checkpoint
- Ownership chapter: stack/heap, moves, `clone`, `Copy`, function scope transfers. [source: Book chapter ch04]

## [TAG: 04-design-patterns] Book ch05 — checkpoint
- Structs: field names, init shorthand, update syntax `..other`; methods take `self` by value/ref/mut ref. [source: Book chapter ch05]

## [TAG: 03-idioms] Book ch06 — checkpoint
- Enums: variants with data, `Option`, `match` exhaustiveness, `if let`. [source: Book chapter ch06]

## [TAG: 03-idioms] Book ch07 — checkpoint
- Modules: privacy, `use`, paths, file split with `mod foo;`. [source: Book chapter ch07]

## [TAG: 03-idioms] Book ch08 — checkpoint
- `Vec`/`String`/`HashMap`: resizing, UTF-8 text, entry API patterns in book. [source: Book chapter ch08]

## [TAG: 06-error-handling] Book ch09 — checkpoint
- `panic!` backtrace, `Result`, `?`, `map_err`, propagating errors. [source: Book chapter ch09]

## [TAG: 02-language-rules] Book ch10 — checkpoint
- Generics, traits, trait bounds, lifetime annotations, lifetime elision discussion. [source: Book chapter ch10]

## [TAG: 10-testing-and-tooling] Book ch11 — checkpoint
- Unit tests in same file module, integration tests in `tests/`, `cargo test` options. [source: Book chapter ch11]

## [TAG: 06-error-handling] Book ch12 — checkpoint
- Minigrep: `Box<dyn Error>`, splitting lib/binary, `println!` vs `eprintln!`. [source: Book chapter ch12]

## [TAG: 03-idioms] Book ch13 — checkpoint
- Closures: `Fn*` traits, `Iterator` adapters, performance notes. [source: Book chapter ch13]

## [TAG: 10-testing-and-tooling] Book ch14 — checkpoint
- Cargo profiles, workspaces, publishing (book chapter — follow rustdoc for details). [source: Book chapter ch14]

## [TAG: 04-design-patterns] Book ch15 — checkpoint
- Smart pointers: `Box`, `Deref`/`Drop`, `Rc`, `RefCell`, cycles/`Weak`. [source: Book chapter ch15]

## [TAG: 07-async-concurrency] Book ch16 — checkpoint
- Threads, channels, mutex, `Send`/`Sync`. [source: Book chapter ch16]

## [TAG: 07-async-concurrency] Book ch17 — checkpoint
- Async/await, futures, streams — book uses `trpl` for teaching. [source: Book chapter ch17]

## [TAG: 04-design-patterns] Book ch18 — checkpoint
- OOP traits, trait objects `dyn`, state pattern vs encoding in types. [source: Book chapter ch18]

## [TAG: 03-idioms] Book ch19 — checkpoint
- Pattern locations, refutability, syntax reference. [source: Book chapter ch19]

## [TAG: 08-unsafe-and-ffi] Book ch20 — checkpoint
- Unsafe superpowers, FFI, statics, Miri. [source: Book chapter ch20]

## [TAG: 07-async-concurrency] Book ch21 — checkpoint
- Multithreaded server ties prior concepts together. [source: Book chapter ch21]

## [TAG: 10-testing-and-tooling] Style Guide — items chapter highlights
- Item order: `extern crate` first alphabetically; then `use` then `mod` declarations; version-sort imports; `self`/`super` first in lists; globs last. [source: Style Guide items]
- Function signatures: break after `(` if multiline args; trailing comma; avoid comments inside signature. [source: Style Guide items]
- Enum variants: one per line unless small-variant rule applies. [source: Style Guide items]
- Generics: prefer single-line `<T: Bound>`; if large, use `where` clause per guide. [source: Style Guide items]
- Where clauses: each predicate on own line when broken; trailing comma rules. [source: Style Guide items]

## [TAG: 10-testing-and-tooling] rustfmt / default style
- Default Rust style (100 cols, 4-space indent, trailing commas) is what `rustfmt` implements by default; mismatches may be bugs. [source: Style Guide intro]

## [TAG: 03-idioms] `std::iter` — additional adapter semantics (from trait docs)
- `chain` sequences iterators; `zip` pairs elements; length ends at shorter iterator when lengths differ (documented in iterator sources). [source: std::iter::Iterator]
- `enumerate` yields `(idx, item)`; `peekable` allows one-step lookahead. [source: std::iter::Iterator]
- `try_fold`/`try_for_each` short-circuit on `Try` types — useful for `Result` inside loops without manual `?` in closure body in some patterns. [source: std::iter::Iterator]

## [TAG: 09-performance] `std::hint`
- Compiler hints module exists for optimization (`black_box`, etc.); see std docs for stability. [source: std module list from std root]

## [TAG: 09-performance] `std::mem` functions (inventory)
- `mem::align_of` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::align_of_val` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::align_of_val_raw` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::discriminant` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::drop` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::forget` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::forget_unsized` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::replace` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::size_of` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::size_of_val` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::size_of_val_raw` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::swap` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::take` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::transmute` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::transmute_copy` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::needs_drop` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::zeroed` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::uninitialized` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::offset_of` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::copy` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::variant_count` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `mem::conjure_zst` — see rustdoc for safety/preconditions (several are `unsafe` or have narrow contracts). [source: https://doc.rust-lang.org/std/mem/index.html]
- `MaybeUninit` union for uninitialized memory; `ManuallyDrop` to suppress destructor. [source: std::mem]

## [TAG: 02-language-rules] Rust keywords — std keyword pages
- Keyword `as` — `std::keyword.as` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `async` — `std::keyword.async` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `await` — `std::keyword.await` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `break` — `std::keyword.break` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `const` — `std::keyword.const` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `continue` — `std::keyword.continue` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `crate` — `std::keyword.crate` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `dyn` — `std::keyword.dyn` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `else` — `std::keyword.else` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `enum` — `std::keyword.enum` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `extern` — `std::keyword.extern` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `false` — `std::keyword.false` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `fn` — `std::keyword.fn` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `for` — `std::keyword.for` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `if` — `std::keyword.if` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `impl` — `std::keyword.impl` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `in` — `std::keyword.in` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `let` — `std::keyword.let` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `loop` — `std::keyword.loop` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `match` — `std::keyword.match` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `mod` — `std::keyword.mod` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `move` — `std::keyword.move` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `mut` — `std::keyword.mut` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `pub` — `std::keyword.pub` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `ref` — `std::keyword.ref` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `return` — `std::keyword.return` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `self` — `std::keyword.self` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `Self` — `std::keyword.Self` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `static` — `std::keyword.static` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `struct` — `std::keyword.struct` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `super` — `std::keyword.super` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `trait` — `std::keyword.trait` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `true` — `std::keyword.true` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `type` — `std::keyword.type` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `union` — `std::keyword.union` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `unsafe` — `std::keyword.unsafe` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `use` — `std::keyword.use` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `where` — `std::keyword.where` links into Reference/Book where applicable. [source: std Keywords section]
- Keyword `while` — `std::keyword.while` links into Reference/Book where applicable. [source: std Keywords section]

## [TAG: 02-language-rules] Primitive types — rustdoc entry points
- `std::primitive::bool` documents methods and related modules. [source: std primitives list]
- `std::primitive::char` documents methods and related modules. [source: std primitives list]
- `std::primitive::f32` documents methods and related modules. [source: std primitives list]
- `std::primitive::f64` documents methods and related modules. [source: std primitives list]
- `std::primitive::f16` documents methods and related modules. [source: std primitives list]
- `std::primitive::f128` documents methods and related modules. [source: std primitives list]
- `std::primitive::i8` documents methods and related modules. [source: std primitives list]
- `std::primitive::i16` documents methods and related modules. [source: std primitives list]
- `std::primitive::i32` documents methods and related modules. [source: std primitives list]
- `std::primitive::i64` documents methods and related modules. [source: std primitives list]
- `std::primitive::i128` documents methods and related modules. [source: std primitives list]
- `std::primitive::isize` documents methods and related modules. [source: std primitives list]
- `std::primitive::u8` documents methods and related modules. [source: std primitives list]
- `std::primitive::u16` documents methods and related modules. [source: std primitives list]
- `std::primitive::u32` documents methods and related modules. [source: std primitives list]
- `std::primitive::u64` documents methods and related modules. [source: std primitives list]
- `std::primitive::u128` documents methods and related modules. [source: std primitives list]
- `std::primitive::usize` documents methods and related modules. [source: std primitives list]
- `std::primitive::str` documents methods and related modules. [source: std primitives list]
- `std::primitive::slice` documents methods and related modules. [source: std primitives list]
- `std::primitive::array` documents methods and related modules. [source: std primitives list]
- `std::primitive::pointer` documents methods and related modules. [source: std primitives list]
- `std::primitive::reference` documents methods and related modules. [source: std primitives list]
- `std::primitive::fn` documents methods and related modules. [source: std primitives list]
- `std::primitive::tuple` documents methods and related modules. [source: std primitives list]
- `std::primitive::unit` documents methods and related modules. [source: std primitives list]
- `std::primitive::never` documents methods and related modules. [source: std primitives list]

## [TAG: 10-testing-and-tooling] Std macros — name index (see rustdoc for each)
- `assert!` — documented under `std::macro.assert` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `assert_eq!` — documented under `std::macro.assert_eq` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `assert_ne!` — documented under `std::macro.assert_ne` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `cfg!` — documented under `std::macro.cfg` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `cfg_select!` — documented under `std::macro.cfg_select` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `column!` — documented under `std::macro.column` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `compile_error!` — documented under `std::macro.compile_error` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `concat!` — documented under `std::macro.concat` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `dbg!` — documented under `std::macro.dbg` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `debug_assert!` — documented under `std::macro.debug_assert` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `debug_assert_eq!` — documented under `std::macro.debug_assert_eq` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `debug_assert_ne!` — documented under `std::macro.debug_assert_ne` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `env!` — documented under `std::macro.env` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `eprint!` — documented under `std::macro.eprint` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `eprintln!` — documented under `std::macro.eprintln` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `file!` — documented under `std::macro.file` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `format!` — documented under `std::macro.format` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `format_args!` — documented under `std::macro.format_args` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `include!` — documented under `std::macro.include` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `include_bytes!` — documented under `std::macro.include_bytes` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `include_str!` — documented under `std::macro.include_str` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `is_x86_feature_detected!` — documented under `std::macro.is_x86_feature_detected` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `line!` — documented under `std::macro.line` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `matches!` — documented under `std::macro.matches` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `module_path!` — documented under `std::macro.module_path` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `option_env!` — documented under `std::macro.option_env` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `panic!` — documented under `std::macro.panic` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `print!` — documented under `std::macro.print` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `println!` — documented under `std::macro.println` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `stringify!` — documented under `std::macro.stringify` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `thread_local!` — documented under `std::macro.thread_local` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `todo!` — documented under `std::macro.todo` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `try!` — documented under `std::macro.try` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `unimplemented!` — documented under `std::macro.unimplemented` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `unreachable!` — documented under `std::macro.unreachable` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `vec!` — documented under `std::macro.vec` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `write!` — documented under `std::macro.write` (or `std` prelude); expansion is compile-time. [source: std Macros list]
- `writeln!` — documented under `std::macro.writeln` (or `std` prelude); expansion is compile-time. [source: std Macros list]

## [TAG: 08-unsafe-and-ffi] `std::ffi` / `std::ptr` pointers
- FFI utilities and raw pointer management live in dedicated modules; cross-reference Nomicon for soundness (Book defers deep unsafe to Rustonomicon). [source: Book ch20 + std modules list]

## [TAG: 12-modern-rust] Book edition notice
- Book assumes `edition = "2024"` in Cargo.toml for all projects (front matter). [source: Book index]

---

## [TAG: 10-testing-and-tooling] Documenting unsafe obligations
- Book recommends `SAFETY:` comments on `unsafe` blocks and unsafe functions explaining caller/implementation proof. [source: Book ch20]

---

## Part III — Error index excerpts (official rustc pages)

### E0106 — missing lifetime (excerpt)
> This error indicates that a lifetime is missing from a type. If it is an error inside a function signature, the problem may be with failing to adhere to the lifetime elision rules.
> Lifetime elision is a special, limited kind of inference for lifetimes in function signatures which allows you to leave out lifetimes in certain cases.
> The lifetime elision rules require that any function signature with an elided output lifetime must either have: exactly one input lifetime; or, multiple input lifetimes, but the function must also be a method with a `&self` or `&mut self` receiver.
> In the first case, the output lifetime is inferred to be the same as the unique input lifetime. In the second case, the lifetime is instead inferred to be the same as the lifetime on `&self` or `&mut self`.
[source: https://doc.rust-lang.org/error_codes/E0106.html]

### E0277 — trait not satisfied (excerpt)
> You tried to use a type which doesn't implement some trait in a place which expected that trait.
> Note that the error here is in the definition of the generic function. Although we only call it with a parameter that does implement `Debug`, the compiler still rejects the function. It must work with all possible input types.
[source: https://doc.rust-lang.org/error_codes/E0277.html]

### E0308 — mismatched types (excerpt)
> Expected type did not match the received type.
> This error occurs when an expression was used in a place where the compiler expected an expression of a different type.
[source: https://doc.rust-lang.org/error_codes/E0308.html]

### E0382 — borrow of moved value (excerpt)
> A variable was used after its contents have been moved elsewhere.
> Since `MyStruct` is a type that is not marked `Copy`, the data gets moved out of `x` when we set `y`. This is fundamental to Rust's ownership system: outside of workarounds like `Rc`, a value cannot be owned by more than one variable.
[source: https://doc.rust-lang.org/error_codes/E0382.html]

### E0502 — conflicting borrows (excerpt)
> A variable already borrowed with a certain mutability (either mutable or immutable) was borrowed again with a different mutability.
[source: https://doc.rust-lang.org/error_codes/E0502.html]

### E0506 — assign to borrowed value (excerpt)
> An attempt was made to assign to a borrowed value.
> Because `fancy_ref` still holds a reference to `fancy_num`, `fancy_num` can't be assigned to a new value as it would invalidate the reference.
[source: https://doc.rust-lang.org/error_codes/E0506.html]

### E0515 — reference to local (excerpt)
> A reference to a local variable was returned.
> Local variables, function parameters and temporaries are all dropped before the end of the function body. A returned reference (or struct containing a reference) to such a dropped value would immediately be invalid.
[source: https://doc.rust-lang.org/error_codes/E0515.html]

### E0597 — does not live long enough (excerpt)
> This error occurs because a value was dropped while it was still borrowed.
[source: https://doc.rust-lang.org/error_codes/E0597.html]

### E0658 — unstable feature (excerpt)
> An unstable feature was used.
> If you're using a stable or a beta version of rustc, you won't be able to use any unstable features. In order to do so, please switch to a nightly version of rustc (by using rustup).
[source: https://doc.rust-lang.org/error_codes/E0658.html]

### E0716 — temporary dropped while borrowed (excerpt)
> A temporary value is being dropped while a borrow is still in active use.
> Ordinarily, the temporary is dropped at the end of the enclosing statement — in this case, after the `let p`.
> Temporaries are not always dropped at the end of the enclosing statement. In simple cases where the `&` expression is immediately stored into a variable, the compiler will automatically extend the lifetime of the temporary until the end of the enclosing block.
[source: https://doc.rust-lang.org/error_codes/E0716.html]

---

## [TAG: 03-idioms] Std `std::*` modules — navigation pointers
- Each entry: open `https://doc.rust-lang.org/std/<name>/` (module index). [source: std crate root Modules section]
- `alloc` — module-level overview + types in rustdoc. [source: std]
- `any` — module-level overview + types in rustdoc. [source: std]
- `arch` — module-level overview + types in rustdoc. [source: std]
- `array` — module-level overview + types in rustdoc. [source: std]
- `ascii` — module-level overview + types in rustdoc. [source: std]
- `backtrace` — module-level overview + types in rustdoc. [source: std]
- `borrow` — module-level overview + types in rustdoc. [source: std]
- `boxed` — module-level overview + types in rustdoc. [source: std]
- `cell` — module-level overview + types in rustdoc. [source: std]
- `char` — module-level overview + types in rustdoc. [source: std]
- `clone` — module-level overview + types in rustdoc. [source: std]
- `cmp` — module-level overview + types in rustdoc. [source: std]
- `collections` — module-level overview + types in rustdoc. [source: std]
- `convert` — module-level overview + types in rustdoc. [source: std]
- `default` — module-level overview + types in rustdoc. [source: std]
- `env` — module-level overview + types in rustdoc. [source: std]
- `error` — module-level overview + types in rustdoc. [source: std]
- `ffi` — module-level overview + types in rustdoc. [source: std]
- `fmt` — module-level overview + types in rustdoc. [source: std]
- `fs` — module-level overview + types in rustdoc. [source: std]
- `future` — module-level overview + types in rustdoc. [source: std]
- `hash` — module-level overview + types in rustdoc. [source: std]
- `hint` — module-level overview + types in rustdoc. [source: std]
- `io` — module-level overview + types in rustdoc. [source: std]
- `iter` — module-level overview + types in rustdoc. [source: std]
- `marker` — module-level overview + types in rustdoc. [source: std]
- `mem` — module-level overview + types in rustdoc. [source: std]
- `net` — module-level overview + types in rustdoc. [source: std]
- `num` — module-level overview + types in rustdoc. [source: std]
- `ops` — module-level overview + types in rustdoc. [source: std]
- `option` — module-level overview + types in rustdoc. [source: std]
- `os` — module-level overview + types in rustdoc. [source: std]
- `panic` — module-level overview + types in rustdoc. [source: std]
- `path` — module-level overview + types in rustdoc. [source: std]
- `pin` — module-level overview + types in rustdoc. [source: std]
- `prelude` — module-level overview + types in rustdoc. [source: std]
- `primitive` — module-level overview + types in rustdoc. [source: std]
- `process` — module-level overview + types in rustdoc. [source: std]
- `ptr` — module-level overview + types in rustdoc. [source: std]
- `range` — module-level overview + types in rustdoc. [source: std]
- `rc` — module-level overview + types in rustdoc. [source: std]
- `result` — module-level overview + types in rustdoc. [source: std]
- `slice` — module-level overview + types in rustdoc. [source: std]
- `str` — module-level overview + types in rustdoc. [source: std]
- `string` — module-level overview + types in rustdoc. [source: std]
- `sync` — module-level overview + types in rustdoc. [source: std]
- `task` — module-level overview + types in rustdoc. [source: std]
- `thread` — module-level overview + types in rustdoc. [source: std]
- `time` — module-level overview + types in rustdoc. [source: std]
- `vec` — module-level overview + types in rustdoc. [source: std]
- `async_iter` — module-level overview + types in rustdoc. [source: std]
- `autodiff` — module-level overview + types in rustdoc. [source: std]
- `bstr` — module-level overview + types in rustdoc. [source: std]
- `f16` — module-level overview + types in rustdoc. [source: std]
- `f128` — module-level overview + types in rustdoc. [source: std]
- `from` — module-level overview + types in rustdoc. [source: std]
- `intrinsics` — module-level overview + types in rustdoc. [source: std]
- `pat` — module-level overview + types in rustdoc. [source: std]
- `random` — module-level overview + types in rustdoc. [source: std]
- `simd` — module-level overview + types in rustdoc. [source: std]
- `unsafe_binder` — module-level overview + types in rustdoc. [source: std]

## [TAG: 10-testing-and-tooling] Std macros — quick map
- `compile_error!`, `concat!`, `env!`, `include!`, `include_str!`, `include_bytes!`, `format!`, `print!`/`println!`, `eprint!`/`eprintln!`, `dbg!`, `assert!` family, `vec!`, `matches!`, `thread_local!`, `todo!`, `unimplemented!`, `unreachable!` — see each macro page for expansion & const-ness. [source: std Macros list]

## [TAG: 02-language-rules] Std keywords index — cross-links
- Keywords documented under `std::keyword.*` in rustdoc link to Book/Reference sections (e.g. `unsafe` -> Book unsafe chapter). [source: std Keywords section]

## [TAG: 02-language-rules] Orphan rule, coherence, newtype (Book ch10 + ch20)
- You cannot implement an external trait for an external type when neither is local to your crate — orphan rule prevents conflicting impls across crates. [source: Book ch10]
- Purpose: others cannot break your code by adding impls; Rust knows which impl applies. [source: Book ch10]
- Newtype pattern: tuple struct wrapper makes type local so trait impl becomes allowed; zero-cost at runtime. [source: Book ch20 Advanced Types]
- Newtype downside: must re-expose methods or implement `Deref` to delegate to inner type if full API needed. [source: Book ch20]

## [TAG: 02-language-rules] Advanced traits — associated types (Book ch20)
- `Iterator` has associated type `Item`; implementors fix `Item` concrete type while `next` returns `Option<Item>`. [source: Book ch20 + std::iter::Iterator]
- Associated types differ from generics: fewer type parameters at call sites; often clearer for trait with one logical output type. [source: Book ch20]

## [TAG: 03-idioms] Slices and UTF-8 (Book ch4 + ch8 cross-note)
- String slice `&str` is UTF-8; cannot index by byte without considering character boundaries (Book develops in ch8). [source: Book ch4 slice intro]
- Slice is fat pointer: address + length; does not own. [source: Book ch4]

## [TAG: 06-error-handling] Error handling — chapter framing (Book ch9 intro)
- Rust forces acknowledging errors before deployment; distinguishes recoverable vs unrecoverable. [source: Book ch09-00]
- Recoverable: e.g. file not found — report/retry. Unrecoverable: bugs like out-of-bounds — stop program. [source: Book ch09-00]
- No exceptions: `Result<T,E>` for recoverable; `panic!` for unrecoverable. [source: Book ch09-00]

## [TAG: 02-language-rules] Patterns — refutability (Book ch19)
- Irrefutable patterns always match; refutable patterns may fail (`Some(x)` vs `None`). [source: Book ch19-02]
- `let`, function params, `for` require irrefutable patterns. [source: Book ch19-02]
- `if let`, `while let`, `let...else` accept refutable patterns; warn on irrefutable `let...else`. [source: Book ch19-02]
- Mismatch yields E0005 (refutable pattern in local binding) — use `let...else`, `match`, or cover `None`. [source: Book ch19-02]

## [TAG: 06-error-handling] Errors referenced in Book ch4 examples
- E0596: cannot borrow as mutable behind shared reference — use `&mut`. [source: Book ch4.2]
- E0499: cannot borrow as mutable more than once at a time. [source: Book ch4.2]
- E0502: cannot borrow mutably because also borrowed immutably (Book's `r1`,`r2`,`r3` example). [source: Book ch4.2]

## [TAG: 03-idioms] Iterator methods — compact index (duplicate for grep-ability)
- `next_chunk()` [source: std::iter::Iterator]
- `size_hint()` [source: std::iter::Iterator]
- `count()` [source: std::iter::Iterator]
- `last()` [source: std::iter::Iterator]
- `advance_by()` [source: std::iter::Iterator]
- `nth()` [source: std::iter::Iterator]
- `step_by()` [source: std::iter::Iterator]
- `chain()` [source: std::iter::Iterator]
- `zip()` [source: std::iter::Iterator]
- `intersperse()` [source: std::iter::Iterator]
- `intersperse_with()` [source: std::iter::Iterator]
- `map()` [source: std::iter::Iterator]
- `for_each()` [source: std::iter::Iterator]
- `filter()` [source: std::iter::Iterator]
- `filter_map()` [source: std::iter::Iterator]
- `enumerate()` [source: std::iter::Iterator]
- `peekable()` [source: std::iter::Iterator]
- `skip_while()` [source: std::iter::Iterator]
- `take_while()` [source: std::iter::Iterator]
- `map_while()` [source: std::iter::Iterator]
- `skip()` [source: std::iter::Iterator]
- `take()` [source: std::iter::Iterator]
- `scan()` [source: std::iter::Iterator]
- `flat_map()` [source: std::iter::Iterator]
- `flatten()` [source: std::iter::Iterator]
- `map_windows()` [source: std::iter::Iterator]
- `fuse()` [source: std::iter::Iterator]
- `inspect()` [source: std::iter::Iterator]
- `by_ref()` [source: std::iter::Iterator]
- `collect()` [source: std::iter::Iterator]
- `try_collect()` [source: std::iter::Iterator]
- `collect_into()` [source: std::iter::Iterator]
- `partition()` [source: std::iter::Iterator]
- `partition_in_place()` [source: std::iter::Iterator]
- `is_partitioned()` [source: std::iter::Iterator]
- `try_fold()` [source: std::iter::Iterator]
- `try_for_each()` [source: std::iter::Iterator]
- `fold()` [source: std::iter::Iterator]
- `reduce()` [source: std::iter::Iterator]
- `try_reduce()` [source: std::iter::Iterator]
- `all()` [source: std::iter::Iterator]
- `any()` [source: std::iter::Iterator]
- `find()` [source: std::iter::Iterator]
- `find_map()` [source: std::iter::Iterator]
- `try_find()` [source: std::iter::Iterator]
- `position()` [source: std::iter::Iterator]
- `rposition()` [source: std::iter::Iterator]
- `max()` [source: std::iter::Iterator]
- `min()` [source: std::iter::Iterator]
- `max_by_key()` [source: std::iter::Iterator]
- `max_by()` [source: std::iter::Iterator]
- `min_by_key()` [source: std::iter::Iterator]
- `min_by()` [source: std::iter::Iterator]
- `rev()` [source: std::iter::Iterator]
- `unzip()` [source: std::iter::Iterator]
- `copied()` [source: std::iter::Iterator]
- `cloned()` [source: std::iter::Iterator]
- `cycle()` [source: std::iter::Iterator]
- `array_chunks()` [source: std::iter::Iterator]
- `sum()` [source: std::iter::Iterator]
- `product()` [source: std::iter::Iterator]

---

## Appendix — concept → official doc map (navigation)
- **Ownership / move / drop** → Book ch4; std::ops::Drop.
- **References / slices** → Book ch4; std::primitive::reference, std::slice.
- **Generics / traits / bounds** → Book ch10; std::marker, std::cmp::PartialOrd, ….
- **Lifetimes** → Book ch10; Reference + rustc error index E0xxx.
- **Error handling** → Book ch9; std::result, std::option.
- **Testing** → Book ch11; std::macro::assert*.
- **Iterators / closures** → Book ch13; std::iter.
- **Smart pointers** → Book ch15; std::boxed, std::rc, std::sync::Arc.
- **Concurrency** → Book ch16; std::thread, std::sync.
- **Async** → Book ch17; std::future, std::task.
- **Patterns** → Book ch19; Reference patterns chapter.
- **Unsafe / FFI** → Book ch20; std::ffi, std::ptr, `unsafe` keyword page.
- **Macros** → Book ch20; std::macros, RBE macro_rules.
- **Collections** → Book ch8; std::collections.
- **Strings** → Book ch8; std::string, std::primitive::str.
- **Modules / crates** → Book ch7; Reference modules.
- **Cargo / workspace** → Book ch14; doc.rust-lang.org/cargo (not duplicated here).
- **Edition migrations** → Edition Guide; rustc book lints.
- **Formatting style** → Style Guide; rustfmt.
- **Send/Sync** → Book ch16 + std; std::marker::{Send, Sync}.
- **Cow / borrowing** → std docs; std::borrow::Cow.
- **Memory layout / transmute** → Book ch20; std::mem.

---

## Part IV — Addendum (deep pass, official docs only)

### [TAG: 02-language-rules] [TAG: 04-design-patterns] Interior mutability (`std::cell`)

- Baseline rule: for an object `T`, either many `&T` **or** one `&mut T` — compiler-enforced. [source: std::cell module]
- `Cell` / `RefCell` / `OnceCell` / `LazyCell` allow controlled mutation under aliasing; they are **not** `Sync`. For threads use `Mutex`, `RwLock`, `OnceLock`, or `atomic`. [source: std::cell]
- **Interior mutability**: mutate through shared `&T`; **inherited mutability**: only via `&mut T` (typical types). [source: std::cell]
- `Cell<T>`: move values in/out; no `&T` to inner; `set` / `replace` / `into_inner`. Prefer for cheap `Copy` types. [source: std::cell]
- `RefCell<T>`: **dynamic borrowing** at runtime; `borrow` / `borrow_mut` panic if rules violated. [source: std::cell]
- `OnceCell` / `LazyCell`: set-once / lazy init; `Sync` counterparts `OnceLock` / `LazyLock`. [source: std::cell]
- `UnsafeCell` is the core primitive. [source: std::cell]
- Interior mutability is a **last resort**; inherited mutability preferred. Legitimate uses: `Rc`/`Arc` + `RefCell`; hidden cache in `&self` API; `Clone` adjusting counts via `Cell`. [source: std::cell]
- `Rc<RefCell<_>>`: borrows must not overlap — dynamic borrow panic hazard. [source: std::cell]
- `RefCell` ↔ `RwLock`: single-threaded vs multi-threaded dynamic borrowing. [source: std::cell]

### [TAG: 07-async-concurrency] [TAG: 04-design-patterns] `Arc<T>`

- Atomically reference-counted heap value; `clone` increments; last drop destroys inner. [source: std::sync::Arc]
- No `&mut` inside by default; use `Mutex`/`RwLock`/atomics, or `Arc::make_mut` / `Arc::get_mut` per docs. [source: std::sync::Arc]
- `Rc` cheaper single-threaded; `Arc` uses atomics. [source: std::sync::Arc]
- `Arc<T>: Send + Sync` requires `T: Send + Sync`. [source: std::sync::Arc]
- Strong `Arc` cycles leak; `Weak` breaks cycles (`downgrade`/`upgrade`). [source: std::sync::Arc]
- Requires atomic pointer ops on platform. [source: std::sync::Arc]

### [TAG: 03-idioms] [TAG: 06-error-handling] `Option<T>` reminders
- Combinators and predicates in rustdoc (`map`, `and_then`, `is_some_and`, …). [source: std::option::Option]

### [TAG: 12-modern-rust] [TAG: 01-meta-principles] Editions (Book Appendix E)
- Six-week releases; edition ~three years. [source: Book appendix-05-editions]
- Editions: 2015, 2018, 2021, 2024; Book uses 2024. [source: Book appendix-05-editions]
- `edition` in Cargo.toml; default 2015 if missing. [source: Book appendix-05-editions]
- Mixed-edition crates link; edition affects parsing. [source: Book appendix-05-editions]
- `cargo fix` + Edition Guide for migration. [source: Book appendix-05-editions]

---

## Part V — Deep dive (`Pin` / `Unpin`, `Drop` / `ManuallyDrop`, `IntoIterator`, `Cow`, dyn compatibility)

### [TAG: 02-language-rules] [TAG: 07-async-concurrency] `Pin<Ptr>` and `Unpin`
- `Pin` wraps a pointer `Ptr` and pins the **pointee** in memory so it is not moved or otherwise invalidated there unless the pointee type implements `Unpin`. [source: std::pin::Pin]
- The pinned value is not stored inside `Pin` — `Pin` holds a pointer to it. [source: std::pin::Pin]
- `Future::poll` takes `self: Pin<&mut Self>` so async state machines (possible self-references) can rely on pinning. [source: std::pin::Pin]
- For `T: Unpin`, `Pin::new` on any pointer to `T` is safe and pinning has no effect. [source: std::pin::Pin]
- For `!Unpin`, typical construction: `Box::pin`, `Box::into_pin`, stack `pin!`, or other std smart-pointer helpers (`Rc`/`Arc` per docs). [source: std::pin::Pin]
- `Pin` has the same layout and ABI as `Ptr`. [source: std::pin::Pin]
- `Unpin` is an auto trait: types that do not rely on pinning invariants; almost all types get it unless opted out (e.g. `PhantomPinned`). [source: std::marker::Unpin]
- Types that **must** rely on pinning for soundness should not be `Unpin` (e.g. add `PhantomPinned`). [source: std::marker::Unpin]
- `mem::replace` applies to any `&mut T`; pinning prevents safe `&mut` to `!Unpin` pointee through `Pin`, which is what makes the pin contract meaningful. [source: std::marker::Unpin]

### [TAG: 04-design-patterns] [TAG: 08-unsafe-and-ffi] `Drop` and destructor glue
- Destruction runs `Drop::drop` when implemented, plus compiler “drop glue” for fields. [source: std::ops::Drop]
- You must not call `Drop::drop` explicitly; use `mem::drop` / scope end (compiler error E0040 for explicit `.drop()`). [source: std::ops::Drop + error_codes/E0040]
- Struct fields drop in declaration order; local variables drop in **reverse** declaration order. [source: std::ops::Drop]
- `Copy` and `Drop` cannot both be implemented. [source: std::ops::Drop]
- “Drop check” constrains when borrows must remain live across implicit drop; details evolving — see Reference/Nomicon. [source: std::ops::Drop]
- `drop` should generally avoid panicking; if it panics during unwinding, double-panic may abort — consider `std::thread::panicking()`. [source: std::ops::Drop]

### [TAG: 08-unsafe-and-ffi] `ManuallyDrop<T>`
- Wrapper that inhibits automatic `T` destructor; zero-cost; same layout/bit validity as `T`. [source: std::mem::ManuallyDrop]
- Safe to access inner value; exposing a dropped `ManuallyDrop` through public safe API is unsound — `ManuallyDrop::drop` is `unsafe`. [source: std::mem::ManuallyDrop]
- Prefer field declaration order for drop order; using `ManuallyDrop` to reorder drops needs `unsafe` and is easy to get wrong with unwinding. [source: std::mem::ManuallyDrop]
- Documented pitfall: `ManuallyDrop` containing `Box` (or `Box` inside `T`) then drop + move may be UB — evolving rules; `MaybeUninit` may be safer. [source: std::mem::ManuallyDrop]
- `take` / `into_inner` / `drop` have documented safety obligations (no double-drop, no use-after-drop). [source: std::mem::ManuallyDrop]

### [TAG: 03-idioms] `IntoIterator` and `for` loops
- Trait: associated types `Item` and `IntoIter: Iterator<Item = Self::Item>`; required `into_iter(self) -> Self::IntoIter`. [source: std::iter::IntoIterator]
- Implementing `IntoIterator` defines how a type participates in `for` loops. [source: std::iter::IntoIterator]
- Common bound pattern: `T: IntoIterator` with extra `Item` bounds; see also `FromIterator` for `collect`. [source: std::iter::IntoIterator]
- `impl IntoIterator for I where I: Iterator` — identity conversion. [source: std::iter::IntoIterator]

### [TAG: 03-idioms] [TAG: 04-design-patterns] `Cow<'a, B>`
- `Cow` is a clone-on-write smart pointer: borrows via `Borrowed(&'a B)`, owns via `Owned(<B as ToOwned>::Owned)`. [source: std::borrow::Cow]
- Implements `Deref` for read access; `to_mut` clones into owned storage when mutation is needed. [source: std::borrow::Cow]
- `Rc::make_mut` / `Arc::make_mut` provide refcounted COW patterns when appropriate. [source: std::borrow::Cow]

### [TAG: 02-language-rules] [TAG: 04-design-patterns] Dyn compatibility (trait objects; formerly “object safety”)
- A **dyn-compatible** trait can be the base trait of a trait object (`dyn Trait`). [source: Reference items/traits#dyn-compatibility]
- All supertraits must be dyn-compatible. [source: Reference items/traits#dyn-compatibility]
- `Self: Sized` must not be required as a supertrait. [source: Reference items/traits#dyn-compatibility]
- No associated constants. [source: Reference items/traits#dyn-compatibility]
- No associated types with generics. [source: Reference items/traits#dyn-compatibility]
- Associated functions must be dispatchable from a trait object (specific receiver forms including `&Self`, `&mut Self`, `Box<Self>`, `Rc<Self>`, `Arc<Self>`, `Pin<…>` per Reference) **or** be `where Self: Sized` non-methods. [source: Reference items/traits#dyn-compatibility]
- Dispatchable methods must not use opaque return types: not `async fn`, not return-position `impl Trait`. [source: Reference items/traits#dyn-compatibility]
- `AsyncFn` / `AsyncFnMut` / `AsyncFnOnce` are not dyn-compatible. [source: Reference items/traits#dyn-compatibility]

---

## Part V continued — Error index excerpts (additional codes)

### E0040 — explicit destructor call (excerpt)
> It is not allowed to manually call destructors in Rust.
> However, if you really need to drop a value by hand, you can use the `std::mem::drop` function
[source: https://doc.rust-lang.org/error_codes/E0040.html]

### E0499 — multiple mutable borrows (excerpt)
> A variable was borrowed as mutable more than once.
> Please note that in Rust, you can either have many immutable references, or one mutable reference.
[source: https://doc.rust-lang.org/error_codes/E0499.html]

### E0596 — mutably borrow immutable binding (excerpt)
> This error occurs because you tried to mutably borrow a non-mutable variable.
[source: https://doc.rust-lang.org/error_codes/E0596.html]

---

## Part VI — Collections / coercion / defaults (`FromIterator`, `Extend`, `Deref`, `AsRef`, `Default`, `PhantomPinned`)

### [TAG: 03-idioms] `FromIterator<A>`
- Defines how a **Sized** type is built from something `IntoIterator<Item = A>` — dual of consuming iteration. [source: std::iter::FromIterator]
- `Iterator::collect` uses `FromIterator` under the hood; `T::from_iter(iter)` can read clearer than turbofish on `collect`. [source: std::iter::FromIterator]
- See also `IntoIterator` for `for` loops and iterator sources. [source: std::iter::FromIterator]
- This trait is **not** dyn-compatible (formerly “not object safe”). [source: std::iter::FromIterator]
- `Result` / `Option` implement `FromIterator` to turn iterators of `Result`/`Option` into combined `Result`/`Option` (short-circuiting patterns). [source: std::iter::FromIterator implementors]

### [TAG: 03-idioms] `Extend<A>`
- Extends an **existing** collection from `IntoIterator<Item = A>` (mutating `self`). [source: std::iter::Extend]
- For maps/sets, extending with an existing key updates or inserts per collection semantics. [source: std::iter::Extend]
- Not dyn-compatible. [source: std::iter::Extend]

### [TAG: 04-design-patterns] `Deref` / `DerefMut` and coercion
- Used for immutable `*` deref and **deref coercion**: `&T` → `&Target`, and method resolution on `Target` for `&self` methods. [source: std::ops::Deref]
- Mutable contexts use `DerefMut` similarly. [source: std::ops::Deref]
- Implement when the type should transparently behave like `Target`, deref is cheap, and coercion surprises are acceptable — it is a large part of public API. [source: std::ops::Deref]
- Avoid if deref could fail unexpectedly, methods likely collide with `Target`, or coercion is not a stable contract. [source: std::ops::Deref]
- Book cross-link: custom smart pointers and `Deref` (ch15-02). [source: Book ch15-02 + std::ops::Deref]
- `Deref::deref` should be effectively infallible in normal use (implicit calls). [source: std::ops::Deref]

### [TAG: 03-idioms] `AsRef<T>` (and contrast with `Borrow`)
- Cheap **reference-to-reference** conversion: `fn as_ref(&self) -> &T`. [source: std::convert::AsRef]
- Prefer `From`/`Into` or custom helpers for **expensive** conversions. [source: std::convert::AsRef]
- Unlike `Borrow`, `AsRef` does not require `Hash`/`Eq`/`Ord` agreement with owned value — fine for projecting one field. [source: std::convert::AsRef]
- Must not fail; if conversion can fail, use `Option`/`Result` API. [source: std::convert::AsRef]
- Do not use `as_ref` only to emulate `Deref` — prefer deref coercion where applicable. [source: std::convert::AsRef]

### [TAG: 03-idioms] `Default`
- Trait for a useful default value: `fn default() -> Self`. [source: std::default::Default]
- `#[derive(Default)]` when all fields implement `Default`; struct update syntax `..Default::default()`. [source: std::default::Default]
- For enums, `#[default]` on exactly one unit variant selects the default variant. [source: std::default::Default]

### [TAG: 02-language-rules] `PhantomPinned`
- Marker type that does **not** implement `Unpin`; if a struct contains `PhantomPinned`, it is not `Unpin` by default. [source: std::marker::PhantomPinned]
- Use to opt out of auto-`Unpin` when a type must rely on pinning invariants. [source: std::marker::PhantomPinned + std::marker::Unpin]

---

## Part VI continued — Error index excerpt (closure lifetime)

### E0373 — closure capture may not live long enough (excerpt)
> A captured variable in a closure may not live long enough.
> By default, Rust captures closed-over data by reference.
> The solution to this problem is usually to switch to using a `move` closure.
[source: https://doc.rust-lang.org/error_codes/E0373.html]

---

## Part VII — Closing gaps (`std::io`, `MaybeUninit`, `Future`/`task`, `Borrow`/`Hash`/`HashMap`, `ControlFlow`, threads, `alloc`/`no_std`)

### [TAG: 07-async-concurrency] `std::io::Read`
- Readers are types that implement `Read`; core method `read(&mut self, buf: &mut [u8]) -> Result<usize>`. [source: std::io::Read]
- Provided helpers (`read_to_end`, `read_exact`, `bytes`, `chain`, `take`, …) build on `read`. [source: std::io::Read]
- Repeated reads advance the same cursor; successive `read_to_end` on a `File` only returns content once — rewind if needed. [source: std::io::Read]
- Using `BufRead`/`BufReader` reduces syscalls when many small reads would otherwise call `read` repeatedly. [source: std::io::Read]

### [TAG: 07-async-concurrency] `std::io::Write`
- Writers implement `write` + `flush`; `write` may write a prefix only; `write_all` loops until complete (respect `ErrorKind::Interrupted` retry). [source: std::io::Write]
- Writers are composable (`BufWriter`, etc.). [source: std::io::Write]

### [TAG: 07-async-concurrency] `std::io::BufRead`
- `BufRead: Read` adds an internal buffer; enables line-oriented APIs (`read_line`, `lines`) efficiently. [source: std::io::BufRead]
- `Read` alone (e.g. `File`) is not `BufRead` — wrap with `BufReader::new`. [source: std::io::BufRead]
- Lower level: `fill_buf` + `consume` coordinate buffer visibility. [source: std::io::BufRead]

### [TAG: 07-async-concurrency] `std::io::Seek`
- `Seek` provides a byte cursor; `seek(SeekFrom::…)` returns new absolute offset from stream start. [source: std::io::Seek]
- `rewind()` ≡ `seek(SeekFrom::Start(0))`. [source: std::io::Seek]
- Seeking beyond end is allowed; behavior is implementation-defined. [source: std::io::Seek]

### [TAG: 08-unsafe-and-ffi] `MaybeUninit<T>`
- Wrapper for **uninitialized** or partially initialized `T`; compiler must not assume valid `T` bit-pattern until initialized. [source: std::mem::MaybeUninit]
- Zeroing/`uninitialized` a `T` with invalid bit patterns (e.g. references, `bool`) is UB — `MaybeUninit` is the safe abstraction boundary. [source: std::mem::MaybeUninit]
- After real initialization, `assume_init` / `assume_init_read` / `assume_init_drop` transfer responsibility per method docs. [source: std::mem::MaybeUninit]
- Supports out-parameters and element-by-element array init patterns (see rustdoc examples). [source: std::mem::MaybeUninit]

### [TAG: 07-async-concurrency] `Future`, `Poll`, `Context`, `Waker`
- `Future::Output`; `poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>`. [source: std::future::Future]
- `Poll::Pending` vs `Poll::Ready`; after `Ready`, do not `poll` again (trait places no further requirements — may panic / UB-adjacent misuse). [source: std::future::Future]
- On `Pending`, store wakeup from the latest `Context::waker()`; only the most recent `Waker` should be scheduled. [source: std::future::Future]
- `poll` must not block; offload long work. [source: std::future::Future]
- Typical usage: `.await` in async Rust rather than manual `poll` loops. [source: std::future::Future]

### [TAG: 03-idioms] `Borrow` / `ToOwned` / `Hash` / `HashMap`
- `Borrow<Borrowed>`: `borrow(&self) -> &Borrowed`; for `HashMap` keys, `Eq`+`Hash` must agree for `K` and borrowed lookup `Q`. [source: std::borrow::Borrow]
- If `Hash`/`Eq` for a wrapper differ from the borrowed type (e.g. case-folding), do **not** implement `Borrow<str>` — use `AsRef` instead. [source: std::borrow::Borrow]
- `ToOwned`: `Owned: Borrow<Self>`; generalizes `Clone`-from-borrow (`clone_into`). [source: std::borrow::ToOwned]
- `Hash` + `Eq`: require `k1 == k2 ⇒ hash(k1) == hash(k2)`; `HashMap`/`HashSet` rely on this (logic error if violated, not necessarily UB). [source: std::hash::Hash]
- Hash output is not portable across platforms/compiler versions — tests should check consistency with `Eq`, not hard-coded hashes. [source: std::hash::Hash]
- `HashMap<K, V, S = RandomState, A>`: default hasher resists HashDoS (SipHash 1-3 today — subject to change); replace via `with_hasher` / `with_capacity_and_hasher`. [source: std::collections::HashMap]
- Do not mutate keys in-place in ways that change `Hash`/`Eq` while inserted (Cell/RefCell/global/IO/unsafe paths). [source: std::collections::HashMap]

### [TAG: 06-error-handling] `ControlFlow` and `?`
- `ControlFlow<B, C>`: `Continue(C)` vs `Break(B)` — explicit early-exit for traversals/visitors. [source: std::ops::ControlFlow]
- Works with `?` to propagate `Break` / continue on `Continue` (see `Iterator::try_for_each` examples). [source: std::ops::ControlFlow]
- For recoverable errors in application code, `Result`/`?` remains primary (Book ch.9). [source: Book ch9 + std::ops::ControlFlow]

### [TAG: 07-async-concurrency] `thread::scope` / scoped threads
- `thread::scope` creates a scope; spawned threads can borrow non-`'static` stack data — joined before `scope` returns. [source: std::thread::scope]
- If auto-joined threads panicked, `scope` panics; join manually to handle. [source: std::thread::scope]
- Lifetimes `'scope` vs `'env` documented on `Scope` (see rustdoc). [source: std::thread::scope]

### [TAG: 07-async-concurrency] `Condvar`
- Block waiting for an event with **no busy-wait**; always paired with a `Mutex` and a predicate checked under the lock. [source: std::sync::Condvar]
- Spurious wakeups possible — use `while !condition { wait(...) }` patterns. [source: std::sync::Condvar]
- Do not use multiple mutexes with one condvar (may panic). [source: std::sync::Condvar]

### [TAG: 07-async-concurrency] `std::sync::mpsc`
- Multi-producer, single-consumer FIFO; `Sender`/`SyncSender` + `Receiver`. [source: std::sync::mpsc]
- `channel()` unbounded async sends; `sync_channel(n)` bounded (0 = rendezvous). [source: std::sync::mpsc]
- Disconnect when peer dropped → `Err` on send/recv. [source: std::sync::mpsc]

### [TAG: 12-modern-rust] `alloc` crate and `no_std`
- `alloc` is the Rust **core allocation and collections** library (heap types + collections); re-exported through `std` for normal crates. [source: alloc crate root]
- Crates with `#![no_std]` typically depend on `core` + `alloc` (and `#[global_allocator]` where needed) instead of `std`. [source: alloc crate root]
- `Box`, `Rc`/`Arc`, `Vec`, `String`, collections — live in `alloc` (see module list). [source: alloc crate root]

### [TAG: 02-language-rules] Reference — namespaces (name resolution)
- Separate namespaces: types, values, macros, lifetimes, labels — same identifier can coexist across namespaces. [source: Reference names/namespaces]
- Macro namespace split: bang-macros vs attribute macros (sub-namespaces). [source: Reference names/namespaces]
- Struct fields are not in a global namespace — only reachable via field expressions. [source: Reference names/namespaces]

---

## Part VIII — Error index: completeness strategy + more excerpts

- **Every** rustc error code has an HTML page under `https://doc.rust-lang.org/error_codes/` and `rustc --explain EXXX` prints the same text — use these for codes not duplicated below. [source: https://doc.rust-lang.org/error_codes/index.html]
- The index is large and evolving; this cluster duplicates **high-traffic** excerpts only; for exhaustive coverage, grep or script against the official index, not a hand-maintained list.

### E0252 — excerpt (see full page)
> Two items of the same name cannot be imported without rebinding one of the items under a new local name.
[source: https://doc.rust-lang.org/error_codes/E0252.html]

### E0425 — excerpt (see full page)
> An unresolved name was used.
[source: https://doc.rust-lang.org/error_codes/E0425.html]

### E0432 — excerpt (see full page)
> An import was unresolved.
[source: https://doc.rust-lang.org/error_codes/E0432.html]

### E0433 — excerpt (see full page)
> An undeclared crate, module, or type was used.
[source: https://doc.rust-lang.org/error_codes/E0433.html]

### E0560 — excerpt (see full page)
> An unknown field was specified into a structure.
[source: https://doc.rust-lang.org/error_codes/E0560.html]

### E0603 — excerpt (see full page)
> A private item was used outside its scope.
[source: https://doc.rust-lang.org/error_codes/E0603.html]

### E0384 — excerpt (see full page)
> An immutable variable was reassigned.
[source: https://doc.rust-lang.org/error_codes/E0384.html]

