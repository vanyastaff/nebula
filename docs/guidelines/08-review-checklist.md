# 8. Code Review Checklist

[← Modern Rust](./07-modern-rust.md) | [README](./README.md) | [Next: Appendices →](./09-appendices.md)

---

Emit code that passes this list. When reviewing, mark violations with the rule ID.

## Signatures

- Takes `&str`, `&[T]`, `&T` — not `&String`, `&Vec<T>`, `&Box<T>` `[I-001]`.
- Library returns typed error, not `Box<dyn Error>` `[A-005]`.
- Consuming fallible fn returns the consumed value on error `[I-016]`.
- No `async` without `.await` `[A-009]`.
- `'static` minimized; only on truly static data `[L-SPECIAL-005]`.

## Types

- Newtype for identifiers and units `[P-003]`.
- `#[non_exhaustive]` on growable public enums `[L-ITEM-002]`.
- `#[must_use]` on `Result`-like returns and guard types `[L-ITEM-003]`.
- Typestate for phase-dependent API `[F-001]`.
- `PhantomData` present on any struct with a generic not appearing in fields `[L-VAR-003]`.

## Ownership and lifetimes

- No `.clone()` "to compile"; a real reason exists `[A-001]`.
- No `Arc<Mutex<…>>` as architectural band-aid `[A-006]`.
- Drop order inspected for structs with destructors and inter-field references `[L-DROP-002]`.
- Guard variables named `_guard` / `_g`, never `_` `[I-005]`.
- Critical resources closed explicitly, not via `Drop` only `[L-DROP-007]`.

## Unsafe

- SAFETY comment on every `unsafe { … }` `[L-UNSAFE-001]`.
- Public `unsafe fn` documents preconditions in `# Safety` `[L-UNSAFE-003]`.
- `unsafe_op_in_unsafe_fn = deny` `[L-UNSAFE-002]`.
- `&raw const/mut`, not `&field as *const _` `[L-UNSAFE-007]`.
- Unsafe encapsulated in small module with safe API `[P-011]`.
- Miri passes.

## Ergonomics

- No `Deref` for non-smart-pointer types `[A-003]`.
- No `#![deny(warnings)]` in `lib.rs` `[A-002]`.
- No `#[allow(…)]` without justification `[A-014]`.
- Docstrings on all `pub` items; doctests real (`[I-014]` where boilerplate).

## Async

- Spawned tasks propagate `Result`; no `.unwrap()` `[A-007]`.
- No `.await` in `Drop`; explicit `close().await` `[L-DROP-008]`.
- `Send` bounds minimized and reasoned `[L-SPECIAL-002]`.
- Async closures used where `impl Fn(&T) -> impl Future<_>` would fail `[R-006]`.

## Traits

- Trait used as `dyn` has generic methods with `where Self: Sized` `[L-DYN-002]`.
- `dyn Error + Send + Sync` auto traits specified where needed `[L-DYN-005]`.
- Orphan rule satisfied; no attempted foreign impls `[L-COH-001]`.

## Layout / FFI

- `#[repr(C)]` on all FFI-exposed structs `[L-REPR-002]`.
- `#[repr(transparent)]` on newtypes used at FFI boundary `[L-REPR-003]`.
- Packed fields accessed via `&raw` / `read_unaligned` `[L-REPR-004]`.
- C enums modeled as `struct Foo(u32)` + consts, not Rust `enum` `[L-REPR-005]`.

## Lints and CI

- `RUSTFLAGS="-D warnings"` in CI.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo fmt --check` passes.
- `cargo deny check` passes.
- `cargo miri test` (on any crate with `unsafe`).

---

[← Modern Rust](./07-modern-rust.md) | [README](./README.md) | [Next: Appendices →](./09-appendices.md)