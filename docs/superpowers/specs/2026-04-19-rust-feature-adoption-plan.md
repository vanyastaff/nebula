---
title: Rust 1.75-1.95 feature adoption plan
date: 2026-04-19
status: draft
related:
  - docs/adr/0014-dynosaur-macro.md
  - docs/adr/0019-msrv-1.95.md
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - docs/STYLE.md
  - docs/MATURITY.md
---

# Rust 1.75-1.95 feature adoption plan

## Executive summary

- **Scope at HEAD.** 88 `#[async_trait]` attributes across 49 `.rs` files in 7
  crates; 0 `dynosaur` usages. ADR-0014 compliance is 0 %. (Audit quoted
  81/54 — the number has drifted +7 attrs, −5 files since 2026-04-19 morning
  because markdown design docs were included in the original count.)
- **Hardest slice.** `TriggerHandler` (18 dyn sites across `action` +
  `api` + `sdk`) and `CredentialAccessor` (16 dyn sites, cross-crate). Each
  is a single-ADR, multi-crate PR — cannot be sharded per-crate without
  breaking the workspace for a cycle.
- **Free lunch.** `once_cell` is down to **one** surviving call site
  (`crates/expression/src/maybe.rs:7` — a `use once_cell::sync::OnceCell;`),
  `lazy_static!` is already zero, and `LazyLock` / `OnceLock` are used 84
  times across 31 files. Removing `once_cell` from `[workspace.dependencies]`
  is a one-line-touch chip.
- **Recommended sequencing.** Phase 1 free-lunch (`once_cell` removal +
  `#[expect]` conversions, single PR per crate, parallelisable). Phase 2
  inherent AFIT for the 18 traits with zero `dyn` sites (one PR per crate,
  no cross-cutting). Phase 3 `dynosaur` migration for high-fanout traits
  (five multi-crate PRs, ADR-gated — see Hazards). Phases 4–5 are polish.
- **Biggest gotcha.** Two legacy storage traits (`WorkflowRepo`,
  `ExecutionRepo`) have duplicated definitions under
  `crates/storage/src/*_repo.rs` (actively used as `Arc<dyn …>` from the
  engine) **and** `crates/storage/src/repos/*.rs` (newer, not yet wired).
  The migration must not merge the two — the `repos/*.rs` layer already
  avoids `dyn`, so only the legacy pair needs dynosaur.

## Per-release adoption matrix (1.75 → 1.95)

Stabilizations confirmed against <https://releases.rs/> per-version
release notes. One row per item with workspace relevance (language
syntax, semantic change, or library API a workflow-engine author
would actually adopt). Transparent compiler relaxations,
arch-specific intrinsics (AVX, NEON, LoongArch), and internal
lint/const-fn expansions are omitted.

Columns:

- **Status** — workspace usage at HEAD (commit `62754680`, 2026-04-19).
  Numbers reproducible via *Methodology notes* commands.
- **Action** — where this plan routes the feature: one of the five
  migration phases (*Migration sequencing*), `polish` (safe to defer
  indefinitely — opportunistic pickup), `done` (already adopted or
  enforced), or `n/a` (doesn't apply to Nebula today).

### Rust 1.75.0 (2023-12-28)

| Feature | Status | Action |
|---|---|---|
| **`async fn` in trait (AFIT) + return-position `impl Trait` in trait (RPITIT)** — combined stabilization | 88 `#[async_trait]` attrs still in tree; 0 native AFIT trait defs; 0 dynosaur usages | **Phase 2 + Phase 3** — the load-bearing migration |
| Match on `usize` / `isize` with half-open ranges | 0 nested-range match sites | n/a |
| `Option::as_slice` / `Option::as_mut_slice` | 80 `as_slice` hits (most are `Value::as_slice` / `Vec::as_slice`) | polish — safe to use when `Some(x).as_slice()` surfaces |
| Pointer `byte_add` / `byte_sub` / `wrapping_byte_*` | 0 raw-ptr math | n/a |
| `Atomic*::from_ptr` | 0 | n/a |
| `FileTimes` + `File::set_modified` / `set_times` | 0 | polish |
| `IpAddr::to_canonical` | 0 | polish |

### Rust 1.76.0 (2024-02-08)

| Feature | Status | Action |
|---|---|---|
| `Arc::unwrap_or_clone` / `Rc::unwrap_or_clone` | 1 site | polish — flip each `Arc::try_unwrap(...).unwrap_or_else(|a| (*a).clone())` when spotted |
| `Result::inspect` / `Result::inspect_err` | 0 | polish — common in error-logging fallthrough paths |
| `Option::inspect` | 0 | polish |
| `ptr::from_ref` / `ptr::from_mut` | 0 | polish — replaces `&x as *const _` where it appears |
| `ptr::addr_eq` | 0 | polish |
| `type_name_of_val` | 0 | polish — useful in diagnostics / tracing spans |
| `std::hash::DefaultHasher` / `RandomState` re-export | transparent | n/a |
| Lint `ambiguous_wide_pointer_comparisons` | transparent | n/a |

### Rust 1.77.0 (2024-03-21)

| Feature | Status | Action |
|---|---|---|
| **C-string literals** `c"..."` | 16 valid sites | polish — flip any `CString::new("...").unwrap()` survivors |
| `mem::offset_of!` | 0 | n/a (no manual layout code) |
| **Async recursive calls with indirection** — removes `Box::pin` boxes for the direct recursion case | 54 `Box::pin(async move)` sites; not all are recursive | polish — identify the recursive subset during Phase 5 |
| `array::each_ref` / `array::each_mut` | 0 | polish |
| Slice methods: `first_chunk` / `last_chunk` / `split_first_chunk` / `split_last_chunk` | 0 | polish |
| `slice::chunk_by` / `chunk_by_mut` | 0 | polish |
| `core::net` stable (without `alloc`) | transparent | n/a |
| `File::create_new` | 0 | polish — replaces `OpenOptions::new().write(true).create_new(true).open(...)` idiom |
| `Mutex::clear_poison` / `RwLock::clear_poison` | 0 | polish |
| `Bound::map` | 0 | polish |
| Lint `static_mut_refs` (warn by default) | transparent; workspace has no `static mut` | n/a |

### Rust 1.78.0 (2024-05-02)

| Feature | Status | Action |
|---|---|---|
| **`#[diagnostic::on_unimplemented]`** + `#[diagnostic]` namespace | 0 adoption | **polish (high value)** — add to `Action` / `Credential` / `Resource` / `TriggerHandler` sealed traits so authors get targeted "you forgot to impl X" diagnostics at the integration seam |
| Upcasting `dyn Trait` → `dyn Trait + Auto` | not measured separately | polish |
| Async-fn-in-trait implementable with concrete signatures | useful during Phase 2/3 | see Phase 2/3 |
| `#[cfg(target_abi = ...)]` | 0 | n/a |
| `impl Read for &Stdin` | 0 | n/a |
| `impl From<TryReserveError> for io::Error` | transparent | n/a |

### Rust 1.79.0 (2024-06-13)

| Feature | Status | Action |
|---|---|---|
| Inline `const { ... }` expressions | 2 sites | polish — opportunistic in `static` array initialisers |
| **Associated type bounds** `T: Iterator<Item: Debug>` (RFC 2289) | 0 narrow matches (candidates in `nebula-schema`, `nebula-validator` generic bounds) | polish |
| Temporary lifetime extension in `if` / `match` expressions | transparent compile-time | n/a (pleasant; no refactor) |
| Unified `num::NonZero<T>` generic | 15 `NonZero` occurrences — already generic form | done (workspace uses the unified form) |
| `path::absolute` | 0 | polish |
| Slice pointer helpers (`NonNull::offset`, etc.) | 0 | n/a |
| `CStr::count_bytes` | 0 | polish |
| `io::Error::downcast` | 0 | polish |
| Lints `redundant_lifetimes` / `unnameable_types` | transparent | n/a |
| Importing `main` from other modules / crates | transparent | n/a |

### Rust 1.80.0 (2024-07-25)

| Feature | Status | Action |
|---|---|---|
| **`LazyCell` / `LazyLock`** | 84 hits across 31 files — adopted as workspace default | **Phase 1** — flip final `once_cell::sync::OnceCell` at `crates/expression/src/maybe.rs:7`, drop workspace dep |
| Exclusive range patterns in `match` (`0..5 =>`) | 0 sites | n/a |
| `Option::take_if` | 0 | polish |
| ASCII trim on `str` / `[u8]` — `trim_ascii` / `trim_ascii_start` / `trim_ascii_end` | 0 | polish |
| `str::split_at_checked` / `split_at_mut_checked` | 0 | polish |
| `Duration::div_duration_f32` / `_f64` | 0 | n/a |
| `BinaryHeap::as_slice` | 0 | polish |
| `Seek::seek_relative` | 0 | polish |
| `Vec::into_flattened`, `as_flattened`, `as_flattened_mut` | 0 | polish |
| IPv4/IPv6 `to_bits` / `from_bits` / `BITS` | 0 | polish |
| `impl IntoIterator for Box<[T]>` | transparent | n/a |

### Rust 1.81.0 (2024-09-05)

| Feature | Status | Action |
|---|---|---|
| **`#[expect(lint)]` attribute** | 21 adopted vs 116 `#[allow]` | **Phase 1** — per-crate chip, target ~80 conversions |
| **`core::error`** module stable (was `std::error` only) | 0 uses of `core::error::Error`; 60 `std::error::Error` refs | polish — switch if a `no_std` story emerges |
| `fs::exists` | 0 | polish — replaces `Path::new(p).try_exists()` calls |
| `hint::assert_unchecked` | 0 (unsafe hint; out of style for workspace) | n/a |
| `AtomicBool::fetch_not` | 0 | polish |
| `Duration::abs_diff` | 0 | polish |
| `IoSlice::advance` / `IoSliceMut::advance` + `_slices` | 0 | n/a |
| `PanicHookInfo` (replaces `std::panic::PanicInfo`) | 0 custom panic hook | polish |
| Stable driftsort / unstable ipnsort (transparent perf) | transparent | n/a |
| Abort on uncaught panics in `extern "C"` | transparent | n/a |

### Rust 1.82.0 (2024-10-17)

| Feature | Status | Action |
|---|---|---|
| **Precise capturing `+ use<'lt>`** on `impl Trait` (RFC 3617) | 0 adoption; ~22 `tokio::spawn(trait.method())` sites at risk after Phase 2 | **Phase 4** — applied inline during AFIT migration |
| **`&raw const` / `&raw mut`** pointer operators (RFC 2582) | 0 | n/a (no raw ptr forming today) |
| **`unsafe extern "..." { ... }`** blocks (RFC 3484) | 0 | n/a |
| **Unsafe attributes** — `unsafe(no_mangle)` / `unsafe(link)` / `unsafe(export_name)` | 0 uses of either form (FFI-minimal workspace) | n/a |
| **`Option::is_none_or`** | 8 sites | polish — expand opportunistically |
| `[T]::is_sorted` / `Iterator::is_sorted` / `_by` / `_by_key` | 0 | polish |
| `CharIndices::offset` | 0 | n/a |
| `iter::repeat_n` | 0 | polish |
| `future::Ready::into_inner` | 0 | n/a |
| `Thread::Builder::spawn_unchecked` | 0 (unsafe) | n/a |
| Nested-field access in `offset_of!` | 0 | n/a |
| `const` operands in inline assembly | 0 inline assembly | n/a |
| Floating-point arithmetic in `const fn` | transparent | n/a |
| Empty-type match patterns can be omitted | transparent | n/a |

### Rust 1.83.0 (2024-11-28)

| Feature | Status | Action |
|---|---|---|
| **`std::sync::Mutex::new` / `RwLock::new` / `Condvar::new` const fn** | 0 `static` uses of `parking_lot::Mutex` / `RwLock`; no const-init pattern to reclaim | **Do not migrate** — `parking_lot` is kept for non-poisoning + fast-path, not for const-init |
| `&mut` / `*mut` / `&Cell` / `*const Cell` in `const` | 0 direct use; transparent compile-time | n/a |
| References to statics in `const` initializers | transparent | n/a |
| **Raw lifetimes + labels `'r#ident`** | 0 | n/a |
| Non-exhaustive empty structs | 0 | polish |
| `const extern` functions with non-C ABI | 0 | n/a |
| `Option::get_or_insert_default` | 0 | polish |
| `ControlFlow::{break_value, continue_value, map_break, map_continue}` | 0 | polish |
| 18 new `ErrorKind` variants (io) | 0 of the new ones matched on | polish |
| `char::MIN` constant | 0 | n/a |
| Atomic race semantics documented | transparent | n/a |

### Rust 1.84.0 (2025-01-09)

| Feature | Status | Action |
|---|---|---|
| **Cargo MSRV-aware resolver** + resolver v3 | Adopted — `resolver = "3"` in `Cargo.toml:40`; `rust-version = "1.95"` drives per-dep MSRV picks | **done** |
| Raw pointer references safe without dereferencing (no `&*raw_ptr` dance) | 0 raw-ptr sites | n/a |
| Allow coercions to drop the principal of trait objects | transparent | n/a |
| Windows forward-slash in `include!()` | transparent | n/a |
| `From<&mut [T]>` for `Box<[T]>` / `Rc<[T]>` / `Arc<[T]>` | 0 | polish |
| Float `copysign`/`abs`/`signum` moved to `core` | transparent | n/a |
| `FromStr for CString`, `TryFrom<CString> for String` | 0 | n/a |
| Next-generation trait solver in coherence checking | transparent (compiler internal) | n/a |

### Rust 1.85.0 (2025-02-20)

| Feature | Status | Action |
|---|---|---|
| **Edition 2024 stable** | Adopted — `edition = "2024"` in workspace package (ADR-0010) | **done** |
| **Async closures** `async \|x\| { ... }` (RFC 3668) | 0 adoption, 54 `Box::pin(async move { ... })` sites — subset is closure-shaped | **Phase 5** — convert the ~5–10 stored `FnMut`-shaped cases |
| `#[diagnostic::do_not_recommend]` | 0 | polish — pair with `on_unimplemented` at sealed-trait sites |
| `AsyncFn*` in prelude across all editions | transparent | n/a |
| `Waker::noop` | 0 | polish (test doubles) |
| `{float}::midpoint` / `{integer}::midpoint` / `NonZero::midpoint` | 0 | polish |
| `io::ErrorKind::QuotaExceeded` / `::CrossesDevices` | 0 | polish |
| `ptr::fn_addr_eq` | 0 | polish |
| `BuildHasherDefault::new` | 0 | polish |
| Combined `#[no_mangle]` + `#[export_name]` lint | transparent (no uses) | n/a |

### Rust 1.86.0 (2025-04-03)

| Feature | Status | Action |
|---|---|---|
| **Trait upcasting coercion** (`&dyn Sub → &dyn Super` without `as`) | 3 `as Arc<dyn ...>` casts: `crates/action/tests/dx_poll.rs:639,749` (`Failing*Emitter` → `Arc<dyn ExecutionEmitter>`), `crates/plugin/src/versions.rs:52` (`Plugin` impl → `Arc<dyn Plugin>`). Most are concrete→trait-object (not upcasts) — upcasting helps only true supertrait casts. | polish — per-site inspection |
| `#[target_feature]` on safe functions | 0 | n/a |
| `{float}::next_down` / `next_up` | 0 | polish |
| `<[_]>::get_disjoint_mut` / `unchecked_mut` + `HashMap::get_disjoint_mut` | 0 | polish — replaces split-borrow dance |
| `NonZero::count_ones` | 0 | polish |
| `Vec::pop_if` | 0 | polish |
| **`sync::OnceLock::wait`** + `Once::wait` / `wait_force` | 0 use of `wait` (all `OnceLock` via `get_or_init`) | polish — for latch-shaped patterns |
| `missing_abi` warn-by-default, `double_negations` lint | transparent | n/a |

### Rust 1.87.0 (2025-05-15)

| Feature | Status | Action |
|---|---|---|
| **`use<...>` in trait RPITIT** — `precise_capturing_in_traits` | 0 use<> adoption (will happen in Phase 4 context) | **Phase 4** — applies when RPITIT-in-trait returns leak unneeded captures |
| `asm_goto` | 0 inline asm | n/a |
| `Self: Sized` methods no longer required in unsized-type impls | transparent | n/a |
| `Vec::extract_if` / `vec::ExtractIf` | 0 | polish |
| `LinkedList::extract_if` | 0 | n/a |
| Slice `split_off` / `split_off_first` / `split_off_last` (+ mut) | 0 | polish |
| `String::extend_from_within` | 0 | polish |
| `OsString::display` / `OsStr::display` | 0 | polish |
| Anonymous pipe API: `io::pipe` / `PipeReader` / `PipeWriter` | 0 | polish (IPC tests) |
| `Box<MaybeUninit<T>>::write` | 0 `MaybeUninit` | n/a |
| `TryFrom<Vec<u8>> for String` | 0 | polish |
| Signed/unsigned pointer offset: `offset_from_unsigned`, `byte_offset_from_unsigned` | 0 | n/a |
| Integer `cast_signed` / `cast_unsigned` / `is_multiple_of` | 0 | polish |
| Parsing `!-5..` / `-foo..` open-beginning ranges | transparent | n/a |

### Rust 1.88.0 (2025-06-26)

| Feature | Status | Action |
|---|---|---|
| **let-chains** — `if let Some(x) = a && x.valid() { ... }` | 0 adoption; 16 `if let Some(...) = _ { if ... }` nested + 23 wider nested | **Phase 5** — per-crate when ≥3 nesting levels share one body |
| **Naked functions** `#[unsafe(naked)]` | 0 | n/a (no low-level asm) |
| `cfg_boolean_literals` — `#[cfg(true)]` / `#[cfg(false)]` | 0 | polish — handy for gated test modules |
| **`Cell::update`** | 0 | polish — low volume |
| `impl Default for *const T` / `*mut T` | 0 | n/a |
| **`HashMap::extract_if` / `HashSet::extract_if`** | 0 | polish |
| `hint::select_unpredictable` | 0 | n/a |
| `proc_macro::Span` accessors (`file`, `local_file`, `line`, `column`, …) | 1 macro crate family touched — check `validator/macros`, `schema/macros` for diagnostic improvements | polish |
| **`[T]::as_chunks` / `as_rchunks` / `as_chunks_unchecked`** | 0 | polish |
| `mod ffi::c_str` stabilization | transparent | n/a |

### Rust 1.89.0 (2025-08-07)

| Feature | Status | Action |
|---|---|---|
| Explicitly inferred const arguments (`feature(generic_arg_infer)`) | 0 const-generic sites using `_` inference | polish |
| `#[repr(u128)]` / `#[repr(i128)]` | 0 | n/a |
| Temporary lifetime extension through tuple-struct / variant constructors | transparent | n/a |
| `NonZero<char>` | 0 | n/a |
| `File::lock` / `lock_shared` / `try_lock` / `try_lock_shared` / `unlock` | 0 | polish — workspace uses separate lock crates today; swap when a path-lock need arises |
| `NonNull::from_ref` / `from_mut` / `without_provenance` | 0 | n/a |
| `OsString::leak` / `PathBuf::leak` | 0 | polish |
| `Result::flatten` | 0 | polish — flattens `Result<Result<T, E>, E>` shapes |

### Rust 1.90.0 (2025-09-18)

| Feature | Status | Action |
|---|---|---|
| `u{n}::checked_sub_signed` family (+`overflowing`, `saturating`, `wrapping`) | 0 | polish |
| `IntErrorKind: Copy + Hash` | transparent | n/a |
| `CStr` / `CString` / `Cow<CStr>` comparison impls | 0 | polish |
| `proc_macro::Ident::new` supports `$crate` | relevant for macro crates | polish |
| `Thread::into_raw` alignment guarantee | 0 | n/a |
| Split of `unknown_or_malformed_diagnostic_attributes` lint | transparent | n/a |
| Volatile access to non-Rust memory | 0 | n/a |

### Rust 1.91.0 (2025-10-30)

| Feature | Status | Action |
|---|---|---|
| Pattern bindings lowered in written order + drop order on primary bindings | transparent | n/a |
| `Path::file_prefix` | 0 | polish |
| **`AtomicPtr::fetch_*`** — `fetch_ptr_add`, `fetch_byte_add`, `fetch_or`, `fetch_and`, `fetch_xor` | 0 AtomicPtr use | polish — useful if a lock-free ptr slot appears in `metrics` / `telemetry` |
| Integer `strict_*` arithmetic (add/sub/mul/div/pow + signed/unsigned variants) | 0 | polish — swap in wherever an overflow panic is the intended behaviour |
| `PanicHookInfo::payload_as_str` | 0 panic hook | polish |
| `core::iter::chain` function form | 0 | polish |
| `core::array::repeat` | 0 | polish |
| `PathBuf::add_extension` / `with_added_extension` | 0 | polish |
| **`Duration::from_mins` / `Duration::from_hours`** | 0 | polish — replaces `Duration::from_secs(60 * n)` |
| `PartialEq` Path ↔ str / String | 0 | polish — simplifies test assertions |
| IPv4/IPv6 `from_octets` / `from_segments` | 0 | polish |
| `Default` for `Pin<Box<T>>` / `Pin<Rc<T>>` / `Pin<Arc<T>>` | 0 | polish |
| **`BTreeMap::extract_if` / `BTreeSet::extract_if`** | 0 | polish |
| Carrying / borrowing arithmetic (`carrying_add`, `borrowing_sub`, `carrying_mul`) | 0 | n/a |
| `Cell::as_array_of_cells` | 0 | n/a |
| `str::ceil_char_boundary` / `floor_char_boundary` | 0 | polish |
| Lint `integer_to_ptr_transmutes` (warn) | transparent (no transmutes) | n/a |
| Lint `dangling_pointers_from_locals` | transparent | n/a |

### Rust 1.92.0 (2025-12-11)

| Feature | Status | Action |
|---|---|---|
| `&raw [mut \| const]` for union fields in safe code | 0 | n/a |
| Auto-trait / `Sized` bound preference for associated types | transparent | n/a |
| `never_type_fallback_flowing_into_unsafe` deny-by-default | transparent (no affected code) | n/a |
| **`RwLockWriteGuard::downgrade`** | 0 | polish — replaces explicit drop-then-read-lock patterns in `resource` / `engine` if they exist |
| `NonZero<u{N}>::div_ceil` | 0 | polish |
| `Location::file_as_c_str` | 0 | n/a |
| `Box::new_zeroed` / `new_zeroed_slice` + `Rc` / `Arc` variants | 0 | n/a |
| `btree_map::Entry::insert_entry` / `VacantEntry::insert_entry` | 0 | polish |
| `iter::Repeat::last` / `count` panic instead of infinite-looping | transparent (no code hits it) | n/a |
| `#[track_caller]` + `#[no_mangle]` combo | 0 | n/a |

### Rust 1.93.0 (2026-01-22)

| Feature | Status | Action |
|---|---|---|
| `asm_cfg` | 0 inline asm | n/a |
| `const` items with mutable references to `static` | 0 | n/a |
| Lint `const_item_interior_mutations` (warn) | transparent | n/a |
| Lint `function_casts_as_integer` (warn) | transparent | n/a |
| **`<[MaybeUninit<T>]>::assume_init_*`** + `write_copy_of_slice` / `write_clone_of_slice` | 0 `MaybeUninit` workspace-wide | n/a |
| **`String::into_raw_parts` / `Vec::into_raw_parts`** | 0 | n/a (would require `unsafe` reassembly — avoid) |
| **`<[T]>::as_array` / `as_mut_array`** + raw-slice variants | 0 adoption (the `as_array` hits in workspace are `serde_json::Value::as_array`) | polish — safe typed conversion from `&[T]` to `&[T; N]` |
| `VecDeque::pop_front_if` / `pop_back_if` | 0 VecDeque use | n/a |
| **`std::fmt::from_fn`** + `FromFn` type | 0 | polish — replaces one-shot `struct FooDisplay(...); impl Display for FooDisplay { ... }` adapter |
| `Duration::from_nanos_u128` | 0 | polish |
| `char::MAX_LEN_UTF8` / `MAX_LEN_UTF16` | 0 | polish |

### Rust 1.94.0 (2026-03-05)

| Feature | Status | Action |
|---|---|---|
| Prior MSRV (per ADR-0010); superseded by 1.95 (ADR-0019) | | **done** |
| Impls and impl items inherit `dead_code` lint level of the trait | transparent — may un-silence some `#[allow(dead_code)]` flips targeted by Phase 1 | see Phase 1 |
| `<[T]>::array_windows` | 0 | polish |
| `<[T]>::element_offset` | 0 | polish (pointer-math safe alternative) |
| **`LazyCell::get` / `get_mut` / `force_mut`** + **`LazyLock::get` / `get_mut` / `force_mut`** | 0 (we use `LazyLock::force()` / deref) | polish — useful in tests that need to mutate a `LazyLock`-wrapped fixture |
| `TryFrom<char> for usize` | 0 | polish |
| `Peekable::next_if_map` / `next_if_map_mut` | 0 | polish — elegant in `nebula-expression` tokenizer |
| `EULER_GAMMA`, `GOLDEN_RATIO` constants for `f32` / `f64` | 0 | n/a |
| Unicode upgraded to 17 | transparent | n/a |
| Lint warn-by-default for unused visibility on `const _` | transparent | n/a |

### Rust 1.95.0 (2026-04-16) — current MSRV

| Feature | Status | Action |
|---|---|---|
| **`if let` guards on match arms** | 0 adoption | **Phase 5** — opportunistic during `engine` / `workflow` match-block touches (ADR-0019 §Context explicitly flags this) |
| **`cfg_select!`** macro | 0 adoption; 0 `cfg_if!` invocations in workspace → nothing to migrate | n/a |
| **Atomic `update` / `try_update`** on `AtomicBool` / `AtomicPtr` / `AtomicIsize` / `AtomicUsize` | 0 adoption; 5 `fetch_update` / `compare_exchange` CAS loops in tree | **Phase 5** — replace where shape matches (ADR-0019 §Context explicitly flags this) |
| **`core::range`** + `RangeInclusive` / `RangeInclusiveIter` | 0 | polish |
| `core::hint::cold_path` | 0 | polish — useful in error-path branches of hot loops |
| Path-segment keyword importing with renaming | 0 | n/a |
| `bool: TryFrom<{integer}>` | 0 | polish |
| `MaybeUninit` array conversions + Cell array refs | 0 | n/a |
| Unsafe pointer `as_ref_unchecked` / `as_mut_unchecked` | 0 | n/a |
| `fmt::from_fn` / `ControlFlow::is_break` / `is_continue` const | 0 | polish |
| MSRV floor pinned at 1.95 | `Cargo.toml:45`, CI, `clippy.toml` (ADR-0019) | **done** |

### Release-to-phase mapping

| Phase | Releases it picks up |
|---|---|
| Phase 1 (free-lunch) | 1.80 `LazyLock` finalisation, 1.81 `#[expect]` |
| Phase 2 (inherent AFIT) | 1.75 AFIT |
| Phase 3 (dynosaur) | 1.75 AFIT + ADR-0014 |
| Phase 4 (precise capture) | 1.82 `use<>`, 1.87 `use<>` in trait RPITIT |
| Phase 5 (polish) | 1.77 c-strings + recursive async, 1.82 `Option::is_none_or`, 1.85 async closures, 1.86 trait upcasting, 1.88 let-chains + `Cell::update` + `[T]::as_chunks`, 1.95 atomic `update` + `if let` guards |
| `done` today | 1.84 resolver 3, 1.85 edition 2024, 1.95 MSRV bump |
| `polish` pool (opportunistic pickup across all crates) | 1.76 `Arc::unwrap_or_clone`, 1.76 `Result::inspect` family, 1.77 `File::create_new` / `Mutex::clear_poison`, 1.78 `#[diagnostic::on_unimplemented]` on sealed traits, 1.80 `Option::take_if` / `trim_ascii` / `split_at_checked` / IPv4-IPv6 `to_bits`, 1.81 `fs::exists` / `Duration::abs_diff`, 1.86 `get_disjoint_mut` / `Vec::pop_if`, 1.87 `Vec::extract_if` / `OsString::display`, 1.88 `HashMap::extract_if`, 1.91 `Duration::from_mins` / `BTreeMap::extract_if`, 1.92 `RwLockWriteGuard::downgrade`, 1.93 `<[T]>::as_array` / `fmt::from_fn`, 1.94 `LazyLock::get_mut` / `Peekable::next_if_map`, 1.95 `core::range` / `cold_path` / `bool: TryFrom<_>` |
| `n/a` for Nebula | 1.75 byte-ptr math / FileTimes, 1.76 strict provenance, 1.77 `offset_of!`, 1.78 `target_abi`, 1.79 `unchecked_*` integer ops, 1.80 exclusive range patterns, 1.82 `&raw const/mut` / unsafe FFI attrs / inline asm, 1.83 raw lifetimes / const-extern non-C ABI, 1.84 raw-ptr ergonomics, 1.85 `fn_addr_eq`, 1.87 `asm_goto` / anonymous pipes, 1.88 naked functions, 1.89 `repr128` / `NonNull::from_ref` / `NonZero<char>`, 1.90 volatile non-Rust memory, 1.91 carrying arithmetic, 1.92 `Box::new_zeroed` family, 1.93 `MaybeUninit` slice helpers / `into_raw_parts`, 1.94 float constants, 1.95 `MaybeUninit` array conversions / `as_*_unchecked` |

## Inventory

### `#[async_trait]` usage (per crate)

Counts come from `rg --count-matches '#\[async_trait\]' --glob '*.rs'`
(filtering out markdown design docs included in the audit). `dyn` column
reflects cross-workspace `\bdyn\s+<Trait>\b` hits at HEAD.

| Crate | Attrs | Defs | Impls | Dyn traits (fanout) | Notes |
|---|---:|---:|---:|---|---|
| `nebula-action` | 33 | 8 | 25 | `TriggerHandler` (18), `StatelessHandler` (14), `ResourceHandler` (9), `ResourceAccessor` (7), `ExecutionEmitter` (7), `StatefulHandler` (6), `TriggerScheduler` (5), `AgentHandler` (2) | Highest concentration. 8 trait families; every `dyn`-consumed integration seam lives here. |
| `nebula-storage` | 27 | 18 | 9 | `ControlQueueRepo` (10), `ExecutionRepo` (6, legacy at `execution_repo.rs:121`), `WorkflowRepo` (4, legacy at `workflow_repo.rs:78`). **17 others with 0 dyn sites** (`WorkflowVersionRepo`, `AuditRepo`, `WorkspaceRepo`, `JournalRepo`, `CredentialRepo`, `ExecutionNodeRepo`, `BlobRepo`, `UserRepo`, `SessionRepo`, `PatRepo`, `OrgRepo`, `ResourceRepo`, `QuotaRepo`, `TriggerRepo`, plus `repos/workflow.rs`-side `WorkflowRepo` & `repos/execution.rs`-side `ExecutionRepo` duplicates). | Split personality: legacy `*_repo.rs` files (3 traits, wired to engine via `Arc<dyn>`) vs newer `repos/*.rs` layer (17 traits, not yet dyn-consumed). Migration must keep the split. |
| `nebula-credential` | 13 | 4 | 9 | `CredentialAccessor` (16, cross-crate). `NotificationSender`/`TestableCredential`/`RotatableCredential` have 0 dyn sites. | Rotation traits are pure generic bounds — inherent AFIT fine. |
| `nebula-engine` | 5 | 0 | 5 | n/a (consumer only) | Impls of traits defined in `storage`, `credential`, `action`. Migrations here are follow-on to upstream trait surgery. |
| `nebula-runtime` | 4 | 3 | 1 | `StatefulCheckpointSink` (6), `BlobStorage` (2). `TaskQueue` has 0 dyn sites. | Small but feeds the engine's hot path — needs careful `'static` analysis after AFIT flip. |
| `nebula-sandbox` | 4 | 1 | 3 | `SandboxRunner` (2) | Low fanout. Single-PR migration. |
| `nebula-api` | 2 | 0 | 2 | n/a (consumer) | Both impls are in `handlers/health.rs`. |
| **Total** | **88** | **34** | **54** | — | 49 `.rs` files; audit 2026-04-19 quoted 81/54 (markdown-inclusive). |

Discrepancy vs audit explained: the audit counted any `.rs` **or**
markdown occurrence (`crates/action/docs/*.md` has 25, `crates/resource/
plans/*.md` has 3). Restricting to compiled code gives 88/49.

#### Duplicate trait definitions (known hazard)

| Trait | Legacy (in-use) | Newer (staged) | Recommendation |
|---|---|---|---|
| `WorkflowRepo` | `crates/storage/src/workflow_repo.rs:78` — consumed as `Arc<dyn WorkflowRepo>` in `crates/engine/src/engine.rs:141,497` and `crates/engine/tests/control_dispatch.rs:148`. | `crates/storage/src/repos/workflow.rs:16` — 0 dyn sites. | Phase 3 migrates the legacy one via `dynosaur`; Phase 2 moves the `repos/*.rs` sibling to inherent AFIT. Do not merge the two under this plan — ADR-0008 refactor owns that decision. |
| `ExecutionRepo` | `crates/storage/src/execution_repo.rs:121` — consumed as `Arc<dyn ExecutionRepo>` in `engine.rs:139,486,2728,3512,7500,7594`. | `crates/storage/src/repos/execution.rs:14` — 0 dyn sites. | Same split; same recommendation. |

### Other migration targets

Counts from `rg --count-matches` at HEAD unless stated.

#### `once_cell` / `lazy_static!` → `LazyLock` / `OnceLock` (stable 1.80)

| Pattern | Count | Notes |
|---|---:|---|
| `once_cell::sync::Lazy` | 0 | — |
| `once_cell::sync::OnceCell` | 1 | Single site: `crates/expression/src/maybe.rs:7`. |
| `once_cell::race::*` | 0 | — |
| `lazy_static!` | 0 | Already fully migrated. |
| `LazyLock` / `OnceLock` / `std::sync::Once` | 84 (31 files) | Pattern is already the workspace default. |
| `once_cell` in a crate `Cargo.toml` | 1 | `crates/expression/Cargo.toml:30`. |

**Workspace-dep removal estimate.** Deleting `once_cell = "1.21"` from
`Cargo.toml:79` and `crates/expression/Cargo.toml:30` plus flipping the
single `OnceCell` at `crates/expression/src/maybe.rs:7` is one small PR.
`OnceLock::get_or_try_init` shipped stable in 1.70 and is a drop-in for
the `OnceCell::get_or_try_init` shape used here (verify on read — the
file is 5 lines of use; no exotic feature).

#### `parking_lot` const-init → `std::sync::Mutex::new` (stable 1.83)

| Pattern | Count | Notes |
|---|---:|---|
| `parking_lot` in `Cargo.toml` | 8 crates | Kept for non-poisoning + uncontended-fast-path. |
| `parking_lot` mentions in `.rs` | 41 | Most are `parking_lot::RwLock` / `Mutex` in hot structs. |
| `static … : parking_lot::(Mutex\|RwLock)<…>` | **0** | No static declarations exist. |
| `const_new` feature requested in any `Cargo.toml` | 0 | — |

**Recommendation: no change.** `parking_lot` earns its place on
uncontended fast paths (no poisoning, smaller stable size). There is
nothing in the workspace using it **purely** for `const_new`, so there is
no "std now has const fn, swap it" win. Leave this class alone.

#### Nested `if let` → let-chains (stable 1.88)

| Pattern | Count | Notes |
|---|---:|---|
| `if let Some(…) = … { if … }` (sampled multiline) | 16 | Direct let-chain candidates. |
| `if let … = … { if … }` (any pattern, sampled) | 23 | Wider pool; not all will read better as chains. |
| `let … else { … }` already in use | 147 | Workspace is already idiomatic about single-escape let-else. |

Heuristic for a PR: convert only where at least three levels of nesting
share one body (i.e. eliminate an `else { return … }` mirror too). Avoid
flattening two-level `if let` / `if` pairs where the inner arm has more
than two statements — the nested form is still easier to read there.
Safe to defer — no dep removal, no compile-time impact.

#### `#[allow(...)]` → `#[expect(...)]` (stable 1.81)

| Pattern | Count | Notes |
|---|---:|---|
| `#[allow(dead_code)]` | 42 | Largest bucket. Many have a "used only when feature X enabled" story — good `#[expect]` candidates. |
| `#[allow(unused*)]` | 4 | Usually local; should be `expect`. |
| `#[allow(deprecated)]` | 5 | Must verify each still fires — `expect` gives us a regression guard. |
| `#[allow(clippy::*)]` | 38 | Rule-specific; safe to flip. |
| All other `#[allow(...)]` | 27 | Scan individually — some are legitimate forward-compat. |
| `#[expect(...)]` already | 21 | Migration started; no blocker. |
| **Total `#[allow]` in tree** | **116** | Upper bound on the chip. |

**Plan:** do *not* flip a blanket `s/allow/expect/`. Flip on a
crate-by-crate pass, confirming the lint still fires (build in verbose
mode, then `grep` for the `unfulfilled_lint_expectations` warning that
exposes a stale expect). The `forward-compat for future lints`
`#[allow]`s — typically the ones with no explanatory comment — stay as
`allow`. A sane target is ~80–90 conversions out of 116.

#### `core::error::Error` (stable 1.81)

| Pattern | Count |
|---|---:|
| `std::error::Error` refs | 60 |
| `core::error::Error` refs | 0 |

Most of Nebula is firmly std-bound (tokio, reqwest, sqlx). Only
candidates for `core::error::Error` would be `nebula-error` itself and
possibly `nebula-expression` / `nebula-validator` — crates that *could*
become `no_std`-adjacent later. Not worth a chip until someone files a
`no_std`-use story. Classified **P3 polish**, not a migration blocker.

#### Async closures (stable 1.85)

| Pattern | Count | Notes |
|---|---:|---|
| `Box::pin(async move …)` | 54 | Pool of potential `async ||` workarounds — must inspect each for shape (closures that capture shared state and are called multiple times are the real target; one-shot `Box::pin(async move)` in a `spawn` is not). |
| Existing `async fn` / `async \|\|` closures | n/a | Nothing to convert to. |

Not every `Box::pin(async move)` is an async-closure candidate; many are
inside `spawn` calls where the pinning is incidental. Real targets are
stored `FnMut`-shaped futures (e.g. retry predicates, observer
callbacks). A useful chip does a one-hour pass, converts ~5–10 real
cases, and stops.

#### `precise capturing use<…>` (stable 1.82) — AFIT migration blocker

| Pattern | Count | Notes |
|---|---:|---|
| `tokio::spawn(…)` total | 80 | Overall spawn surface. |
| `tokio::spawn(async move { adapter.<method>(…).await })` | ~22 | These call a method on a trait object that is currently `#[async_trait]` — after naive AFIT the returned future captures `'self` and cannot cross thread boundaries. |
| `use<…>` uses at HEAD | 0 | No adoption yet. |

The 22 at-risk spawn sites are concentrated in:
`crates/action/tests/dx_poll.rs` (15 sites — `adapter.start(...)` on
`TriggerHandler`), `crates/action/tests/dx_webhook.rs` (3 sites —
`adapter.handle_event(...)` / `stop(...)`), `apps/cli/src/commands/
actions.rs:294` (`handler.start(...)`), `crates/sdk/src/runtime.rs:231`
(`handler.start(...)`), `crates/engine/src/control_consumer.rs:308`
(`self.run(...)`), and `crates/storage/src/pg/control_queue.rs:1034-1035`
(integration-test worker spawns on `ControlQueueRepo`).

After the Phase 3 dynosaur flip these keep working (the `Dyn*` sibling
returns `Pin<Box<dyn Future + Send>>` just like `async-trait` did). The
`use<>` precise-capture chip kicks in for Phase 2 — the 18 inherent-AFIT
traits' return futures. Without `use<>`, spawning an inherent AFIT call
on a non-`'static` borrow breaks compile. Budget 1–2 per-call adjustments
per crate.

#### Small wins (sweep)

| Pattern | Count | Notes |
|---|---:|---|
| `inline const { … }` blocks | 2 | Already a few; low-volume target. |
| `Option::as_slice` (rough) | 80 occurrences of `as_slice` workspace-wide — hard to attribute cleanly without per-call inspection | Most are already the slice variant; no blanket migration. |
| `Cell::update` | 0 | No use sites — hand-written `let v = c.get(); c.set(f(v));` audits would be the chip. Low-value. |
| `[T]::as_chunks` (1.88) | 0 hits for `.chunks(` | Unused. |
| Atomic `update` / `try_update` candidates (`fetch_update` / `compare_exchange`) | 5 | Small enough to inline in Phase 5 polish. |
| `cfg_if!` invocations | 0 | `cfg_select!` has nothing to replace. |
| Match-arm `if let` guards (stable 1.95) | 0 nested `=> if` hits in sample | Future use, not a migration. |

## Migration sequencing

Five phases, ordered by reviewability and blast radius. Each phase
produces an independent, revertible PR family.

### Phase 1 — Free-lunch sweep (1 `once_cell` PR + per-crate `#[expect]` chips)

Two slices. Sub-phase 1a is a single workspace PR; sub-phase 1b is
per-crate and can run in parallel chips because the changes are
crate-local.

**1a (1 PR) — drop `once_cell`.**

- Flip `crates/expression/src/maybe.rs:7` from `once_cell::sync::OnceCell`
  to `std::sync::OnceLock` (`get_or_try_init` is API-compatible).
- Delete `once_cell = "1.21"` from `Cargo.toml:79` and
  `crates/expression/Cargo.toml:30`.

**1b (per-crate chip, one PR per crate) — `#[allow]` → `#[expect]`.**

- Convert `#[allow(dead_code)]`, `#[allow(unused*)]`, `#[allow(deprecated)]`,
  `#[allow(clippy::…)]` to `#[expect(…)]` only when the attribute already
  has an explanatory comment (or the rationale is obvious from context).
- Skip bare `#[allow]` with no rationale — those are legitimate
  forward-compatibility declarations and should stay as `allow`.
- Budget: ~80–90 conversions out of 116 total `#[allow]`s.

**Risk.** Zero blast radius on both slices; 1a deletes one workspace
dep; 1b changes lint attributes only.

**Verify (same for 1a and 1b).**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

For 1b also run `cargo clippy --workspace` in verbose mode once and
confirm no `unfulfilled_lint_expectations` warnings fire — a stale
`#[expect]` means the underlying lint no longer triggers and the
attribute is wrong.

**Acceptance — 1a:** `cargo deny check` still green; `rg 'once_cell'
crates/` and `rg 'lazy_static!' crates/` both return nothing;
`[workspace.dependencies]` drops `once_cell` (one entry).

**Acceptance — 1b (per crate):** `rg '#\[allow\(' crates/<crate>/src/`
count drops by the number of conversions the PR made; `rg '#\[expect\('
crates/<crate>/src/` grows by the same amount; no
`unfulfilled_lint_expectations` warnings in the build.

### Phase 2 — Inherent AFIT for zero-dyn traits (3 PRs, one per owner crate)

**Scope.** 18 traits that have **zero** `dyn Trait` use sites,
distributed across 3 owner crates. Drop `#[async_trait]`, leave the
trait as plain `async fn` (stable 1.75 AFIT), delete the `async_trait`
macro imports from the crate. Impl blocks that live in other crates
(e.g. in-memory impls consumed by tests) get their `#[async_trait]`
removed in the same PR as their owner crate — downstream impl-only
crates do not get separate PRs.

Trait list by owner crate:

| Owner crate | Traits with 0 `dyn` sites | File anchors |
|---|---|---|
| `nebula-storage` (new `repos/*.rs` layer) | `WorkflowVersionRepo`, `AuditRepo`, `WorkspaceRepo`, `JournalRepo`, `CredentialRepo`, `ExecutionNodeRepo`, `BlobRepo`, `UserRepo`, `SessionRepo`, `PatRepo`, `OrgRepo`, `ResourceRepo`, `QuotaRepo`, `TriggerRepo`, newer `WorkflowRepo` (repos/workflow.rs:16), newer `ExecutionRepo` (repos/execution.rs:14) | `crates/storage/src/repos/*.rs` |
| `nebula-credential` | `NotificationSender`, `TestableCredential`, `RotatableCredential` | `crates/credential/src/rotation/{events.rs,validation.rs}` |
| `nebula-runtime` | `TaskQueue` | `crates/runtime/src/queue.rs:50` |

**Per-crate PR shape.**

1. Remove `#[async_trait]` attribute on each trait definition and each
   impl in the crate.
2. Delete the `async_trait` macro import in each `.rs` file touched.
3. Delete the `async-trait` dependency from the crate's `Cargo.toml` if
   no other consumer remains.
4. For any `spawn` site that breaks under inherent AFIT because the
   returned future captures a non-`'static` borrow, add an
   `impl Future<Output = …> + Send + use<>` (or the explicit generic
   whitelist) to the method signature. Inventory says no such sites
   exist for these traits today (they are all either consumed via
   generics or not spawned).
5. Update the crate's README if it mentions `async-trait`.
6. Run the canonical quickgate. Knife scenario must still pass on
   `storage` changes (see `crates/api/tests/knife.rs`).

**Risk.** Per-PR risk is small. No cross-crate coordination. Main gotcha
is impl blocks in downstream crates that were themselves decorated with
`#[async_trait]` — those also need the attribute off. Grep for
`impl <Trait> for` before merging.

**Verify.** Same as Phase 1 plus `cargo test --workspace --doc`.

**Acceptance.** `rg '#\[async_trait\]' crates/<touched-crate>/src/`
returns nothing; `async-trait` Cargo.toml entries drop to the crates
that still need it for Phase 3.

### Phase 3 — `dynosaur` migration for cross-crate `dyn` traits (5 PRs, ADR-tracked)

> **Cancelled (2026-04-20) — superseded by
> [ADR-0024](../../adr/0024-defer-dynosaur-migration.md).** The 14
> `dyn`-consumed traits stay on `#[async_trait]`. Rationale: `dynosaur`
> adoption is narrow (v0.3.0, 13 reverse-deps, aging), the dual
> generic+dyn value-prop serves zero current call sites (all 14 are
> 100 % dyn-consumed), and `async_fn_in_dyn_trait` will eventually make
> the whole macro class obsolete (tracking
> [rust-lang/rust#133119](https://github.com/rust-lang/rust/issues/133119)).
> The section below is retained for historical context and for the
> re-evaluation trigger defined in ADR-0024 §5.

**Scope.** The 14 traits that *are* consumed as `dyn` somewhere. One
coordinated PR per family; the ADR gate in Hazards below applies.

Ordered by fanout (highest first — where the most call sites change):

| # | Trait family | Owner crate | Dyn sites | Consumer crates | Notes |
|---:|---|---|---:|---|---|
| 1 | `TriggerHandler` | `nebula-action` | 18 | `action`, `api`, `sdk`, `apps/cli`, `apps/desktop` | Highest fanout. Action's integration seam. |
| 2 | `CredentialAccessor` | `nebula-credential` | 16 | `credential`, `engine`, `action`, `resource` | Hot path — preserving static dispatch is the point. |
| 3 | `StatelessHandler` | `nebula-action` | 14 | `action`, `sdk`, `apps/cli` | Internal action-crate seam mostly. |
| 4 | `ControlQueueRepo` | `nebula-storage` | 10 | `storage`, `engine`, `api` | Canon §12.2 durable control plane — extra care. |
| 5 | Storage legacy dyn pair | `nebula-storage` | `ExecutionRepo` (6), `WorkflowRepo` (4) | `storage`, `engine`, `api` | Keep split from `repos/*.rs` siblings. Batch into one PR. |
| 6 | Remaining action traits | `nebula-action` | `ResourceHandler` (9), `ResourceAccessor` (7), `ExecutionEmitter` (7), `StatefulHandler` (6), `TriggerScheduler` (5), `AgentHandler` (2) | `action`, `engine`, `sdk` | Bundle because same crate, same review context. |
| 7 | Remaining runtime/sandbox | `nebula-runtime`, `nebula-sandbox` | `StatefulCheckpointSink` (6), `BlobStorage` (2), `SandboxRunner` (2) | `runtime`, `sandbox`, `engine` | Smallest family. Last PR. |

Rows 6 and 7 can bundle (same-crate consolidations); rows 1–5 each
want their own PR.

**Per-PR shape (ADR-0014 alignment).**

1. Add `#[dynosaur::dynosaur(DynFoo)]` to the trait definition (AFIT form).
2. Drop `#[async_trait]` from the trait and every impl in the workspace.
3. At storage/registry sites, replace `Arc<dyn Foo>` with
   `Arc<dyn DynFoo>`. At static-dispatch sites, keep `impl Foo`.
4. Pin `dynosaur = "<exact>"` in `[workspace.dependencies]` (ADR-0014
   follow-up note — use exact pin so semver bumps are intentional).
5. Update crate README and, where relevant, the `*Metadata`/`*Schema`
   types' public docs.
6. Run the knife scenario (`crates/api/tests/knife.rs`) — this hits the
   engine's dispatch dyn-path for `TriggerHandler`, `ExecutionRepo`,
   `ControlQueueRepo` end-to-end.

**Risk.** Dynosaur is a young crate (ADR-0014 explicitly flags this).
Mitigations:

- Never downgrade via `cargo update` — exact pin.
- Every PR runs the full workspace test suite plus the knife scenario.
- If a trait has `Self: Sized` bounds, generic methods, or returns
  referencing `Self`, dynosaur refuses — move those methods to a
  sealed static-dispatch sibling trait before flipping. **None of the
  34 trait defs today use these shapes** (spot-checked the 14 dyn ones
  during inventory); if a future refactor adds one, revisit.

**Verify.**

```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo +1.95 check --workspace        # MSRV gate
cargo nextest run -p nebula-api --test knife  # full knife scenario
```

**Acceptance.** `rg '#\[async_trait\]' crates/ apps/` returns at most
the markdown design docs; `rg 'dynosaur::dynosaur' crates/` equals 14
(one per dyn-consumed trait); `async-trait` dep removed from every
crate's `Cargo.toml`; `[workspace.dependencies]` loses the
`async-trait = "0.1.89"` line.

### Phase 4 — `use<…>` precise-capture cleanup (inline, no standalone PR)

Happens inside Phase 2 and Phase 3 PRs when a compile error points at
the future returned from an AFIT method. No separate chip unless the
compiler finds something we missed — in which case one tidy-up PR
covers the stragglers.

Budget: ~22 touches across the 18 spawn-through-trait sites enumerated
under "precise capturing use<...>" above.

### Phase 5 — Late polish (parallel, each ~1 PR)

| Pattern | Shape |
|---|---|
| let-chains | One chip per crate, touch the 16 `if let Some = _ { if … }` nesting sites. Defer indefinitely if nobody cares. |
| Atomic `update` / `try_update` | Replace 5 `fetch_update` / `compare_exchange` loops in `telemetry`/`metrics` where the shape matches. |
| `inline const { … }` | Opportunistic only; no dedicated chip warranted. |
| `core::error::Error` | Only if a `no_std`-adjacent goal emerges (not today). |
| Async closures | One-hour pass over the 54 `Box::pin(async move)` sites; convert the stored `FnMut`-like cases, leave the incidental spawn-wrappers alone. |

## Hazards / things that need an ADR before code moves

| Hazard | Where | Action |
|---|---|---|
| `ExecutionRepo`, `WorkflowRepo`, `ControlQueueRepo` are part of `nebula-storage`'s workspace-internal surface. ADR-0021 (crate publication policy, PR #501) should decide whether these crates are `publish = true` before Phase 3 makes `dyn DynExecutionRepo` a rename. If `storage` stays `publish = false` this is a CHANGELOG note; if it flips to `publish = true` the rename is a SemVer breaking event that needs its own ADR. | `crates/storage/Cargo.toml` | Verify publication status with tech-lead before merging Phase 3 PR #4/#5. |
| `TriggerHandler` is re-exported through `nebula-sdk::prelude` (see audit finding about the 60-item glob prelude). A rename `dyn TriggerHandler` → `dyn DynTriggerHandler` in sdk consumer code is a visible API change, even if sdk is `publish = false`. Examples that depend on the current form exist in `examples/` workspace member. | `crates/sdk/src/prelude.rs`, `examples/**/*.rs` | Update `examples/` in the same PR; note that ADR-0014 §Style already prescribes the `Dyn*` naming, so this is intended behaviour — just not silent. |
| `CredentialAccessor` dyn migration touches `EncryptionLayer` composition path. ADR-0023 (KeyProvider, just landed PR #502) introduced a new seam right next to it; sequencing dynosaur here after ADR-0023 has stabilised avoids re-review of overlapping diffs. | `crates/credential/src/accessor.rs`, `layer/encryption.rs` | Phase 3 row #2 waits until ADR-0023 follow-ups close. |
| Dynosaur version selection. ADR-0014 §Follow-ups calls for exact version pinning; the workspace has no entry yet. First Phase 3 PR adds `dynosaur = "=<exact>"` to `[workspace.dependencies]`. | `Cargo.toml` | Include in Phase 3 PR #1 (`TriggerHandler`), not earlier. |
| Two-definition storage trait hazard. The PR that touches `crates/storage/src/workflow_repo.rs` and the PR that touches `crates/storage/src/repos/workflow.rs` must not run concurrently on different branches — they will merge-conflict over `lib.rs` re-exports. Keep them in the same PR (Phase 3 row #5). | `crates/storage/src/lib.rs:101` | Single PR, not parallel chips. |

## Out of scope

- **Edition 2024 migration beyond what 1.95 already implies.** ADR-0010
  already committed edition 2024; this plan doesn't revisit it.
- **GATs-on-futures / async stream traits.** Not a stable story yet
  workspace-wide; ADR-0014 explicitly calls out `trait-variant` as a
  future re-evaluation when the picture changes.
- **`no_std` support.** Nothing in this plan flips any crate to `no_std`.
  `core::error::Error` is mentioned only to *inventory* the surface.
- **`#[unstable(feature = …)]` gating.** Canon §11.6 feature-gating is
  orthogonal to toolchain feature adoption.
- **Internal refactors enabled by the migration.** E.g. merging
  legacy/new storage repo trait pairs, splitting
  `crates/engine/src/engine.rs` (7923 LOC) — both are independent
  audit action items owned by tech-lead (P1 #19 / ADR backlog) and are
  **not** coupled to this rollup.
- **`async-trait` in examples / docs.** Markdown design documents under
  `crates/*/docs/` and `crates/*/plans/` contain 28 `#[async_trait]`
  mentions that are illustrative, not compiled. Leave them; update on
  the next doc pass touching each file.

## Methodology notes

Counts at HEAD (2026-04-19, commit `62754680`) using the canonical
commands below so a reviewer can reproduce:

```bash
# async_trait in compiled code
rg --count-matches '#\[async_trait\]' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 88

# async_trait files
rg --files-with-matches '#\[async_trait\]' --glob '*.rs' crates/ apps/ examples/ | wc -l
# → 49

# dyn fanout per trait (example for TriggerHandler)
rg --count-matches '\bdyn\s+TriggerHandler\b' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 18

# once_cell surface
rg --count-matches 'once_cell' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 1

# #[allow] surface
rg --count-matches '#\[allow\(' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 116

# dynosaur today
rg --count-matches 'dynosaur' --glob '*.rs' crates/ apps/ examples/ \
  | awk -F: '{sum+=$2} END {print sum}'
# → 0
```

Re-run these before opening any Phase PR — the drift from audit-time
(+7 attrs, −5 files over ≈12 hours) is a reminder that these numbers
move fast.
