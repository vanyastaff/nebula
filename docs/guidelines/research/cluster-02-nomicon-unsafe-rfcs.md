# Cluster 02: Nomicon, Unsafe Guidelines, RFCs, Modern Rust

Synthesized from: **The Rustonomicon** (`doc.rust-lang.org/nomicon/print.html` full book fetch), **Unsafe Code Guidelines** glossary (`rust-lang.github.io/unsafe-code-guidelines/glossary.html`), **Rust RFC Book** (e.g. RFC 2094 NLL, RFC 1210 specialization), **Rustc Dev Guide** (`rustc-dev-guide.rust-lang.org/print.html` excerpts: type inference, trait resolution, MIR, coherence/typing mode), plus Rust Reference, std docs, Edition Guide 2024, and modern RFC clusters (async fn in trait, RPITIT, let-else, let-chains, unsafe-op-in-unsafe-fn).

**Supplement note (2026-04):** The blocks marked *Source-extracted* below quote or tightly paraphrase official book text for LLM grounding; the Rustonomicon warns it can lag the Reference — on conflict, prefer [The Reference](https://doc.rust-lang.org/reference/).

## [TAG: 08-unsafe-and-ffi] The Safe/Unsafe Boundary

- Rule: Safe Rust is sound by definition: no matter what you write in 100% safe Rust, you cannot cause Undefined Behavior. All memory/type-safety violations require crossing into unsafe. [Nomicon: meet-safe-and-unsafe]
- Rule: `unsafe` has four distinct uses, each with different semantics:
  1. `unsafe fn` declares the function has extra contract obligations the caller must uphold.
  2. `unsafe trait` declares implementors must uphold invariants the compiler cannot check (e.g. `Send`, `Sync`, `GlobalAlloc`).
  3. `unsafe impl` for an `unsafe trait` asserts the implementation upholds the trait's contract.
  4. `unsafe { ... }` block asserts that the enclosed unsafe operations have had their contracts verified by the programmer.
  [Nomicon: safe-unsafe-meaning]
- Rule: Safe code is allowed to trust unsafe code. Unsafe code must NOT trust arbitrary safe code — particularly generic safe code, where a parameter's trait impl (e.g. `PartialOrd`, `Clone`, `Deref`) could be a logic bomb. Mark traits `unsafe` whenever correctness of unsafe code depends on their correct implementation.
- Rule: `unsafe` is NOT a promise of correctness; the responsibility shifts to the programmer to uphold invariants. UB inside `unsafe` is still UB. [Reference: behavior-considered-undefined]
- Rule (Rust 2024, RFC 2585 / `unsafe_op_in_unsafe_fn`): Inside an `unsafe fn` you must still wrap unsafe operations in `unsafe { ... }`. The function-level `unsafe` only signals the caller-side contract; it no longer silently allows ad-hoc unsafe ops in the body. Lint `unsafe_op_in_unsafe_fn` warns by default in 2024, deny by default recommended.
- Rule (Rust 2024): `unsafe extern { ... }` is required — `extern` blocks are now required to be marked `unsafe` in 2024, making the unsafe nature of foreign bindings explicit at declaration.
- Rule (attribute unsafety, Rust 2024): Attributes that can cause UB (`#[no_mangle]`, `#[link]`, `#[export_name]`) now require `#[unsafe(...)]` syntax in 2024.
- Gotcha: Putting `unsafe` on a block does not limit the scope of reasoning. Unsafe code reliance spills out to anywhere that can violate the struct invariants it depends on — the effective scope is the enclosing module. Privacy (non-public fields) is the only airtight boundary. [Nomicon: working-with-unsafe]
- Pattern: Bury all unsafe-critical state in private fields of a module, so only code you control can break invariants. Safe code inside the same module that corrupts the invariant is still unsound. Example: mutating `Vec::cap` directly in safe code within the `Vec` module breaks every `push`'s unsafe block.

## [TAG: 08-unsafe-and-ffi] Soundness & UB Catalogue

- Rule (comprehensive UB list, Reference behavior-considered-undefined):
  - Data races (see concurrency section).
  - Dereferencing a dangling reference/pointer (for a non-zero-sized access). A pointer is dangling if its bytes are not all in one live allocation.
  - Misaligned pointer access — the required alignment is the *pointer type's* alignment, not the accessed field's. `(*ptr).f` where `ptr: *const S` must be aligned for `S`, not for the type of `f`.
  - Breaking aliasing rules: `&T` must see no writes to its memory except through `UnsafeCell`. `&mut T` must be unique — no other refs, no reads/writes via pointers not derived from it.
  - Mutating bytes of `static` / `const`-promoted / bindings / behind shared reference (except inside `UnsafeCell`).
  - Producing an invalid value (even temporarily, even in a transmute result you never read) for types with validity invariants. Canonical invalid values below.
  - Calling a compiler intrinsic incorrectly.
  - Executing instructions requiring target features not in the current build's target CPU features.
  - Calling with the wrong ABI.
  - Unwinding through a frame that doesn't allow unwinding (e.g. a `"C"` (without `-unwind`) function).
  - Incorrect inline asm.
  - Deallocating a Rust stack frame without running its destructors (e.g. `longjmp` over Rust code).
  - UB in foreign code called from Rust (or vice versa) is UB in the whole program.
  - Const-context provenance violations: storing pointer-with-provenance in an integer type, swapping pointer bytes then reading as reference.
- Rule (validity invariants — producing any value of these types that is invalid is immediate UB, even if you don't read it):
  - `bool`: must be the bit pattern `0x00` or `0x01`. Anything else is UB.
  - `char`: must be a Unicode scalar value — not in `0xD800..=0xDFFF` (surrogates) and `<= 0x10FFFF`.
  - `!`: never has any valid bit pattern (cannot exist).
  - Integer / float / raw pointer: must be fully initialized (no uninit bytes), BUT any bit pattern is a valid value.
  - `str`: must be valid UTF-8 AND fully initialized like `[u8]`.
  - `fn` pointer: must be non-null.
  - `&T`, `&mut T`, `Box<T>`: must be non-null, aligned, not dangling, pointing to a valid `T`. Additional aliasing rules as above.
  - Wide pointer metadata: `[T]` length must be `<= isize::MAX` bytes-worth; `dyn Trait` vtable must be a valid vtable for the trait.
  - `NonNull<T>`, `NonZero<T>`: obvious niche constraints.
  - `enum`: must be a valid discriminant AND the fields of that variant must be valid.
  - `struct` / tuple / array: every field/element must be valid.
  - `union`: only the bytes a safe constructor could write need to be valid; zero-sized fields trivially make any bit pattern valid.
- Rule: Uninitialized memory is *implicitly invalid* for any type with validity invariants. Only `MaybeUninit`, unions, and padding bytes may legally be uninit.
- Rule: UB is not "the code will still work most of the time"; LLVM will aggressively propagate the assumption that UB is unreachable. Code that "seemed to work" after committing UB cannot be relied upon across compiler versions.

## [TAG: 08-unsafe-and-ffi] Aliasing Model

- Rule: `&mut T` grants exclusive access to T. No other reference (`&T` or `&mut T`) to the same memory may coexist and be live with it. [Nomicon: aliasing, references]
- Rule: `&T` grants shared read access. Multiple `&T` may coexist. Under an `&T`, the memory must not be mutated except through bytes inside `UnsafeCell<U>`.
- Rule: These rules apply to live references. A reference is live from its creation until its last use (NLL/MIR-borrowck analysis), dereference, function pass/return, or reborrow.
- Rule: `Box<T>` is treated like `&'static mut T` for aliasing purposes (unique ownership, no aliased access to the underlying bytes except through the owner).
- Gotcha: Creating a `&mut T` that aliases any other reference or any other `&mut T` is UB even if the mutable reference is never used. The mere act of creation is UB. [Reference]
- Gotcha: Transmuting `&T` to `&mut T` is ALWAYS UB. Never do this, no exceptions. [Nomicon: transmutes]
- Rule: `UnsafeCell<T>` is the ONLY legal way to mutate through a shared reference. `Cell`, `RefCell`, `Mutex`, `RwLock`, `AtomicXxx` all compose on top of it. [Reference: interior-mutability]
- Rule: `UnsafeCell<T>` is `#[repr(transparent)]` over `T` — same layout and ABI — but disables niche optimization on `T` so that `Option<UnsafeCell<NonNull<u8>>>` is 16 bytes (vs 8 for `Option<NonNull<u8>>`).
- Rule: `UnsafeCell::get` returns `*mut T`; this must be the ONLY way to obtain a `*mut T` from a `&UnsafeCell<T>`. Casting the shared reference directly is UB.
- Rule: Even through `UnsafeCell`, two live `&mut T` borrows are still UB — interior mutability does not relax the uniqueness of mutable references, only the immutability of shared ones.
- Rule (optimization rationale): The compiler optimizes under the assumption that `&mut T` is unique. E.g. in `fn compute(input: &u32, output: &mut u32)`, the compiler may cache `*input` in a register across writes to `*output` because they cannot alias. [Nomicon: aliasing]
- Rule: Rust's formal aliasing model is still unfinalized. The operational semantics explored are Stacked Borrows and its replacement, Tree Borrows (implemented in Miri). Both forbid common C-like patterns: escaping raw pointer provenance, mutating through a `&T`, using `from_raw_parts_mut` on overlapping regions, etc.

## [TAG: 08-unsafe-and-ffi] Raw Pointers, Provenance, Strict Provenance

- Rule: Raw pointers carry (address, provenance). Provenance is permission (spatial: which bytes, temporal: which window, and mutability). Accessing memory without provenance over it is UB. [std::ptr]
- Rule: Provenance cannot grow, be forged out of thin air, or be recombined across unrelated allocations. Reading bytes that witness provenance and then reinterpreting gives *some* provenance the compiler best-effort selects (with Exposed Provenance API); under Strict Provenance it's never allowed.
- API (Strict Provenance, stable 1.84+):
  - `ptr.addr() -> usize` — extract address, no cast-back permission.
  - `ptr.with_addr(addr: usize) -> *mut T` — create a new pointer that reuses `ptr`'s provenance with a different address.
  - `ptr.map_addr(|addr| ...)` — convenience combinator. Use this for tagged pointers.
  - `ptr::without_provenance(addr)` — produce a pointer with no provenance (useful for sentinels you never dereference).
- API (Exposed Provenance, fallback for bare-metal / legacy):
  - `ptr.expose_provenance() -> usize` — exposes the pointer's provenance to a conceptual global list.
  - `ptr::with_exposed_provenance::<T>(addr)` — forge a pointer by scanning the exposed list. Best-effort; UB if no match exists.
- Rule: `*const T` is covariant in T. `*mut T` is invariant in T (like `&mut T`).
- Rule: `NonNull<T>` is a `*mut T` guaranteed non-null, covariant in T. Use for self-referential / collection internals where you want covariance + null-niche. [std::ptr::NonNull]
- Rule: `NonNull<T>` is `!Send` and `!Sync` — same lint as raw pointers.
- Rule: `NonNull::dangling()` returns a non-null, aligned, never-dereferenceable pointer — used as a sentinel before allocation (e.g. `Vec::new()`).
- Rule: Converting `&T -> NonNull<T>` is fine; calling `.as_mut()` on it to create `&mut T` when the source was shared is UB unless the bytes are inside `UnsafeCell`.
- API (pointer methods that require alignment):
  - `ptr::read`, `ptr::write`: aligned + dereferenceable required.
  - `ptr::read_unaligned`, `ptr::write_unaligned`: no alignment required but still dereferenceable and within-provenance.
  - `ptr::read_volatile` / `write_volatile`: compiler cannot elide/reorder; used for MMIO.
  - `ptr::copy(src, dst, n)` = memmove (may overlap).
  - `ptr::copy_nonoverlapping(src, dst, n)` = memcpy; UB if regions overlap.
  - `ptr::swap_nonoverlapping`, `ptr::replace`, `ptr::drop_in_place`.
- Rule: `ptr::drop_in_place` runs the destructor of a `T` behind a pointer without deallocating. Essential for destructors in unsafe containers that control allocation (e.g. Vec, Box).
- Rule: `ptr::offset(n)` (and `add`, `sub`) require that the resulting pointer stay within, or one past, the same allocation as the original. Uses `isize` arithmetic — allocations are limited to `isize::MAX` bytes precisely for this reason.
- API (Rust 1.82+): `&raw const` and `&raw mut` operator syntax replaces `std::ptr::addr_of!` / `addr_of_mut!`. Use this to create a raw pointer to a place without first materializing a reference (critical for unaligned / uninit fields, e.g. inside `#[repr(packed)]`).
- Gotcha: Taking `&packed_struct.field` on a `#[repr(packed)]` field is UB because of misalignment — even reading the reference is UB. Use `&raw const packed_struct.field` and `read_unaligned`.

## [TAG: 08-unsafe-and-ffi] Uninitialized Memory & MaybeUninit

- Rule: `std::mem::uninitialized` and `std::mem::zeroed` are DEPRECATED and should never be used. They produce immediately-invalid values for most types (reference, bool, NonZero, enum, etc.) — calling them is UB even if the value is never read. [Nomicon: uninitialized; MaybeUninit docs]
- Rule: Use `MaybeUninit<T>` to work with uninitialized memory:
  - `MaybeUninit::uninit()` — no validity guarantees; any bytes.
  - `MaybeUninit::zeroed()` — zero bytes; only safe if all-zero is a valid T.
  - `.write(val)` — initialize without dropping prior contents; returns `&mut T`.
  - `.assume_init()` — moves out as T; UB unless fully initialized and bit-valid for T.
  - `.assume_init_read()` — reads a bitwise copy; dangerous for non-Copy types (double-drop).
  - `.assume_init_drop()` — drops in place.
  - `.assume_init_ref()` / `.assume_init_mut()` — borrow the T without moving.
- Rule: `MaybeUninit<T>` is `#[repr(transparent)]` — same size/alignment/ABI as T.
- Rule: `&mut T` ⟶ `&mut MaybeUninit<T>` is UB (exposes uninit memory to safe code that may try to read).
- Rule: `Option<MaybeUninit<bool>>` = 2 bytes (niche disabled). `Option<bool>` = 1 byte.
- Pattern (array-init): `[const { MaybeUninit::uninit() }; N]`, then fill, then transmute to `[T; N]`. On error, drop only the initialized prefix.
- Pattern (struct-init): `let ptr = uninit.as_mut_ptr(); (&raw mut (*ptr).field).write(value);` for each field, then `assume_init()`.
- Gotcha: Reading from an `&uninit_bytes` as any type with validity invariants is instant UB. Even "reading" a `bool` from uninit bytes is UB. Even *comparing* uninit bool to true may mis-optimize.
- Rule: Padding bytes inside `repr(Rust)` and `repr(C)` types are legally uninitialized; reading them as individual bytes is UB (except when reading inside `MaybeUninit<T>`).

## [TAG: 02-language-rules] Subtyping & Variance

- Rule: Subtyping in Rust exists for lifetimes (and by extension types containing them). `'long <: 'short` iff `'long` completely contains `'short`. So `'static <: 'any`.
- Rule: Variance determines how subtyping propagates through type constructors: [Nomicon: subtyping, Reference: subtyping]
  - Covariant F: `Sub <: Super` implies `F<Sub> <: F<Super>`.
  - Contravariant F: `Sub <: Super` implies `F<Super> <: F<Sub>`.
  - Invariant F: no relationship.
- Complete variance table (memorize):
  - `&'a T` — covariant in `'a`, covariant in `T`.
  - `&'a mut T` — covariant in `'a`, INVARIANT in `T`.
  - `*const T` — covariant in `T`.
  - `*mut T` — INVARIANT in `T`.
  - `Box<T>`, `Vec<T>`, `Rc<T>`, `Arc<T>` — covariant in `T`.
  - `Cell<T>`, `RefCell<T>`, `UnsafeCell<T>`, `Mutex<T>` — INVARIANT in `T`.
  - `fn(T) -> U` — CONTRAVARIANT in `T`, COVARIANT in `U`.
  - `[T]`, `[T; n]` — covariant in `T`.
  - `PhantomData<T>` — covariant in `T`; see PhantomData section.
  - `dyn Trait<T> + 'a` — covariant in `'a`, INVARIANT in `T` (type parameters of the trait).
- Rule: For a user-defined type, variance over parameter `A` is computed: covariant if all uses are covariant; contravariant if all uses contravariant; invariant if mixed or any invariant use.
- Rule (why `&mut T` is invariant in T): allowing `&mut &'static str <: &mut &'a str` would let you write a shorter-lived `&'a str` through the mutable reference into the `'static` slot, causing use-after-free when `'a` ends.
- Rule (why function arguments are contravariant): a function accepting `&'static str` is safely usable where `fn(&'a str)` is expected — the caller has strictly less; the function is fine being more demanding about callers.
- Gotcha: If you store a `fn(T) -> T` field, the param `T` is BOTH contravariant and covariant → invariant.
- Gotcha: `*mut T` and `&mut T` are invariant because of write-through. Storing a raw pointer in a struct makes the struct invariant in T unless you deliberately use `NonNull<T>` or `*const T` for covariance and manage mutation manually.
- Rule: Higher-ranked trait object or fn pointer: `for<'a> fn(&'a T) -> &'a T` is a subtype of `fn(&'static T) -> &'static T` (you can instantiate 'a with anything).

## [TAG: 04-design-patterns] PhantomData

- Rule: `PhantomData<T>` is a zero-sized marker used to declare that a type logically has a T even when no field of type T exists. Needed for:
  1. Correct variance.
  2. Drop-check ownership semantics (RFC 1238).
  3. Auto-trait propagation (Send/Sync).
  [Nomicon: phantom-data]
- Complete PhantomData variance+auto-trait+dropck table:
  - `PhantomData<T>` — covariant in T; Send/Sync inherit from T; dropck: owns T (most restrictive).
  - `PhantomData<&'a T>` — covariant in `'a` and T; requires `T: Sync` for Send; dropck: doesn't own.
  - `PhantomData<&'a mut T>` — covariant in `'a`, INVARIANT in T; inherits Send/Sync; dropck: doesn't own.
  - `PhantomData<*const T>` — covariant in T; `!Send + !Sync`; dropck: doesn't own.
  - `PhantomData<*mut T>` — INVARIANT in T; `!Send + !Sync`; dropck: doesn't own.
  - `PhantomData<fn(T)>` — CONTRAVARIANT in T; always `Send + Sync`; dropck: doesn't own.
  - `PhantomData<fn() -> T>` — covariant in T; always `Send + Sync`; dropck: doesn't own.
  - `PhantomData<fn(T) -> T>` — INVARIANT in T; always `Send + Sync`; dropck: doesn't own.
  - `PhantomData<Cell<&'a ()>>` — INVARIANT in `'a`; `Send + !Sync`; common trick to make a type `Send` but not `Sync`.
- Pattern: Iterator over `&'a [T]`:
  ```rust
  struct Iter<'a, T: 'a> {
      ptr: *const T,
      end: *const T,
      _marker: PhantomData<&'a T>,
  }
  ```
- Pattern: "Owning raw pointer" (e.g. `Box`-like, `Vec`-like internal):
  ```rust
  struct Unique<T> {
      ptr: *const T,      // *const for covariance
      _owns: PhantomData<T>, // own T for dropck + auto-traits
  }
  ```
- Rule: Since RFC 1238, if your type has a `Drop` impl, the compiler assumes it owns its generic parameters for dropck purposes — `PhantomData<T>` is redundant for dropck in that case (still needed for variance + auto-traits).
- Rule: The `#[may_dangle]` attribute (nightly, or via `unsafe_drop` machinery in std) lets you opt an `unsafe impl Drop` out of dropck for a specific generic parameter — use when your destructor *provably* doesn't touch T's value. `PhantomData<T>` disables this escape hatch (forces "owns T" semantics to win).

## [TAG: 02-language-rules] Drop Check & Destructor Order

- Rule: For a generic type to soundly implement Drop, its generic lifetime/type arguments must strictly outlive the value. [Nomicon: dropck]
- Rule: Struct fields are dropped in declaration order (top-to-bottom). Tuple fields in index order. Enum fields of the active variant in declaration order. Arrays/slices in reverse index order.
- Rule: Local variables are dropped in reverse order of declaration within their scope.
- Rule: A temporary in an expression is dropped at the end of the enclosing statement (unless lifetime-extended by `let`).
- Rule: Inside `Drop::drop(&mut self)`, you cannot move fields out of `self`. Workaround: wrap the field in `Option` and use `.take()`, or use `mem::replace` / `ManuallyDrop`.
- Rule: After your `drop` body runs, Rust automatically drops each field of `self` in order. You cannot suppress this except via `ManuallyDrop::new(...)` or `mem::forget`.
- Rule (`ManuallyDrop<T>`): Inhibits automatic drop. You must call `ManuallyDrop::drop(&mut x)` (unsafe) or `ManuallyDrop::into_inner(x)` yourself. Used by collections to control destructor timing (`mem::take`-style tricks).
- Rule (`mem::forget`): Marked SAFE. Leaks the value without running its destructor. Unsafe code MUST NOT assume destructors run; relying on drop for memory safety is unsound.
- Rule: Drop flags — the compiler inserts runtime bitflags on the stack to track whether partially-moved locals still need dropping. Unneeded when static analysis can prove the state unconditionally. [Nomicon: drop-flags]
- Rule: For a DST (`[T]`, `str`, `dyn Trait`), the destructor runs per-element / via the vtable's drop slot (first vtable entry).
- Pattern (RAII drop guard for panic safety):
  ```rust
  struct Guard<'a, T> { data: &'a mut Vec<T>, prev_len: usize }
  impl<T> Drop for Guard<'_, T> {
      fn drop(&mut self) { self.data.truncate(self.prev_len); }
  }
  ```
- Pattern ("Pre-poop your pants"): make unsafe state unobservable to external code *before* doing anything that can panic, so a panic leaves a valid-but-leaky state rather than an inconsistent one (see `Vec::drain`).
- Gotcha: `drop_in_place` on a `#[repr(packed)]` field is UB if it requires alignment — you must move to a local first.

## [TAG: 02-language-rules] Lifetimes, NLL, Elision, HRTBs

- Rule (NLL, RFC 2094): A borrow is live from its creation until its last use along each control-flow path, not until the end of its lexical scope. This unblocks:
  - Get-or-insert patterns that used to fail with "first borrow still live".
  - Conditional borrows where one branch uses a borrow and the other doesn't.
  - Reborrows that end before the next mutable borrow starts.
- Rule: Two-phase borrows (part of NLL) split a `&mut` into reserve and activate phases, so `vec.push(vec.len())` compiles (the `&mut vec` is reserved at call start, `vec.len()` takes `&vec`, then activate `&mut` at actual push).
- Rule (elision rules, complete — for `fn` / `impl` / trait / `Fn*` types):
  1. Each elided input lifetime becomes a distinct lifetime parameter.
  2. If there is exactly one input lifetime, it is assigned to all elided output lifetimes.
  3. If there are multiple inputs and one is `&self` or `&mut self`, the lifetime of `self` is assigned to all elided output lifetimes.
  If none of these apply, elision fails and the program is rejected. [Nomicon: lifetime-elision]
- Examples:
  - `fn get(s: &str) -> &str` → `fn get<'a>(s: &'a str) -> &'a str`.
  - `fn foo(&self, s: &str) -> &str` → `'self`.
  - `fn pick(a: &str, b: &str) -> &str` — REJECTED; must annotate.
  - `fn get() -> &str` — REJECTED; must annotate or return owned.
- Rule (unbounded lifetimes): Lifetimes arising from `transmute`, `transmute_copy`, `*const T` dereferences, or explicit annotations that don't tie back to any input are "unbounded" — they expand to whatever lifetime the context demands. This is a common source of use-after-free bugs. Always bind the returned lifetime to an input as soon as possible. [Nomicon: unbounded-lifetimes]
- Rule (HRTB, `for<'a>`): needed for types that must work for any lifetime, not a specific one. Canonical case: a closure bound that references its argument.
  ```rust
  fn higher<F>(f: F) where F: for<'a> Fn(&'a i32) -> &'a i32 { ... }
  ```
  HRTBs desugar to an infinite family of bounds. [Nomicon: hrtb]
- Rule (Rust 2024 RPIT capture): In 2024, `fn foo(x: &T) -> impl Trait` implicitly captures ALL in-scope lifetimes and type parameters (equivalent to `+ use<'_, T>`). Pre-2024 captured only those appearing syntactically in the bounds. Use `+ use<..>` to explicitly control captures. Migrate with `cargo fix --edition`.
  ```rust
  // 2024:  fn f<'a>(x: &'a ()) -> impl Sized {}   // captures 'a
  // 2021:  fn f<'a>(x: &'a ()) -> impl Sized {}   // does NOT capture 'a
  // Explicit:  -> impl Sized + use<>              // capture nothing
  //            -> impl Sized + use<'a, T>        // capture specific
  ```

## [TAG: 02-language-rules] Coherence, Orphan Rule, Specialization

- Rule (coherence/overlap): There can be at most one `impl` of a trait for any given type in the global trait resolution context. Conflicting impls are a compile error even across crates. [rustc-dev-guide: coherence — knowledge]
- Rule (orphan rule): You may write `impl Trait for T` only if:
  - `Trait` is local to your crate, OR
  - `T` is local to your crate (where "local" is determined by a walk over type params and fundamental types).
- Rule (`#[fundamental]` types): `&T`, `&mut T`, `Box<T>`, `Pin<T>` are marked fundamental — implementing `Trait for &LocalType` is treated as implementing for `LocalType`. This is why `&LocalType: YourTrait` is considered local.
- Rule: Blanket impls with a fully generic type parameter require the trait to be local. `impl<T> MyTrait for T` is fine (you own MyTrait). `impl<T> Display for T` is forbidden.
- Rule: Negative coherence (`impl !Send for Foo`) is stable only for auto-traits (`Send`, `Sync`, `Unpin`, `UnwindSafe`). Requires `#![feature(negative_impls)]` otherwise.
- Status: Full specialization (RFC 1210) is stuck on soundness issues (dropck + lifetime-dependent specialization is unsound). `min_specialization` is used inside rustc internally; not stable for users. Design work continues but don't rely on it.

## [TAG: 02-language-rules] Dyn Safety (Object Safety)

- Rule: A trait is dyn-compatible only if:
  - No `Self: Sized` supertrait.
  - All supertraits are dyn-compatible.
  - No associated constants.
  - No generic associated types with type params (GATs violate dyn compatibility).
  - Methods are dispatchable: no generic type parameters, no `where Self: Sized` (or if present, the method is not part of the vtable), no `Self` in return position, receiver is `self`, `&self`, `&mut self`, `Box<Self>`, `Rc<Self>`, `Arc<Self>`, `Pin<P>` where P is one of those.
  - No RPIT/async-fn return (these are opaque and per-impl).
  - `AsyncFn*` traits are never dyn-compatible.
- Pattern: keep Sized-only methods inside the trait by bounding them `where Self: Sized` — they're callable on concrete types but skipped in the vtable.
- Rule: Trait objects use a fat pointer: (data pointer, vtable pointer). Vtable layout: size, alignment, drop-in-place, methods in declaration order. Not ABI stable; never transmute between trait object layouts.
- Rule: Upcasting (`&dyn Sub` → `&dyn Super`) became stable in Rust 1.86. Downcasting between trait objects still requires `Any`.
- Rule: `dyn Trait` without `+ 'a` defaults to `+ 'static` in most positions (a trait object default-captures `'static`). Inside `Box<dyn Trait>`, this default applies; use `Box<dyn Trait + 'a>` to bind to a non-static lifetime.

## [TAG: 04-design-patterns] Sealed Traits

- Pattern: Prevent downstream crates from implementing a trait by making a supertrait with a private item only you can satisfy:
  ```rust
  mod private { pub trait Sealed {} }
  pub trait MyTrait: private::Sealed { /* public API */ }
  impl private::Sealed for MyType {}
  impl MyTrait for MyType { ... }
  ```
- Rationale: lets you add methods/associated-items to `MyTrait` without breaking downstream impls — only *you* have impls. Also lets you rely on an exhaustive set of types. Used pervasively in `std` (e.g. `os::unix::fs::OsStrExt`).
- Gotcha: Completely private sealed traits can block legitimate extension. Use sparingly and document.

## [TAG: 04-design-patterns] ZSTs (Zero-Sized Types)

- Rule: ZSTs have `size_of::<T>() == 0`. Examples: `()`, `PhantomData<T>`, empty structs/tuples, `[T; 0]`.
- Rule: ZSTs still have alignment ≥ 1. `size_of::<T>()` is always a multiple of `align_of::<T>()` — and 0 is a multiple of any alignment.
- Rule: References/pointers to ZSTs must still be non-null and aligned, BUT loading/storing zero bytes through any aligned non-null pointer is NOT UB. `ptr::read` and `ptr::write` are no-ops for ZSTs.
- Rule: Pointer arithmetic on ZSTs is a runtime no-op (the compiler optimizes it away). Offset-by-n on a `*T` where `size_of::<T>() == 0` does nothing. Collections that track "length by pointer subtraction" must special-case this by treating pointer-as-counter (see Nomicon Vec/ZST).
- Pattern: `HashSet<K>` implemented as `HashMap<K, ()>` — the `()` is free. [Nomicon: exotic-sizes]
- Rule: Vec of ZST — capacity is implicitly `usize::MAX`, no allocation occurs; length tracks how many logical elements exist. Do not call the allocator.
- Rule: `NonNull::dangling()` returns an aligned non-null pointer suitable as a placeholder for ZST "allocations".

## [TAG: 04-design-patterns] DSTs & Wide Pointers

- Rule: DSTs (`[T]`, `str`, `dyn Trait`, custom structs ending in DST fields) cannot be stored by value — they always live behind a pointer type (`&`, `&mut`, `Box`, `Arc`, `Rc`, `*const`, `*mut`, `NonNull`).
- Rule: A pointer to a DST is a "wide" (fat) pointer: `(data pointer, metadata)`:
  - `[T]` / `str`: metadata is `usize` length.
  - `dyn Trait`: metadata is vtable pointer.
  - Size: 2× `size_of::<usize>()`, alignment `align_of::<usize>()`.
- Rule: Custom DST: `struct MySlice<T: ?Sized> { header: u32, tail: T }` — when `T = [U]`, `MySlice<[U]>` is a DST. Construct via unsizing coercion from a sized version.
- Rule: `Box<dyn Trait>` / `Rc<dyn Trait>` / `Arc<dyn Trait>` are wide pointers too.
- Rule (slice metadata validity): the length field must satisfy `size_of::<T>() * len <= isize::MAX`. Otherwise UB.

## [TAG: 02-language-rules] Layout Rules (repr)

- Rule: `#[repr(Rust)]` (default) provides only soundness guarantees: fields are aligned, fields don't overlap, type alignment ≥ max field alignment. NO guarantees on ordering, padding, niche placement. Layout may differ between monomorphizations, compilations, versions, or even two identically-declared structs.
- Rule: `#[repr(C)]`:
  - Fields placed in declaration order.
  - For each field, offset is aligned up to field's alignment, then field placed, then offset += size.
  - Final size padded up to the struct's alignment (max field alignment).
  - Matches C ABI struct layout. Use for FFI.
  - DST-terminating, wide-pointer, and Rust tuples are NOT FFI-safe under `repr(C)`.
  - Fieldless enum with `repr(C)` has platform's C-enum layout; constructing with an invalid discriminant bit pattern is UB.
- Rule: `#[repr(transparent)]`:
  - Struct or single-variant enum with exactly one non-ZST field (any number of ZST fields allowed).
  - Layout + ABI identical to the non-ZST field.
  - Transmuting between wrapper and inner is sound (at layout level; semantic invariants are your problem).
  - Used by `UnsafeCell`, `MaybeUninit`, `NonNull`, newtype wrappers.
- Rule: `#[repr(u8/u16/u32/u64/usize/i8/...)]`:
  - Only on enums.
  - Fieldless: discriminant has specified size; integer cast to/from enum works.
  - With fields (tagged union), lays out as `repr(C)` union-of-structs with prepended discriminant.
  - SUPPRESSES the niche/null-pointer optimization for `Option<Enum>`.
- Rule: `#[repr(packed)]` / `#[repr(packed(N))]`:
  - Forces alignment to at most N (default 1). Removes inter-field padding.
  - Taking a reference to a packed field is UB if misaligned. Use `&raw const field` + `read_unaligned`.
  - Cannot combine with `repr(align(N))`.
  - Avoid except for wire-format parsing.
- Rule: `#[repr(align(N))]`:
  - Forces minimum alignment to N (power of two). Useful for cache-line alignment in concurrent code (typical N = 64 or 128).
  - Cannot combine with `repr(packed)`.
- Niche optimization: In `repr(Rust)`, compiler places enum discriminants in forbidden bit patterns of fields: `Option<&T>` = sizeof `&T` (null is the None); `Option<NonZeroU32>` = 4 bytes; `Option<bool>` = 1 byte (using 2..=255); `Option<Option<bool>>` = 1 byte.
- Rule: Guaranteed niches: `&T`, `&mut T`, `Box<T>`, `NonNull<T>`, `NonZeroU*`/`NonZeroI*`, function pointers (non-null), `char` (not a surrogate). These are ALWAYS niched in `Option` (and similar 2-variant enums).

## [TAG: 02-language-rules] Concurrency: Send, Sync, Data Races

- Rule: `Send` = "moving to another thread is safe". `Sync` = "`&T` can be shared across threads safely" = "T is Sync iff &T is Send". [Nomicon: send-and-sync]
- Rule: `Send`/`Sync` are AUTO TRAITS. If all fields of a struct are Send, the struct is automatically Send. You opt out by including a `!Send` or `!Sync` field (e.g. `PhantomData<*const ()>`, `Rc<T>`, `UnsafeCell<T>`).
- Rule: `*const T`, `*mut T`, `UnsafeCell<T>`, `Rc<T>` are all `!Send + !Sync` by default. `RefCell<T>` is `!Sync`. `Cell<T>` is `!Sync`. `MutexGuard<T>` is `!Send` but `Sync`.
- Rule: Custom `unsafe impl Send/Sync` shifts the soundness obligation to you. Must verify: no unsynchronized shared mutable state; destructors behave correctly when moved across threads; all contained types are Send/Sync as appropriate.
- Rule: A data race is (≥2 threads, ≥1 writer, at least one unsynchronized access) and is immediate UB. [Nomicon: races]
- Rule: Race conditions (order-dependent behavior that is semantically wrong) are NOT UB by themselves — only actual data races. Safe Rust can have race conditions; it cannot have data races.
- Rule: Mixed-size atomic accesses on the same memory concurrently are UB even if all are atomic (e.g. one thread `AtomicU16::store` and another `AtomicU8::store` through transmute). Atomic accesses must be the same size to race-safely.

## [TAG: 08-unsafe-and-ffi] Atomics & Memory Ordering

- Rule: Orderings (strength increasing): `Relaxed < Release/Acquire < AcqRel < SeqCst`.
- Rule (Relaxed): atomicity only; no happens-before beyond "this operation is indivisible". Use for counters where readers don't need synchronized view of other data (e.g. Arc's clone fetch_add).
- Rule (Acquire): on a load — all subsequent memory ops (in program order) cannot be reordered before. Pairs with Release.
- Rule (Release): on a store — all prior memory ops cannot be reordered after. Pairs with Acquire.
- Rule (AcqRel): for RMW (read-modify-write) — load side is Acquire, store side is Release. Use for CAS loops / lock acquire.
- Rule (SeqCst): everything AcqRel plus a single total order over all SeqCst operations across the program. Slowest; required only when you need a global total order (Dekker-style algorithms, double-checked locking variants).
- Rule: `compare_exchange` vs `compare_exchange_weak`:
  - Strong: never fails spuriously. Use when you're only doing a single CAS.
  - Weak: may fail spuriously (typical on ARM LL/SC), cheaper in a loop. Use in a retry loop.
  - Both take two orderings: success ordering (applied on success) and failure ordering (applied on failure). Failure ordering must not be stronger than success ordering and must not be Release/AcqRel.
- Rule: Fences:
  - `fence(Ordering::Release)` — fence-as-release; all prior ops synchronized.
  - `fence(Ordering::Acquire)` — fence-as-acquire; all subsequent ops synchronized.
  - `compiler_fence(...)` — prevents compiler reorder only; no hardware barrier. Use for signal handlers.
- Patterns:
  - Spinlock acquire: `while lock.compare_exchange(false, true, Acquire, Relaxed).is_err() { hint::spin_loop(); }`
  - Spinlock release: `lock.store(false, Release);`
  - Arc clone: `fetch_add(1, Relaxed)` — no synchronization needed on acquisition, only ownership increment. Abort if count ≥ `isize::MAX` to prevent `mem::forget` bombs.
  - Arc drop: `fetch_sub(1, Release)`; if result was 1, `fence(Acquire)` before deallocation. The Release prevents stores to the data from being reordered past the decrement; the Acquire fence ensures this thread sees all prior Releases before it deallocates.
- Rule: `Ordering::Consume` is not exposed in Rust (gets promoted to Acquire). Matches C++ reality where no compiler implements true consume.
- Platform: x86/64 is strongly ordered — Acquire/Release are basically free. ARM/ARM64/RISC-V weakly ordered — incorrect code that "works on x86" will break. Test concurrent code on weakly-ordered hardware (or Miri with `-Zmiri-many-seeds`).

## [TAG: 08-unsafe-and-ffi] Panic, Unwind, Exception Safety

- Rule: Rust has two panic strategies: `panic=unwind` (default) runs destructors; `panic=abort` just aborts the process. Set per-profile in Cargo.toml.
- Rule: Unwinding across an FFI boundary is UB UNLESS the ABI is `*-unwind` (stable since 1.71). Default `extern "C"` panics must be caught with `catch_unwind` before returning to C. `extern "C-unwind"` / `"system-unwind"` / `"stdcall-unwind"` allow panics to cross the boundary (used for C++ interop that expects structured exceptions). [Nomicon: unwinding, ffi]
- Rule: A foreign exception entering Rust via a non-`-unwind` ABI is UB (best case: abort; worst: corrupted state). With `-unwind` ABI, foreign exceptions can unwind through Rust frames, running `Drop` impls.
- Rule: `std::panic::catch_unwind` catches unwinding panics only — it doesn't catch aborts (`panic=abort` builds, double panics, stack overflows, OS aborts). Cost is near-zero on no-panic paths.
- Rule: Exception safety levels:
  - Minimal (unsafe code): must not cause UB if a panic interrupts mid-operation.
  - Maximal (safe code): should leave data in a coherent state. Panic-safety implies "basic exception guarantee" — no resource leaks, no inconsistent invariants that safe code could observe.
- Rule (exception-safety canonical pitfall — `Vec::push_all`-style):
  ```rust
  self.set_len(self.len() + n);        // BUG: invariant now broken
  for i in 0..n {
      ptr::write(..., clone()); // panic here leaves uninitialized slots inside [0..len)
  }
  ```
  Fix: increment len AFTER each write, or use a scope guard that truncates len on drop.
- Rule (double-drop via re-entrancy): if you create two live copies of a value (e.g. `BinaryHeap::sift_up` moving an element to a hole while a user-supplied comparator panics), both copies get dropped. Use a `Hole<T>` RAII guard that writes the element back on drop.
- Rule (poisoning): `Mutex` / `RwLock` poison themselves when a holder panics, so future `.lock()` calls return `Err(PoisonError { .. })`. The guarded data may be in an inconsistent state. `PoisonError::into_inner` lets you recover anyway. Poisoning is advisory; in Rust 1.83+ `Mutex::clear_poison` exists. [Nomicon: poisoning]
- Rule: Destructors can panic, but panicking during drop while already unwinding aborts the process.
- Pattern (drop guard):
  ```rust
  struct Guard<'a, T> { vec: &'a mut Vec<T>, prev_len: usize }
  impl<T> Drop for Guard<'_, T> { fn drop(&mut self) { unsafe { self.vec.set_len(self.prev_len); } } }
  ```

## [TAG: 08-unsafe-and-ffi] Leaking / Pre-Pooping

- Rule: Leaks do NOT violate memory safety in Rust's model. `mem::forget`, `Box::leak`, `Rc` cycles, infinite loops — all legal, all safe. [Nomicon: leaking]
- Consequence (important): unsafe code MUST NOT rely on destructors running for soundness. Any invariant you need must survive `mem::forget`.
- Historical casualty: `thread::scoped::JoinGuard` (removed — RFC 1066 aka "Leakpocalypse") required its destructor to run to maintain scope safety; forgetting it allowed a child thread to outlive its parent's stack. Replaced by `std::thread::scope` (the scope function, not a guard).
- Pattern (leak amplification): if you're about to temporarily invalidate something, preemptively mark the whole structure as leaked/empty so that forgetting your guard just leaks more memory but leaves no UB. Example: `Vec::drain` sets `vec.len = 0` at start; if Drain is `mem::forget`ten, elements are leaked but the Vec is consistent.
- Pattern: always assume the `Drop` on a scope guard might not run. Don't make safety contingent on it.
- Rule: `Box::leak(b) -> &'a mut T` produces a leak to get a `'static` (or bounded-by-caller's-lifetime) reference. Useful for initialization-time globals; memory is reclaimed only via `Box::from_raw` on the original pointer.
- Rule: `Arc`/`Rc` cycles — cycles of strong references leak. Break with `Weak`.

## [TAG: 08-unsafe-and-ffi] FFI

- Rule: `extern { ... }` blocks declare foreign items. In Rust 2024 this must be `unsafe extern { ... }`.
- Rule: Foreign function calls are always `unsafe`. Compiler cannot check signatures against reality; wrong signature → UB.
- Rule: Supported ABIs include `"C"` (default, platform C ABI), `"system"` (stdcall on 32-bit Windows x86, else C), `"Rust"` (unstable ABI — the default for Rust calls), plus platform-specific (`"stdcall"`, `"cdecl"`, `"fastcall"`, `"vectorcall"`, `"aapcs"`, `"win64"`, `"sysv64"`, `"thiscall"`).
- Rule: `"Rust"` ABI is unspecified, unstable, may change across versions. NEVER expose a Rust function via `#[no_mangle]` without `extern "C"` (or appropriate ABI).
- Rule: `#[link(name = "foo")]` asks the linker for `-lfoo`. `kind = "static"` vs `"dylib"` vs `"framework"` (macOS only).
- Rule: `#[unsafe(no_mangle)]` (2024 syntax; previously `#[no_mangle]`) disables name mangling — used when exporting Rust functions to be called from C.
- Rule: `#[unsafe(export_name = "foo")]` — explicit export symbol name.
- Rule (what's FFI-safe):
  - Primitive integers / floats / bool.
  - `repr(C)` structs/unions/enums.
  - `#[repr(transparent)]` newtypes.
  - `Option<NonZeroU*>` and `Option<NonNull<T>>` and `Option<extern "C" fn(..)>` — niched to C-compatible pointer/int types.
  - `*const T`, `*mut T` (including `*const [T]` for raw slice — but slice is NOT FFI-safe as a value, the wide pointer has no C analogue).
  - C string via `*const c_char` (use `CStr` / `CString`).
- Rule (NOT FFI-safe without care):
  - `&T`, `&mut T`, `Box<T>` — technically guaranteed non-null but rust-side semantics (aliasing) must be upheld by the C side.
  - `str`, `String` — no null terminator, wide pointer.
  - `[T]` value — wide pointer.
  - `dyn Trait`.
  - `Vec<T>`, `HashMap<K,V>`, etc. — Rust internal layout.
  - Enums with fields without `repr(C, u*)`.
- Gotcha: the `lint improper_ctypes` warns on most of the above. Heed it.
- Pattern (opaque foreign type, aka C-style incomplete type):
  ```rust
  #[repr(C)]
  pub struct Opaque { _data: [u8; 0], _marker: PhantomData<(*mut u8, PhantomPinned)> }
  ```
  Prevents instantiation, prevents Send/Sync/Unpin auto-impl, prevents the compiler from assuming a finite size.
- Rule: Global statics from C: `unsafe extern { static NAME: T; static mut NAME: T; }`. All access is `unsafe`. Mutable statics are a footgun — all access races with any other code that touches them.

## [TAG: 08-unsafe-and-ffi] Transmute, Casting, Conversions

- Rule: `mem::transmute<T, U>(x)` requires `size_of::<T>() == size_of::<U>()` (checked statically). Everything else is UB you own.
- Rule: Transmuting to a type with validity invariants is UB if the source bits don't happen to be valid (e.g. `transmute::<u8, bool>(2)` — UB).
- Rule: Transmuting between `repr(Rust)` compound types is NEVER reliable — even two identically-declared structs can have different layouts.
- Rule: Transmuting `&T` to `&mut T` is ALWAYS UB.
- Rule: Transmuting to a reference without a bound lifetime produces an unbounded lifetime — caller-side UB.
- Rule: `mem::transmute_copy<T, U>(&t)` lifts the size check; UB if `size_of::<U>() > size_of::<T>()`.
- Rule (as-casts):
  - Integer widening (unsigned): zero-extend; (signed): sign-extend.
  - Integer narrowing: truncate.
  - Integer → bool: not allowed (use `x != 0`).
  - Float → int: saturating since 1.45 (NaN → 0, infinities → min/max).
  - Int → float: rounds to nearest, never UB.
  - Pointer → pointer: bit-preserving; fat→thin drops metadata; thin→fat is NOT allowed. Use `ptr::slice_from_raw_parts` etc.
  - `*const [u16] as *const [u8]`: gotcha — length is NOT adjusted.
  - Function pointer → usize / pointer: allowed but under strict provenance, prefer `expose_provenance`.
- Rule: casts are not transitive. `e as T1 as T2` does not imply `e as T2` is legal.
- Rule: Reinterpreting via `union` fields is well-defined: writing one field and reading another is a "type pun". UB still occurs if the read value violates the target type's validity invariants.

## [TAG: 02-language-rules] Interior Mutability Hierarchy

- Rule: `UnsafeCell<T>` — fundamental primitive. `!Sync`. Raw `.get() -> *mut T`.
- Rule: `Cell<T>` — single-threaded. No borrowing — only `get()` (requires `T: Copy`), `set(v)`, `replace(v)`, `take()`, `swap(other)`. `!Sync`.
- Rule: `RefCell<T>` — single-threaded. Runtime borrow checking: `borrow() -> Ref<T>`, `borrow_mut() -> RefMut<T>`. Panics on borrow violation. `!Sync`.
- Rule: `Mutex<T>` — threaded exclusion lock. `lock() -> LockResult<MutexGuard<T>>`. Poisons on panic.
- Rule: `RwLock<T>` — threaded multi-reader / single-writer. Writer starvation on some platforms (Windows fair-ish; Linux pthreads depends).
- Rule: `OnceCell<T>` / `OnceLock<T>` — write-once. `OnceLock` is threaded. Use for lazy statics (replacing `lazy_static`).
- Rule: `LazyCell<T>` / `LazyLock<T, F>` (stable 1.80+) — lazy init closure. Preferred idiomatic lazy-global.
- Rule: `AtomicXxx<T>` — lock-free for primitives. Based on UnsafeCell; full Sync.

## [TAG: 08-unsafe-and-ffi] Pin & Self-Referential Types

- Rule: `Pin<P>` where P is a pointer — promises the pointee will not be moved (unless pointee is `Unpin`). Library concept; no compiler magic. [std::pin]
- Rule: `Unpin` is an auto trait. Almost every type is `Unpin`. A `!Unpin` type marks itself by containing `PhantomPinned` (or a nested `!Unpin` field).
- Rule: `Pin<&mut T>` with `T: Unpin` == `&mut T` with no extra constraints. Only `!Unpin` types actually "pin" under `Pin`.
- Rule (drop guarantee): once you put a `!Unpin` value into `Pin<..>`, its memory must not be deallocated/reused until its `drop` runs. `mem::forget` is OK (memory leaked, never reused); writing a new value into the same slot is UB.
- Rule: `async fn` and `async {}` produce anonymous `!Unpin` futures. Polling them requires `Pin<&mut Future>`.
- Rule: `Box::pin(t) -> Pin<Box<T>>` is the easy heap path. `std::pin::pin!(expr)` creates a stack-pinned `Pin<&mut T>` tied to the local.
- Rule: Pin projection:
  - Non-structural: field is not pinned even if struct is. Safe. Field can be `!Unpin` and still moved freely. You must not rely on pinning for that field.
  - Structural: pinning propagates to field. Requires: cannot impl `Unpin for Self` unless field is `Unpin`; `Drop` must be written as if receiving `Pin<&mut Self>`; no move-out methods; not `#[repr(packed)]`.
- Pattern: use `pin-project-lite` or `pin-project` to generate correct projection methods.
- Gotcha: Writing a `Drop::drop(&mut self)` for a `!Unpin` type — the body must treat `self` as if `Pin<&mut Self>`. Wrap explicitly: `fn drop(&mut self) { let this = unsafe { Pin::new_unchecked(self) }; ... }`.
- UB: moving out of a pinned `!Unpin` value. Calling `mem::replace` or `mem::swap` through `Pin<&mut T>` for `T: !Unpin` is UB unless wrapped carefully.

## [TAG: 12-modern-rust] async / await, Futures

- Rule (since Rust 1.39, RFC 2394): `async fn` desugars to a function returning `impl Future<Output = T>`. `async {}` blocks produce unnamed anonymous futures. `.await` is only legal inside async contexts.
- Rule: `Future::poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>`. Returns `Poll::Ready(T)` or `Poll::Pending`. Must return `Pending` only after registering a waker from `cx.waker()`.
- Rule: The compiler-generated state machine for `async fn` may be self-referential (references to locals across `.await` points), so the type is `!Unpin`. Must be pinned before polling.
- Rule (async fn lifetimes): `async fn foo<'a>(x: &'a str) -> &'a str` desugars such that the returned future captures `&'a` — the future cannot outlive `'a`. 2021: explicit `Captures<'a>` needed in some bounds; 2024: implicit via `use<..>`.
- Rule (Send bounds): `impl Future<Output = ()> + Send`. A future that holds a `!Send` type across an `.await` is `!Send`. Common pitfall: holding an `Rc<T>` or `MutexGuard<T>` over `.await`. Use `Mutex` from `parking_lot`/`tokio` or scope the lock.
- Rule (cancellation): dropping a future is cancellation. Design async functions to be cancel-safe — cleanup happens via destructors. `tokio::select!` branches run to first completion; other futures are dropped.
- Rule (runtime-agnostic): std provides `Future`, `Waker`, `Context`. `tokio` / `async-std` / `smol` / `embassy` provide executors. No built-in executor in std.
- Rule: `async fn` in `Fn` traits is NOT yet representable — use `impl Future` return or boxed trait objects.

## [TAG: 12-modern-rust] RPITIT, AFIT, Send bounds

- Rule (RFC 3425, stable 1.75): return-position impl-trait-in-trait (RPITIT) and async-fn-in-trait (AFIT):
  ```rust
  trait Service {
      fn handle(&self, req: Request) -> impl Future<Output = Response>;
      // or equivalently
      async fn handle(&self, req: Request) -> Response;
  }
  ```
- Rule: each impl of an RPITIT/AFIT trait can have its own opaque return type. These traits are NOT dyn-compatible (each impl's hidden type differs).
- Rule (Send bound limitation before 1.75): there is no syntax in the trait definition to say "all impls must return a `Send` future". Workarounds:
  - `#[trait_variant::make(Send)]` + crate.
  - `async-trait` crate (boxes everything — `Pin<Box<dyn Future + Send + 'a>>`).
  - Explicit `-> impl Future<Output = T> + Send` if you accept that the return type is non-`Send`-choosable per impl (defeats purpose).
- Rule (Rust 1.85, RFC 3668 "return-type-notation" / "sendness"): stabilized `trait Service { async fn handle(&self); } fn spawn<S: Service<handle(..): Send> + Send + 'static>(s: S)` — `Trait<method(..): Bound>` syntax for applying bounds to method return types.
- Rule (tooling): `#[trait_variant::make(Trait: Send)]` generates a `Send`-bounded sibling trait.
- Rule (dyn compat for async traits, 1.85+): `dyn Trait` where Trait has async fn is still NOT supported without boxing helpers. Use `async-trait` for type-erased async traits.

## [TAG: 12-modern-rust] let-else, if-let chains, let chains

- Rule (let-else, stable 1.65):
  ```rust
  let Some(x) = option else { return; };
  ```
  The `else` block must diverge (`return`, `panic!`, `continue`, `break`, `!`-typed expr). Used heavily to flatten early-exit guard patterns.
- Rule (if-let chains / let chains, stable 1.88+ in 2024 edition):
  ```rust
  if let Some(x) = a && let Some(y) = b && x > 0 {
      // both matches succeeded AND predicate holds
  }
  ```
  Only `&&` — no `||`. `let` bindings must be at the top level, not parenthesized. Edition-gated to 2024 (RFC 2497 behavior but reshaped to avoid prior inconsistencies).
- Rule (RFC 3137 if-let rescoping in 2024): the temporary in the `if let` scrutinee is dropped at the end of the `if`/`else` arm, not at the end of the enclosing statement. Fixes a historical footgun where `MutexGuard`s held across arms deadlocked.
- Pattern: replace nested `match` with `let-else`; replace nested `if let { if let { ... } }` with `if let && if let && ...`.

## [TAG: 12-modern-rust] Rust 2024 Edition Highlights

- Never type fallback: `!` now defaults to `!` (diverges) instead of `()`. In 2024+, `fn foo<T>() where T: Default` called as `foo()?` with `?` flowing to a diverging context keeps `!` instead of silently becoming `()`. The lint `never_type_fallback_flowing_into_unsafe` is deny by default.
- RPIT captures all in-scope generics implicitly (see Lifetimes section above). Opt out with `+ use<>`.
- `unsafe_op_in_unsafe_fn` warns by default.
- `unsafe extern` required on extern blocks.
- `#[unsafe(attr)]` required for `no_mangle`, `link`, `export_name`.
- Temporary scope for `if let` scrutinee tightened (RFC 3137).
- `gen { ... }` block syntax — RFC 3513 — stabilized (Rust 1.89+) providing synchronous generators yielding via `yield`.
- `core::ptr::read` / `write` — now with strict provenance available.
- Cargo resolver v3 by default in 2024 (MSRV-aware resolution).
- Module prelude refresh (adds `Future`, `IntoFuture`).
- Tail expression temp scope narrowed (avoids spurious lifetime extensions in tail positions).

## [TAG: 12-modern-rust] Disjoint Closure Captures (RFC 2229)

- Rule (2021+): closures capture only the *fields* they actually use, not the whole struct. Before 2021, `|| x.a` captured all of `x`.
- Consequence: a closure using `x.a` and another closure using `x.b` can coexist as mutable borrows. Prior to 2021 they would conflict.
- Consequence (edge case): drop order of captured fields may differ from the struct's natural order. Migration lint `rust_2021_incompatible_closure_captures` flags cases where this matters.
- Rule: `move` closures capture by value per field — individual fields, not the whole struct.

## [TAG: 12-modern-rust] Generic Associated Types (GATs)

- Rule (stable 1.65): associated types can take generic parameters, including lifetimes:
  ```rust
  trait Iter { type Item<'a> where Self: 'a; fn next<'a>(&'a mut self) -> Option<Self::Item<'a>>; }
  ```
- Rule: GATs open up "lending iterators" — iterators yielding references that borrow from self each iteration (impossible with plain `Item`).
- Rule: Using GATs disables dyn-compatibility of the trait.
- Gotcha: `where Self: 'a` bound is almost always required on the GAT — the lang team stabilized with warnings suggesting it.

## [TAG: 09-performance] When Unsafe is Warranted for Performance

- Rule of thumb: DON'T reach for unsafe unless profiled. Safe Rust with iterators frequently matches hand-rolled C.
- Reasonable unsafe wins:
  - `slice::get_unchecked` / `get_unchecked_mut` inside a hot loop where bounds are already proven — lets LLVM unroll/vectorize.
  - `Vec::set_len` after a bulk write — skip per-element reallocation / length updates.
  - `MaybeUninit::uninit_array` + bulk init for large arrays that would zero-init via `Default`.
  - `ptr::copy_nonoverlapping` for memcpy-style bulk moves when `slice::copy_from_slice` doesn't quite fit.
  - `str::from_utf8_unchecked` when you have externally validated bytes.
  - `NonZeroU*` / `NonNull<T>` niche optimization for data structures.
  - SIMD via `std::simd` (portable, stable in 1.77+) or `core::arch::*` intrinsics (unsafe).
- Pattern (bulk init without zeroing):
  ```rust
  let mut buf: Box<[MaybeUninit<u8>]> = Box::new_uninit_slice(n);
  // fill buf...
  let buf: Box<[u8]> = unsafe { buf.assume_init() };
  ```
- Pattern (SIMD intrinsics gating):
  ```rust
  #[target_feature(enable = "avx2")]
  unsafe fn hot(x: &[u8]) { ... }
  if is_x86_feature_detected!("avx2") { unsafe { hot(x) } } else { fallback(x) }
  ```
- Gotcha: target_feature + panic=unwind have interaction subtleties — an unwind across a target_feature boundary without the same features enabled is UB. Prefer `panic=abort` in hot SIMD crates.
- Rule: `#[inline(always)]` and unsafe do not mix automatically — always measure. LTO often makes it unnecessary.

## [TAG: 05-anti-patterns] Unsafe Anti-Patterns

- Anti-pattern: `unsafe { /* arbitrary big block */ }` instead of isolating the minimal unsafe operation. Make unsafe blocks as small as possible and follow each with a `// SAFETY:` comment explaining why the invariants hold.
- Anti-pattern: "It worked in my tests" — UB may silently misoptimize only in release mode or with newer compilers. Run Miri (`cargo +nightly miri test`) on all unsafe code.
- Anti-pattern: `mem::uninitialized::<T>()` — deprecated; use `MaybeUninit`.
- Anti-pattern: `mem::zeroed::<T>()` for references / non-zero types — UB. Only safe for types with explicit zero-validity.
- Anti-pattern: transmuting `&T` to `&mut T`. Always UB.
- Anti-pattern: transmuting between `repr(Rust)` compound types. Layout is not stable.
- Anti-pattern: casting `*const T` to `*mut T` then writing. Fine by itself; UB if the original bytes were immutable (e.g. behind `&T`, in a `static`, etc.).
- Anti-pattern: constructing `&T` to a place that may be mutated (aliasing violation). Even if you don't dereference the `&T` further, creating it is UB per the reference-validity invariant.
- Anti-pattern: relying on `Drop` for safety of unsafe code. Wrong assumption — `mem::forget` is safe. Make unsafe invariants robust against leaks.
- Anti-pattern: exposing a safe API that can be misused to cause UB. Example: a `Vec::set_len` that's safe — instantly unsound. All such fns MUST be `unsafe fn`.
- Anti-pattern: unsafe code that trusts generic arguments for correctness without `unsafe trait` bounds. E.g. `unsafe` container that assumes `T: PartialOrd` is consistent — a malicious `PartialOrd` impl can break invariants. Use `unsafe trait TrustedPartialOrd` or check bounds.
- Anti-pattern: `#[no_mangle] pub fn foo(..)` without `extern "C"`. You're exposing the Rust ABI, which is not stable.
- Anti-pattern: FFI without `catch_unwind` around the Rust code when using a non-`-unwind` ABI.
- Anti-pattern: `#[repr(packed)]` + `&field` — UB. Use `&raw const`.
- Anti-pattern: skipping `UnsafeCell` for "thread-local-only" shared-mutable state. Even single-threaded, you still need UnsafeCell to avoid aliasing UB through `&T`.
- Anti-pattern: `unsafe impl Send/Sync` on a type with raw pointers without checking all safety obligations. Typically you want `unsafe impl<T: Send> Send for MyContainer<T> {}` with a bound.
- Anti-pattern: raw-pointer walking without `wrapping_offset` when you might step out of the allocation — `offset` is UB if outside the allocation.
- Anti-pattern (const): `transmute` of provenance-carrying values to integers inside `const fn`. UB since const-eval tightened provenance.
- Anti-pattern: storing a `MutexGuard<T>` across an `.await`. Makes the future `!Send` (MutexGuard is !Send). Scope the guard: take the lock, operate, drop, then await.
- Anti-pattern: recursive async fn directly — produces an infinitely-sized state machine. Box it: `async fn foo() -> Box<impl Future<...>> { Box::pin(async { ... }) }` or use the `async-recursion` crate.

## [TAG: 02-language-rules] Ownership-Based Resource Management

- Rule: RAII is the default in Rust. Destructor runs at scope exit (or when moved into another owner). [Nomicon: obrm]
- Rule: Rust has exactly one way to construct a value of a user-defined type — name it and provide all fields. Every "constructor" (`::new`, `::from`, `::with_capacity`) is just a regular function that calls this. No constructors, no copy-constructors, no move-constructors. [Nomicon: constructors]
- Consequence: types are freely memcpy-movable anywhere. No intrusive linked lists without unsafe self-referential tricks (see Pin).
- Rule: `Clone` (explicit .clone()) and `Copy` (implicit bit-copy on use) are the Rust analogues of copy constructors. `Copy` types must be pod-like — no `Drop` impl.
- Rule: `Default` is a convenience, not a construction protocol. `T::default()` is just a function.

## [TAG: 02-language-rules] Traits: Object Safety, Auto Traits, Unsafe Traits

- Rule: Auto traits (`Send`, `Sync`, `Unpin`, `UnwindSafe`, `RefUnwindSafe`, `Freeze`) are automatically implemented if every field implements them. Opt-out via `PhantomData<!SomeTrait>` or `impl !Trait for T` (nightly for non-auto, stable for auto).
- Rule: `Freeze` (unstable but semantically meaningful) — type contains no `UnsafeCell`. Used for `const`-promotion rules.
- Rule: marker traits (`Send`, `Sync`, `Copy`, `Sized`, `Unpin`) have no methods and only carry type-system meaning.
- Rule: `Sized` is implicit on every generic parameter — `fn f<T>(x: T)` means `T: Sized`. Opt out with `T: ?Sized` to accept DSTs.
- Rule: `?Sized` parameter can only be used behind a pointer: `&T`, `Box<T>`, etc.

## [TAG: 02-language-rules] Const Eval & Const Generics

- Rule: `const fn` callable in const context. Body must be a subset of safe Rust: no heap allocation (yet), no trait calls (mostly — `const Trait` is unstable), no `&mut` across iterations of a loop (stable in newer versions with caveats), no generic trait dispatch.
- Rule: Const blocks: `const { expr }` forces `expr` to be evaluated at compile time. Useful for `[const { vec![] }; 1000]` — no runtime repetition.
- Rule: Const generics: `struct Arr<const N: usize> { data: [u8; N] }`. Parameters must be primitives (integers, `bool`, `char`, or `&'static str` unstable). Complex const generics (`const generics expressions`) gated behind `#![feature(generic_const_exprs)]`.
- Rule: In const context, UB is a hard compile error: out-of-bounds index, overflow, UB from intrinsics — all diagnose at compile time. At runtime, overflow in debug is panic, in release is two's complement wrap (unless `overflow_checks` is enabled).
- Rule: In const, pointer provenance is stricter — you cannot transmute a pointer with provenance to a non-pointer type.
- Rule: `'static` lifetime extension: `const C: &u8 = &0;` extends the temporary. Mutable-borrow lifetime extension is forbidden in const context.

## [TAG: 08-unsafe-and-ffi] Allocator API

- Rule: `std::alloc::{alloc, dealloc, realloc, alloc_zeroed}` operate on `*mut u8` given a `Layout`.
- Rule: `Layout::from_size_align(size, align)` — size must be ≤ `isize::MAX` rounded up to `align`. `Layout::array::<T>(n)` does this math for you, returning `Err` on overflow.
- Rule: `dealloc(ptr, layout)` — layout MUST match exactly the layout used in `alloc`.
- Rule: `alloc` returns null on OOM; call `handle_alloc_error(layout)` to abort (do NOT panic — unwinding can re-enter the allocator).
- Rule: `GlobalAlloc` trait — `unsafe trait`. Use `#[global_allocator]` static to override. Implementors must respect Layout invariants strictly.
- Rule: Allocator API v2 (`Allocator` trait, `Box::new_in`, `Vec::new_in`, ...) — unstable (as of 1.95). Use `allocator-api2` shim crate for stable use.

## [TAG: 04-design-patterns] Vec Internals (Structural Summary)

- Layout: `Vec<T> = { ptr: NonNull<T>, cap: usize, len: usize }`.
  - `NonNull<T>` provides covariance in T and enables the null-niche (`Option<Vec<T>>` is the same size).
  - `unsafe impl<T: Send> Send for Vec<T> {}` + corresponding Sync — since `NonNull` is `!Send/!Sync`, we re-establish auto-trait ownership semantics.
- Empty state: `Vec::new()` does NOT allocate. `ptr = NonNull::dangling()`, `cap = 0`, `len = 0`.
- Growth: double capacity; first growth from 0 → 1 or to some floor (std uses `max(cap*2, 4)` for small T, varies).
- Allocation guard: total size `cap * size_of::<T>() ≤ isize::MAX`. Panic/abort if violated.
- OOM: call `handle_alloc_error`, which aborts. Never panic.
- ZST handling: cap = `usize::MAX`, never allocate, never dealloc. Length tracks logical count. Pointer arithmetic is a no-op (handle by casting ptr to usize for iterator end).
- Destructor: pop all elements (or drop_in_place the initialized prefix), then dealloc the buffer if cap != 0 (and not ZST).
- RawVec abstraction: `RawVec<T> = { ptr, cap }` — allocation logic; shared between `Vec<T>`, `VecDeque<T>`, `IntoIter<T>`, `Drain<T>`.
- `IntoIter<T>`: holds RawVec (for dealloc) + start/end pointer pair. Drop: drop remaining elements, then RawVec dealloc happens.
- `Drain<'a, T>`: borrows &mut Vec, sets vec.len = 0 immediately (leak amplification), iterates via pointers, on drop restores any remaining elements or truncates. Forgetting a Drain leaks everything.
- `insert(i, x)`: bounds check `i <= len`, grow if needed, `ptr::copy(ptr+i, ptr+i+1, len-i)`, `ptr::write(ptr+i, x)`, `len += 1`.
- `remove(i)`: bounds check `i < len`, `len -= 1`, `ptr::read(ptr+i)`, `ptr::copy(ptr+i+1, ptr+i, len-i)`, return element.

## [TAG: 04-design-patterns] Arc Internals (Structural Summary)

- Layout: `Arc<T> = { ptr: NonNull<ArcInner<T>> }`; `ArcInner<T> = { strong: AtomicUsize, weak: AtomicUsize, data: T }`.
- Clone: `inner.strong.fetch_add(1, Ordering::Relaxed)`; abort if ≥ `isize::MAX` (guard against mem::forget bomb producing wraparound → use-after-free).
- Drop: `let n = inner.strong.fetch_sub(1, Release); if n != 1 return; fence(Acquire); unsafe { Box::from_raw(self.ptr.as_ptr()); }`.
- Ordering rationale: Relaxed on clone (no data synchronization needed; just ownership count). Release on final drop → pairs with Acquire fence to synchronize data writes before dealloc.
- Weak: separate count; upgrade attempts `compare_exchange` to bump strong from non-zero. If zero, data already dropped but allocation may still live until weak = 0.
- Send/Sync requirements: `Arc<T>: Send + Sync` iff `T: Send + Sync` (shared mutable access across threads requires both).

## [TAG: 02-language-rules] MIR Mental Model (for borrowck)

- Rule: rustc lowers Rust to MIR (Mid-level IR) — a CFG of basic blocks, each ending in a Terminator (branch / call / return / drop).
- Rule: Each local variable ("place") has a fixed slot. Moves invalidate the slot; drop flags track conditional drops.
- Rule: Borrowck runs on MIR (post-NLL). Region inference determines live ranges; conflicts are reported where incompatible borrows overlap.
- Rule: Two-phase borrows split `Fn(&mut T, ...)` into (reserve `&mut T`, compute args using `&T`, activate `&mut T` at call).
- Rule: Drop elaboration — MIR inserts explicit `Drop(local)` terminators where needed, respecting drop flags.
- Rule: Polonius (work-in-progress) replaces NLL with a Datalog-based analysis that accepts more programs (e.g. "conditional return of a mutable borrow"). Available experimentally via `-Zpolonius`.

## [TAG: 02-language-rules] Coherence in Detail

- Rule: "Covered" types — for an `impl T for A<B>` to satisfy the orphan rule, the first uncovered type parameter must be local. `&T`, `Box<T>`, etc. being fundamental means they're treated as "uncovered".
- Rule: Blanket impls (`impl<T> Trait1 for T where T: Trait2`) — `Trait1` must be local OR every concrete type substitution path passes through a local type.
- Rule: `impl<T> From<T> for T { ... }` — reflexive impl of `From`. Any type. Built into std; no downstream conflict.
- Rule: Default associated types (GATs, default `type Item = T`) don't affect coherence but do reduce explicit impls needed.

## [TAG: 02-language-rules] Type Inference Mental Model

- Rule: Rust uses a bidirectional Hindley-Milner-style inference: type information flows outward (from expressions) and inward (from annotations / declared types).
- Rule: `let x = ...;` without a type — inference proceeds from the RHS and uses sites. If ambiguous at use, error.
- Rule: `let x: T = ...;` constrains RHS to T.
- Rule: Method call resolution: `value.method(args)` tries:
  1. `T::method(value)`.
  2. `<&T>::method(value)` then `<&mut T>::method(value)`.
  3. If `T: Deref<Target = U>`, repeat with `U`.
  4. Unsizing: `[T; N]` → `[T]`, sized → unsized at the tail.
  First match wins. Trait bounds on `T` affect resolution order. [Nomicon: dot-operator]
- Rule: Trait bound inference — for `fn foo<T: Bar>()`, each call site must prove the concrete type fulfills `Bar`. Inference may try coercions.
- Rule: Closure capture inference is disjoint (2021+) — compiler picks the minimal set of fields to capture.
- Rule: Inferred type defaults when ambiguous: integer → `i32`, float → `f64`, `!` → `!` (2024) or `()` (pre-2024).
- Rule: `_` type holes: `let x: Vec<_> = something.collect();` — collect's generic return is constrained by the Vec, inner is inferred.
- Common inference failures:
  - "type annotations needed" — closure captures from unbounded generic, or `collect` result ambiguous.
  - "cannot infer type for type parameter `S` declared on the function `hash`" — default type parameter wasn't chosen because multiple options exist.
  - Fix: provide turbofish (`foo::<Type>(...)`) or annotate.

## [TAG: 02-language-rules] Trait Resolution Quirks

- Rule: trait resolution uses "candidates" — built-in, impl, where-clause. Ambiguity across candidates is a hard error.
- Rule: inherent method vs trait method — inherent wins in method resolution. Multiple trait methods of same name require explicit disambiguation (`<T as Trait>::method(&x)`).
- Rule: orphan implementation — cannot `impl External for External`; must own one side.
- Rule: specialization (nightly) — `default impl` allows overriding in descendant impls, but lifetime-dependent specialization is unsound and gated.

## [TAG: 12-modern-rust] Other Stable Features to Internalize (1.70–1.95)

- `std::sync::OnceLock<T>` — 1.70.
- `let else` — 1.65.
- GATs — 1.65.
- Scoped threads (`std::thread::scope`) — 1.63.
- `#[derive(Default)]` on enums via `#[default]` variant — 1.62.
- `let chains` in `if`/`while` — stable in Rust 2024 (edition-gated).
- `async fn` in traits + RPITIT — 1.75.
- `AFIT Send bound via RTN` — 1.85.
- `std::sync::Exclusive<T>` — 1.85 (Send wrapper).
- Strict provenance API — 1.84.
- `Ord::cmp_min`/`cmp_max`, `const` fn stabilizations.
- `ManuallyDrop::take` — stable.
- Raw reference operators (`&raw const`, `&raw mut`) — 1.82.
- `impl Trait` in `type` aliases (TAIT) — 1.74 partial / 1.85 full.
- `expect` on lints (`#[expect(lint)]`) — 1.81.
- `Read::read_buf` / `BufRead::has_data_left` / misc I/O — 1.80.
- `Box<[T; N]>` / `Rc<[T; N]>` / `Arc<[T; N]>::new_zeroed` / `new_uninit` family — stable.
- `LazyCell` / `LazyLock` — 1.80.
- `std::simd` portable SIMD — stable in 1.93/1.94 (std::simd module graduating; check current release notes).

## [TAG: 12-modern-rust] gen blocks and Iterator-from-generator

- Rule (RFC 3513, stabilized around Rust 1.89): `gen { yield x; ... }` produces an anonymous type implementing `Iterator`. `yield expr` produces the next item. Iteration terminates when the block returns.
- Rule: `gen fn` syntax — function returning an Iterator. Example:
  ```rust
  gen fn counter() -> i32 { let mut i = 0; loop { yield i; i += 1; } }
  ```
- Rule: inside a `gen` block, locals persist across `yield` (compiler builds a state machine analogous to async).
- Rule: `gen` blocks are `!Unpin` like futures, so iterating requires pinning. `std::pin::pin!` helper. Consumer APIs often wrap this.
- Rule: differences from async — gen is synchronous, no `.await`. No `Context`/`Waker`.
- Async gen (future): `async gen` for Stream — still gated on nightly.

## [TAG: 08-unsafe-and-ffi] Misc UB corner cases

- UB: dereferencing `&` or `&mut` to a dangling location, even if just to take another reference or project a field. The validity check is on the reference itself.
- UB: reading uninitialized `i32` memory — though any bit pattern is valid for i32, the `uninit` status propagates and is LLVM-level poison; read operations eventually materialize to UB.
- UB: `*mut T` to an allocation that has been freed (use-after-free) — even if just reading a field offset without dereferencing, if the offset crosses the original allocation bounds.
- UB: calling a function pointer whose signature differs from the real target. `transmute::<fn(i32), fn(u32)>` is legal at the type system, calling it is UB.
- UB: executing code with `#[target_feature(enable = "avx2")]` on a CPU without AVX2. Hence the `is_x86_feature_detected!` guard pattern.
- UB: inline assembly that violates calling convention, reads from uninitialized regs treated as inputs, writes to regs not marked as clobbered, etc.
- UB: double-init via `MaybeUninit::write` twice when T has a `Drop` impl — the first value is leaked (fine) but... actually `.write` just overwrites; no UB. HOWEVER `ptr::write` on a MaybeUninit that was already assume_init'd and whose T has Drop leaks the first and doesn't drop — OK. Just don't `assume_init` twice.
- UB: mutating through `*const T` when the original allocation is only accessible through `&T` chain. Provenance must permit writes.
- UB: going from `*mut T` to `&mut T` when there are other live references to the same place.
- Semi-UB (lint error, may become UB): bit-patterns outside the niche of `NonZero*`/`NonNull`/`&` etc. via transmute.

## [TAG: 08-unsafe-and-ffi] Validity vs Safety Invariants (UCG mental model)

- Rule (UCG terminology): a type has two invariants:
  - VALIDITY invariant — bit-level. Violated = UB. Checked at every production site. E.g. `bool` validity is 0 or 1.
  - SAFETY invariant — semantic, library-defined. Violated = logic bug but not necessarily UB. E.g. `NonZero::new_unchecked(0)` violates the safety invariant; reading through the result as a NonZero (which is niched) would then become UB.
- Rule: safe code must always uphold safety invariants. Unsafe code may *temporarily* break safety invariants (e.g. Vec has len > initialized during a push) as long as no safe code can observe the broken state. Safety invariants must be restored before any safe code sees the value (return, panic, borrow escape).
- Rule: a library type's safety invariant is the implementor's contract. A `String` must be valid UTF-8 (safety invariant) — breaking this doesn't instantly UB, but using it in anything expecting `&str` will UB.
- Rule: validity invariants are enforced uniformly by the language; safety invariants are documented per-type in `# Safety` doc comments.

## [TAG: 08-unsafe-and-ffi] "Frozen" Memory (UCG concept)

- Rule: bytes reached only through shared references (not UnsafeCell) are "frozen" — the compiler assumes they never change for the reference's lifetime. The `Freeze` auto trait captures types without `UnsafeCell` inside.
- Rule: this is why `Cell`/`RefCell` must be behind `UnsafeCell`: they need to mutate through shared refs, so they opt out of freezing.
- Rule: Rust's `const`-promotion and static-scheduling depend on Freeze — only Freeze-values are promoted to `static`.

## [TAG: 08-unsafe-and-ffi] Borrow Splitting Tricks

- Pattern: `slice::split_at_mut(mid) -> (&mut [T], &mut [T])` — canonical unsafe split. Internally uses raw pointers so the borrow checker cannot see the overlap proof.
- Pattern (iter_mut for singly-linked list):
  ```rust
  // Invariant: &mut Option<Box<Node<T>>>
  fn next(&mut self) -> Option<&mut T> {
      self.0.take().map(|node| {
          self.0 = node.next.as_mut().map(|n| &mut **n);
          &mut node.elem
      })
  }
  ```
  Key: take the current head (Option::take), yield reference, chain into the next — no overlapping lifetimes.
- Pattern (slice iter_mut):
  ```rust
  let slice = std::mem::take(&mut self.slice);   // Replace with &mut []
  let (l, r) = slice.split_at_mut(1);
  self.slice = r;
  l.get_mut(0)
  ```
- Rule: you cannot split a struct's fields via indexing alone (the borrow checker doesn't track disjoint array indices), but struct field projections `(&mut s.a, &mut s.b)` DO work for disjoint fields.

## [TAG: 08-unsafe-and-ffi] Higher-Level Invariants You Must Preserve

- Vec: `ptr` is allocated for `cap * size_of::<T>()` bytes (or dangling for cap=0/ZST). `len <= cap`. First `len` elements are initialized T; the rest uninit.
- String: inner `Vec<u8>` is valid UTF-8.
- Box<T>: non-null, aligned, points to valid T in a heap allocation owned exclusively by this Box. Drop deallocates.
- Rc<T>/Arc<T>: non-null pointer to ArcInner-shaped header; strong and weak counts consistent across all copies.
- Mutex<T>: underlying pthread/SRW-lock primitive is valid. Poison flag tracks whether a panic was detected.
- UnsafeCell<T>: no additional invariant beyond containing a T.
- Pin<P>: pointee will not move if !Unpin. Drop must run before memory reuse.
- Future: after `Poll::Pending`, the future's state is self-referential (potentially); must not be moved until completed.

## [TAG: 12-modern-rust] Documentation & Language Stability Meta

- Rule: the Rust Reference is normative where it applies; UCG is where the working group hashes out semantics that the Reference doesn't yet cover. Nomicon is authoritative for unsafe practice but explicit about being incomplete.
- Rule: assume Miri is the operational semantics for your unsafe code. If `cargo +nightly miri test` passes with `-Zmiri-strict-provenance`, you're in very good shape.
- Rule: RFC status ≠ stabilization. Many RFCs are accepted but unimplemented or gated for years (impl specialization, extern types, pattern types).
- Rule: newer editions (2018 → 2021 → 2024) don't break old crates — per-crate edition field in Cargo.toml. Macros use the edition of their definition crate. Edition changes are sugar / lint / defaults.

## [TAG: 08-unsafe-and-ffi] Checklists

### Writing an unsafe fn
1. Document `# Safety` section listing preconditions.
2. Mark fn `unsafe`.
3. Inside, wrap unsafe ops in `unsafe { ... }` blocks with `// SAFETY:` comments (Rust 2024 required).
4. Run under Miri.
5. Ensure every public safe API that calls the unsafe fn fulfills its preconditions.

### Writing an unsafe trait
1. `unsafe trait Name { ... }`.
2. Document the contract for implementors.
3. All `impl` must be `unsafe impl`.
4. Do not expose safety-critical methods publicly without confirming implementors must ensure correctness.

### Designing a raw-pointer-backed type
1. Use `NonNull<T>` (covariant, non-null niche).
2. Add `PhantomData<T>` if you logically own T (for auto-traits and dropck).
3. Use `#[repr(transparent)]` for newtypes around `NonNull`.
4. Manually implement `Send`/`Sync` with appropriate bounds: `unsafe impl<T: Send> Send for MyPtr<T> {}`.
5. Consider variance: `*mut T` or `Cell<T>` in field makes invariant in T.
6. Implement `Drop` with `drop_in_place` before dealloc.
7. Check for ZST. Special-case allocator calls.
8. Check for isize::MAX capacity overflow.
9. Hand ownership via `into_raw` / `from_raw` pairs; document lifetimes carefully.

### Reviewing unsafe code
- Every unsafe block has a SAFETY comment justifying it.
- No `&T -> &mut T` via transmute.
- No `transmute` between `repr(Rust)` types.
- No `mem::uninitialized` / `mem::zeroed` — MaybeUninit instead.
- No pointer offsets past end-of-allocation via `offset` (use `wrapping_offset`).
- No creating references to packed fields directly.
- Alignment + provenance respected for all pointer reads.
- Drop order analyzed for panic paths.
- Send/Sync impls have appropriate trait bounds.
- Doesn't rely on destructors running for safety.

### Reviewing FFI
- `unsafe extern "C"` (2024).
- Every imported function has `# Safety` or explicitly marked unsafe at call site.
- All types are FFI-safe (primitives, repr(C), transparent, pointers). No `String`, `&str`, `Vec`, etc. at the boundary.
- Panics caught before the boundary (unless `"C-unwind"` and receiver is C++).
- Opaque struct pattern used for foreign handles.
- `#[link(...)]` attribute matches system library conventions.
- Platform-specific ABI via `cfg_attr`.

### Reviewing async code
- No MutexGuard across `.await`.
- No blocking syscalls inside async fn (use async equivalents or spawn-blocking).
- Large state machines not held by value — box large futures.
- Recursive async functions boxed (`async-recursion` or manual).
- Cancel-safety considered (what happens if a `select!` arm drops this future mid-execution).
- Send/Sync bounds verified for spawning onto multi-thread executors.

## [TAG: 12-modern-rust] Tool & Crate Ecosystem Anchors (helpful in unsafe review)

- Miri: interpreter with UB detection. `cargo +nightly miri test`.
- cargo-careful: extra runtime checks.
- Loom: permutation-based concurrency tester for lock-free code.
- Shuttle: randomized concurrency testing.
- `parking_lot`: faster `Mutex`/`RwLock` with no poisoning.
- `crossbeam`: epoch-based reclamation, channels, deque.
- `pin-project-lite` / `pin-project`: safe Pin projections.
- `bytemuck`: safe transmute for POD-like types (validates via trait bounds).
- `zerocopy`: parse/serialize via repr(C) validation.
- `ouroboros`: safe self-referential struct builder.
- `async-trait`: type-erased async traits via Box.
- `trait-variant`: generate Send-bounded async trait variants.
- `allocator-api2`: stable shim for unstable Allocator trait.

## [TAG: 02-language-rules] Quick Reference Tables

### Aliasing
| Reference form | Reads allowed | Writes allowed | Can coexist with |
|---|---|---|---|
| `&T` | yes | only via UnsafeCell | other `&T` |
| `&mut T` | yes | yes | nothing |
| `*const T` | yes (when aliasing rules preserved) | no (under Stacked Borrows discipline) | anything (but access must respect live references) |
| `*mut T` | yes | yes | anything |
| `Box<T>` | yes (exclusive) | yes | nothing aliasing the owned allocation |

### Niche Guarantees
| Type | Forbidden bit pattern used as niche |
|---|---|
| `&T`, `&mut T`, `Box<T>` | null |
| `NonNull<T>` | null |
| `NonZeroU8..128`, `NonZeroI*` | zero |
| `bool` | 2..=255 |
| `char` | > 0x10FFFF and surrogates |
| Fn pointers | null |
| `enum` (with `repr(Rust)`) | reserved discriminant bits |

### Common `!Send`/`!Sync` offenders
| Type | Send | Sync |
|---|---|---|
| `Rc<T>` | ✗ | ✗ |
| `RefCell<T>` | ✓ if T: Send | ✗ |
| `Cell<T>` | ✓ if T: Send | ✗ |
| `UnsafeCell<T>` | ✓ if T: Send | ✗ |
| `*const T`, `*mut T` | ✗ | ✗ |
| `NonNull<T>` | ✗ | ✗ |
| `MutexGuard<'_, T>` | ✗ | ✓ if T: Sync |
| `Arc<T>` | ✓ if T: Send+Sync | ✓ if T: Send+Sync |

### Ordering cheat sheet (bold = pairs for synchronization)
- **Release store ⇄ Acquire load** (same atomic, same memory) — establishes happens-before between pre-store and post-load.
- **Release fence ⇄ Acquire fence** (same logical sync) — synchronizes ALL prior / subsequent ops across the fences.
- SeqCst — total order across all SeqCst ops program-wide. Use when you need Dekker/Lamport-style mutual exclusion without hardware fences on weak memory models.
- Relaxed — atomicity only. Counters, flags.

## [TAG: 02-language-rules] Rust 1.75+ Feature Summary (abbreviated)

- 1.75: async fn in trait, RPITIT. `Option::as_slice` / `as_mut_slice`.
- 1.76: ABI compat docs. `Arc::unwrap_or_clone`.
- 1.77: `offset_of!` macro for struct field offsets. C-string literals.
- 1.78: `#[diagnostic::on_unimplemented]`. Deterministic `HashMap` iteration for tests.
- 1.79: Inline const expressions. Bounds on associated types. `{Option,Result}::(as_ref, as_mut, as_deref, ...)` expansion.
- 1.80: `LazyCell` / `LazyLock`. `Duration::div_duration_f64`. Checked range literals in patterns.
- 1.81: `#[expect(lint)]`. `core::error::Error`. `Error::source` stable in core.
- 1.82: `&raw const` / `&raw mut` operators. `IoSlice::advance`. `char::from_u32_unchecked` const.
- 1.83: `const` references to statics. `Mutex::clear_poison`. `Waker::data/vtable`.
- 1.84: Strict provenance APIs stable. `cargo generate-lockfile --minimum-rust-version`.
- 1.85: Rust 2024 edition. Async closures. `FromIterator<T>` for `Box<[T]>`. `trait Trait { fn foo() impl Future<Output=i32> + Send { } }` supported.
- 1.86: Trait upcasting coercions. `HashMap::get_many_mut`.
- 1.87: Stabilize `asm_goto`, `naked_asm!`, additional const fn.
- 1.88: Let chains in 2024 (`if let ... && let ...`). `i128`/`u128` in extern "C" on more platforms.
- 1.89: `gen` blocks and `gen fn` synchronous generators.
- 1.90+ (through 1.95): incremental type-inference improvements, more `const fn`, additional atomic ops (`AtomicUsize::fetch_update` ergonomics), async drop (edition-dependent experimental), inline const drop, improved cycle detection diagnostics, stabilized `Try` trait variants where applicable.

## [TAG: 02-language-rules] Destructor Scope Details (Reference: destructors)

- Rule: Drop scopes: the function, each statement, each expression, each block, each match arm.
- Rule: Scope nesting — inner to outer:
  - Function body block → entire function
  - Expression statement → statement scope
  - `let` initializer → `let` statement scope
  - Match guard → match arm → match expression
- Rule (function parameters): all parameters dropped at function end, in reverse declaration order, AFTER local bindings.
- Rule (pattern bindings): multiple bindings in one pattern are dropped in reverse order of declaration within the pattern.
- Rule (or-patterns): drop order follows the FIRST subpattern's declaration order regardless of which arm matched. So `(Ok([x,y]) | Err([y,x]))` always drops x before y.
- Rule (temporary scope — 2024 changes):
  - `if let` scrutinee temporaries dropped before the `else` block starts (2024 change — pre-2024 held across both arms).
  - Tail expression temporaries in a block dropped immediately after the block's value is produced (2024 change).
- Rule (temporary lifetime extension): temporaries extend to the outer `let` when:
  - The `let` pattern is identifier with `ref`/`ref mut`, or struct/tuple/slice/or-pattern containing one.
  - The initializer expression, or operand of extending borrow, or argument to certain constructors (extending cast/struct/tuple/enum).
- Not extending:
  - Function call arguments: `f(&temp())` — temp dies at end of statement.
  - Method receivers: `(&temp()).method()`.
  - Match scrutinees: `match &temp() { ... }`.
  - Closure bodies, async block bodies, loop break values.
- Pattern: `let x = &temp();` extends; `let x = f(&temp());` does not.
- Rule (const promotion): `const C: &T = &expr` promotes `expr` to `'static` if it has no interior mutability, no destructors, no side effects. Useful for `const FOO: &[u32] = &[1,2,3];`.

## [TAG: 08-unsafe-and-ffi] Unions in Detail

- Rule: Union fields must be one of:
  - A `Copy` type.
  - A reference `&T`, `&mut T` for arbitrary T.
  - `ManuallyDrop<T>` for arbitrary T (opts out of auto-drop).
  - A tuple or array of the above.
- Rule: Unions never auto-drop their fields; the union type has no Drop implementation automatically. Implement Drop yourself if needed, read the active variant by other means (tag), and call `ManuallyDrop::drop` on the field.
- Rule: Reading a field: UNSAFE. You must uphold the validity invariant of the field's type — e.g. reading `u.f1: bool` when the underlying bytes are 0x02 is UB.
- Rule: Writing a field: SAFE. Overwrites memory with valid bits of that type. No drop runs (none of the fields have drop).
- Rule: Pattern matching on a union is unsafe and requires `unsafe` block. Exactly one field per arm.
- Rule: Borrowing a field borrows all fields. `&mut u.f1` blocks `&mut u.f2` and `&u.f3`. Read-only access to one still blocks other writes to the union.
- Rule: `repr(C) union` guarantees all fields start at offset 0 and size = max field size. Equivalent to C union.
- Rule: `#[repr(transparent) union]` requires exactly one non-ZST field (nightly/unstable as of 1.95 — `transparent_unions`).
- Rule: Unions are the legitimate vehicle for "type punning" (bit-level reinterpretation). Transmute is equivalent but more restrictive (size must be equal).
- Pattern (tagged union in FFI):
  ```rust
  #[repr(C)]
  pub struct TaggedUnion { tag: u8, data: TaggedUnionData }
  #[repr(C)]
  pub union TaggedUnionData { i: i32, f: f32 }
  ```
- Rule: `MaybeUninit<T>` is a union internally: `union MaybeUninit<T> { uninit: (), value: ManuallyDrop<T> }`. This is why it has no validity constraints.

## [TAG: 02-language-rules] Trait Object Deeper Rules

- Rule: `dyn Trait` can bind at most ONE principal (non-auto) trait and any number of auto traits (`Send`, `Sync`, `Unpin`) and at most ONE lifetime.
- Rule: `dyn Trait1 + Trait2` is forbidden when both are non-auto. Workaround: define `trait Combined: Trait1 + Trait2 {}` and use `dyn Combined`.
- Rule: Auto trait order doesn't matter: `dyn Trait + Send + Sync` ≡ `dyn Trait + Sync + Send`.
- Rule: Default lifetime of a trait object:
  - Bare `Box<dyn Trait>` / `Rc<dyn Trait>` / `Arc<dyn Trait>` → `'static`.
  - `&'a dyn Trait` → `'a`.
  - In struct: follows elision rules; often must be explicit.
- Rule: Trait object coercions:
  - `&T` → `&dyn Trait` if `T: Trait` and trait is dyn-compatible.
  - `Box<T>` → `Box<dyn Trait>`.
  - Upcast `&dyn Sub` → `&dyn Super` stable since 1.86.
  - Pointer cast `*const (dyn Foo + Send)` → `*const dyn Foo` drops the auto trait (OK).
- Rule: Pointer cast between trait objects: principal must be the same. Auto traits may be dropped; may only be added if supertraits guarantee it. Lifetime may only shorten. Generic args must match.
- Gotcha: `dyn Trait` without lifetime in a struct field defaults to `'static`, which is often too strict. Use `dyn Trait + 'a` and add `'a` to the struct.
- Gotcha: `Box<dyn Error>` is `Send + 'static` implicit? NO — it's just `dyn Error`. For thread-safe error type, use `Box<dyn Error + Send + Sync>`.

## [TAG: 02-language-rules] Cast (as) Complete Rules

- Numeric:
  - Same-size integer cast: bit-preserving. `-1i8 as u8 == 255`.
  - Larger → smaller integer: truncation (drop high bits).
  - Smaller → larger integer: zero-extend for unsigned, sign-extend for signed.
  - Float → integer: truncate toward zero; NaN → 0; saturate to min/max on overflow (stable 1.45+).
  - Integer → float: closest representable with round-ties-to-even.
  - f32 → f64: lossless.
  - f64 → f32: round-ties-to-even; saturate infinities/overflow.
- Bool/char:
  - `bool as u8` → 0 or 1.
  - `char as u32` → code point.
  - `u8 as char` → ASCII/Latin-1 code point (only `u8` → `char` allowed).
  - Other integer → char: NOT allowed (only u8).
- Enum:
  - Only field-less enums (or unit-variants only) can cast to integer — yields discriminant.
  - Forbidden if the enum `impl Drop`.
  - Reverse (integer → enum) NOT allowed directly; must go via transmute + validity check (or enum-repr + `TryFrom`).
- Pointers:
  - Pointer → integer: machine address. Under strict provenance, prefer `addr()`.
  - Integer → pointer: forged address; dereferencing is UB unless provenance is obtained (use `with_exposed_provenance` or have a valid round-trip).
  - Pointer → pointer (sized ⇄ sized): bitwise identity. `*const T as *const U` does NOT reinterpret.
  - Pointer to unsized → pointer to sized: DROPS metadata.
  - Pointer to unsized → pointer to unsized: only when metadata is compatible (see trait object rules above, and `[T] <-> [U]` length preserved).
  - Raw slice length is NOT scaled: `*const [u16; 4] as *const [u8]` → length still 4, not 8. Gotcha.
- Function pointers:
  - Function item → fn pointer → raw pointer / integer.
  - Zero-capture closure → fn pointer.
  - Capturing closures can NEVER be cast to fn pointers.
- Rule: `as` can perform coercions PLUS these unsafe-adjacent casts (int↔ptr, enum→int, etc.). Coercions alone are safer and preferred.

## [TAG: 02-language-rules] Coercions Enumerated

- Reborrowing: `&mut T` → `&T` (shorter lifetime allowed), `&mut T` → `&mut T` (reborrow with shorter lifetime).
- Deref: `&T` → `&U` if `T: Deref<Target = U>`. Chained.
- Box/Rc/Arc deref coercion.
- Unsized coercions:
  - `&[T; N]` → `&[T]`.
  - `&T` where `T: Trait` → `&dyn Trait` (if dyn-compatible).
  - `Box<T>` → `Box<dyn Trait>`.
  - `&Struct<[T; N]>` → `&Struct<[T]>` for structs ending in DST.
- Pointer: `&T` → `*const T`; `&mut T` → `*mut T`; `*mut T` → `*const T`.
- Function item (ZST distinct type per function) → `fn` pointer (has a size).
- Never type: `!` coerces to any type (function returning `!` can be used in any expression position).
- Closure → `fn` pointer (only if no capture).
- Subtype coercion (lifetime): any `&'long T` → `&'short T` (covariance).
- Coercions do NOT apply for trait resolution. `&mut T` coerces to `&T`, but an `impl Trait for &T` does NOT apply to `&mut T` automatically.

## [TAG: 04-design-patterns] Builder, Typestate, Sealed Enum Patterns (+ interactions with unsafe)

- Builder: typical builder pattern (Rust doesn't use default args). Can be combined with typestate — each setter transitions to a new type.
- Typestate: encode the state machine in the type: `Connection<Disconnected>` → `.connect() -> Connection<Connected>` → `.send(...)` available only in `Connected`.
- Sealed enum: combine with `#[non_exhaustive]` attribute to prevent downstream match exhaustiveness from breaking on variant addition.
- Newtype + `repr(transparent)`: zero-cost wrapping with distinct type. Keep source `NonZero` trait/invariants.
- Never-typed branches: `unreachable!()` macro at dead paths; compiler sees `!` and can remove.

## [TAG: 02-language-rules] Attributes Affecting Unsafe / ABI

- `#[inline]`, `#[inline(always)]`, `#[inline(never)]` — advise compiler; no safety impact.
- `#[cold]` — marks a function as rarely executed; affects optimization.
- `#[track_caller]` — makes `Location::caller()` return the caller's location; used by panic machinery.
- `#[must_use]` — warn if result is ignored; no safety.
- `#[repr(...)]` — layout control (see layout section).
- `#[unsafe(no_mangle)]` (2024) — export symbol under Rust ABI at the given name. Unsafe because names can collide with system libs.
- `#[unsafe(export_name = "foo")]` — explicit symbol name.
- `#[unsafe(link_section = ".name")]` — place in specific linker section. Unsafe because placement can break memory layout / security assumptions.
- `#[unsafe(link(name = "foo"))]` (2024) — link directive. Unsafe because linker can silently merge conflicting bindings.
- `#[target_feature(enable = "...")]` — enable CPU features for a function. Callers must ensure feature availability; calling without is UB.
- `#[naked]` — no prologue/epilogue (stable-ish for `core::arch::naked_asm!`). ABI-sensitive; must write manually.
- `#[panic_handler]` — define the panic handler in `no_std` crates. Must be exactly one per final binary.
- `#[global_allocator]` — override the default allocator.
- `#[alloc_error_handler]` — handle allocation failure in no_std.
- `#[non_exhaustive]` — prevent downstream matches from being exhaustive; allow future variant additions.
- `#[diagnostic::on_unimplemented]` — customize error message when trait bound is not met.

## [TAG: 08-unsafe-and-ffi] Reference/Pointer Validity Summary

- `&T` / `&mut T`: aligned, non-null, dereferenceable, pointee must be a valid T, aliasing respected for its lifetime.
- `Box<T>`: unique owner. Same rules as `&mut T` for aliasing during drop.
- `*const T`, `*mut T`: no liveness, no aliasing. But using them for writes/reads must respect the aliasing rules of whatever live references exist.
- `NonNull<T>`: non-null always. May dangle as long as not dereferenced.
- Wide pointers (`&[T]`, `&dyn Trait`, `&str`, `Box<[T]>`, etc.): metadata part must be valid:
  - Slice length: `size_of::<T>() * len <= isize::MAX`.
  - Trait object: pointer to a valid vtable matching the trait (exact vtable, not "a vtable for a subtrait").

## [TAG: 08-unsafe-and-ffi] C/C++ Interop Specifics

- `extern "C-unwind"` (stable 1.71) bridges Rust panics and C++ exceptions through ABI. Pair with compiler flag `-C panic=unwind`.
- Passing closures to C: closure must be non-capturing (use explicit `extern "C" fn`) OR you pass `(fn_ptr, user_data)` manually.
- C++ name mangling: use `#[unsafe(export_name = "...")]` to match C++ mangled names if calling C++ directly (or use `extern "C"` wrappers on the C++ side).
- Strings:
  - `CStr::from_ptr(ptr)` — must be NUL-terminated, valid up to NUL.
  - `CString::new(bytes)` — error if bytes contain NUL.
  - `OsStr` / `OsString` — platform-native string (not necessarily UTF-8 on Windows).
  - `Path` / `PathBuf` — filesystem paths; wrap OsStr.
- Integer types: `c_int`, `c_uint`, `c_char` (signed-ness varies per platform), `c_void` in `std::ffi` / `core::ffi` (stable). Don't assume `c_int == i32`.
- Bindgen flow: `bindgen` auto-generates Rust bindings; output uses `extern "C"`, `repr(C)`, `*mut c_char`, etc. Always review for function pointer pairs with user-data.
- Cbindgen flow: generate C/C++ headers from Rust source. Keep ABI stable by using `repr(C)` or `repr(transparent)` for all exposed types.

## [TAG: 09-performance] Performance Gotchas with Unsafe

- `Vec::with_capacity(n)` > `Vec::new()` + many pushes.
- `Box::new([0; N])` for large N stack-overflows; use `vec![0; N].into_boxed_slice()` or `Box::new_zeroed_slice(N).assume_init()` (unsafe).
- `ManuallyDrop` avoids cost of the drop-flag in wrappers.
- `MaybeUninit::uninit_array` avoids the compiler zeroing your array first.
- `slice::copy_from_slice` is memcpy; prefer over `for` loops.
- `Vec::extend_from_slice` on `T: Copy` is specialized to memcpy.
- `str::chars().count()` is O(n) UTF-8 decode; `str::len()` is byte count. Don't confuse.
- `HashMap` hasher: default is `RandomState` (DoS-resistant but slow). Use `ahash`/`foldhash` for internal maps.
- Branch prediction: `std::hint::likely` / `unlikely` (nightly). On stable, use `std::hint::black_box` for benchmarking.
- `#[cold]` + `#[inline(never)]` on panic paths keeps the happy path cache-warm.
- Cache-line padding: `#[repr(align(64))]` on fields shared between threads to prevent false sharing.
- `Vec::shrink_to_fit` relocates to tight allocation; may save memory or may cost an allocation.

## [TAG: 12-modern-rust] Scoped Threads

- Rule (stable 1.63): `std::thread::scope(|s| { s.spawn(|| ...); ... })` — closures in the scope can borrow non-`'static` data from the enclosing scope.
- Rule: Replaces the unsound `thread::scoped::JoinGuard`. The `scope` call blocks until all spawned threads complete.
- Pattern:
  ```rust
  let data = vec![1, 2, 3];
  thread::scope(|s| {
      s.spawn(|| println!("{:?}", &data));
      s.spawn(|| println!("{:?}", &data));
  });
  ```
- Rule: spawned threads have type `thread::ScopedJoinHandle<'scope, T>`. They auto-join at scope exit; no explicit `join()` required for correctness.

## [TAG: 12-modern-rust] Async Closures (Rust 1.85+)

- Rule: `async || { ... }` — closure returning a Future.
- Rule: `AsyncFn`, `AsyncFnMut`, `AsyncFnOnce` traits (1.85). Similar to `Fn/FnMut/FnOnce` but the call produces a Future.
- Rule: AsyncFn* traits are NOT dyn-compatible.
- Usage: passing async callbacks where previously you'd pass `impl Fn() -> impl Future<Output=T>` with awkward `for<'a>` bounds.
- Gotcha: async closures that capture by reference have lifetime implications; often need explicit bounds.

## [TAG: 08-unsafe-and-ffi] Niche Optimization Mechanics

- Rule: Compiler finds an unused bit pattern in a field type and uses it to represent other variants of the enclosing enum. E.g. `Option<&T>`: None uses null; Some uses any non-null.
- Rule: Multi-level niches: `Option<Option<bool>>` uses values 2 and 3 for the two None levels, so size = 1.
- Rule: Niche is a compiler choice, NOT guaranteed in `repr(Rust)`. The ONLY guaranteed niches are:
  - `&T`, `&mut T`, `Box<T>`, `NonNull<T>` inside an `Option<...>` is size-equivalent to the wrapped type.
  - `Option<fn(...) -> ...>` same as the fn pointer.
  - `Option<NonZeroU*>` same as the NonZero.
  - `Option<char>` same as `char` (4 bytes).
- Rule: `#[repr(u8)]` on an enum with fields DISABLES niche optimization.
- Rule: User-defined `NonZero*`-style niche requires `#[rustc_layout_scalar_valid_range_start]`/`_end` (internal).
- Rule: `core::num::NonZero<T>` (generic, stable 1.79) — `Option<NonZero<T>>` same size as `T`.
- Gotcha: Storing a `NonZero*` behind `UnsafeCell` cancels the niche optimization in outer layers — `UnsafeCell` forces pessimistic layout.

## [TAG: 12-modern-rust] const Features Evolution

- `const fn` expansion per release:
  - Basic arithmetic, panic!, assert! — early.
  - `if`, loops, match — 1.46.
  - Destructuring in const — 1.59.
  - Raw pointer cast, Ord/Eq trait methods — 1.61+.
  - `const_trait_impl` (generic const bounds) — still unstable.
  - Many intrinsics stable (e.g. `size_of`, `align_of`).
- `const _: () = assert!(...)` — compile-time asserts.
- Inline `const { }` block in non-generic position — stable 1.79.
- `const generics` for primitives stable; custom types remain gated.

## [TAG: 02-language-rules] The `Freeze` Auto Trait (Stable Semantics)

- Rule: `Freeze` is an auto trait (unstable in Rust public surface but present internally). A type is Freeze if it contains no UnsafeCell.
- Rule: Compiler uses Freeze to decide:
  - Whether a value can be placed in a read-only static.
  - Whether a `&T` borrow transitively freezes the bytes.
  - Const promotion: only Freeze values can be promoted to `'static`.
- Rule (user-visible implication): inside `const` / `static`, `UnsafeCell` forbids most operations that otherwise would be allowed.

## [TAG: 08-unsafe-and-ffi] Unsafe & Tests

- Miri: compile & run tests under `cargo +nightly miri test`. Requires `rustup component add miri` on nightly toolchain.
- Miri flags:
  - `-Zmiri-strict-provenance`: reject integer-to-pointer casts.
  - `-Zmiri-symbolic-alignment-check`: aggressive alignment checking.
  - `-Zmiri-track-raw-pointers`: track each raw pointer provenance.
  - `-Zmiri-tag-gc-interval`: tune Stacked Borrows GC.
  - `-Zmiri-many-seeds`: run many random seeds to catch non-determinism.
  - `-Zmiri-permissive-provenance`: allow ptr↔int roundtrips (default mode).
- Rule: failing Miri often means real UB; fix before proceeding.
- Rule: Miri is an interpreter, slow (10-100× slower than native). Use for unit tests of unsafe code specifically.
- Loom: `loom::model(|| { ... })` explores permutations of thread schedules. Good for lock-free algorithms.
- Kani: bounded model-checker for unsafe/contract verification (semi-stable).
- Prusti, Creusot: deductive verifiers — require annotations.

## [TAG: 08-unsafe-and-ffi] `split_at_mut` Rewritten Safely (Since 1.72)

- Stable `slice::split_at_mut` signature uses normal safe Rust:
  ```rust
  pub fn split_at_mut(&mut self, mid: usize) -> (&mut [T], &mut [T]) {
      assert!(mid <= self.len());
      unsafe { self.split_at_mut_unchecked(mid) }
  }
  ```
- Internally still uses unsafe pointer arithmetic; safe wrapper exists.
- Pattern: expose safe wrappers that internally do the unsafe manipulation with asserts on entry.

## [TAG: 08-unsafe-and-ffi] Shared Statics, Mutable Statics, Thread Locals

- `static FOO: T = ...;` — truly global, immutable, `'static`. Must be `Sync`.
- `static mut FOO: T = ...;` — mutable global. Reading AND writing are `unsafe`. Racy with any concurrent access — prefer `Mutex<T>` in a `static` or `AtomicXxx`.
- Rule (Rust 2024): `&raw const FOO` / `&raw mut FOO` are preferred over `&FOO` / `&mut FOO` for `static mut`, because the reference form can race with concurrent access. Also, direct field access `FOO.field` is being deprecated for `static mut` in favor of raw pointers.
- `thread_local!(static FOO: RefCell<T> = RefCell::new(...));` — per-thread storage. Always requires interior mutability.
- `const FOO: T = ...;` — inlined at each use site; does NOT have a single address. Cannot take `&`-reference that's the "same" pointer each time (the compiler may inline differently). Use `static` when address matters.
- Pattern: lazy init of static via `LazyLock::new(|| ...)` (1.80+). Replaces `lazy_static!` macro.

## [TAG: 02-language-rules] Edge Cases and Gotchas

- `&'static str` is NOT a general string type — it's a reference to static memory. `String::leak()` can produce one at runtime.
- `Box<str>` / `Box<[T]>`: owned DSTs. Single allocation; size = wide pointer.
- `Rc<[T]>::from(&[1, 2, 3])` (1.37+) creates a compact Rc with payload inline.
- `impl Trait` in argument position (APIT): generates an anonymous type parameter. Two `impl Trait` args become two DIFFERENT type params. `f(impl Trait, impl Trait)` ≠ `fn f<T: Trait>(T, T)`.
- `impl Trait` in return position captures in-scope lifetimes (2024+).
- `async fn` captures ALL in-scope lifetimes — this is intentional, but generates more restrictive Send bounds.
- `for<'a> Fn(&'a T)` is a higher-ranked bound, needed for closures that work across calls.
- Default type parameters: `struct Foo<T = i32>` — only used in inference when there's no constraint. In generics on fns, defaults rarely apply.
- `dyn Trait` autotrait leakage: `dyn Trait` inherits `Send`/`Sync` from what it was constructed from? NO — `dyn Trait` is a type; you choose `dyn Trait + Send` at cast site. Forgetting the bound downgrades auto traits.
- `Box<dyn FnOnce()>`: calling consumes the Box via `call_once`. Stable via `FnOnce` impl on `Box<F>`.

## [TAG: 08-unsafe-and-ffi] Examples with Explanations

### Vec::push, sound variant
```rust
fn push(&mut self, val: T) {
    if self.len == self.cap { self.grow(); }
    // SAFETY: len < cap guaranteed by grow(), allocation large enough.
    unsafe { ptr::write(self.ptr().add(self.len), val); }
    self.len += 1;
}
```
Key invariants: cap is the allocated capacity, len <= cap, first len elements initialized.

### Correct Send/Sync for custom smart pointer
```rust
pub struct MyBox<T> { ptr: NonNull<T>, _phantom: PhantomData<T> }
unsafe impl<T: Send> Send for MyBox<T> {}
unsafe impl<T: Sync> Sync for MyBox<T> {}
```

### Safe API that forbids misuse
```rust
pub fn to_ref(&self) -> &T { /* ... */ }     // safe
unsafe fn from_raw(ptr: *mut T) -> Self {      // unsafe — caller responsibility
    // SAFETY: caller guarantees ptr is exclusive and valid
    ...
}
```

### Drop guard (exception safety)
```rust
let _g = scopeguard::defer(|| restore_state());
// do stuff that may panic
// if panic: _g's Drop runs during unwinding
```

### Interior mutability in Rc (single-threaded)
```rust
let data: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(vec![]));
data.borrow_mut().push(42);  // runtime checked
```

### Mutex with conditional logic (don't hold across .await)
```rust
let guard = m.lock().unwrap();
let x = *guard;          // copy out
drop(guard);             // release before await
let y = compute(x).await;
```

### Pin-projecting manually
```rust
impl MyFuture {
    fn project<'a>(self: Pin<&'a mut Self>) -> (&'a mut u32, Pin<&'a mut Inner>) {
        // SAFETY: structural pinning: Inner must stay pinned; u32 can move.
        unsafe {
            let this = self.get_unchecked_mut();
            (&mut this.unpinned, Pin::new_unchecked(&mut this.pinned))
        }
    }
}
```

### Atomic spinlock
```rust
let flag = AtomicBool::new(false);
while flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
    std::hint::spin_loop();
}
// critical section
flag.store(false, Ordering::Release);
```

## [TAG: 12-modern-rust] Accepted RFCs — Dense Reference

### RFC 2094 — Non-Lexical Lifetimes
- Three motivating problems solved:
  1. Mutable borrow ending before next statement (previously extended to end of block).
  2. Borrows that span match arms but are only used in one.
  3. Self-referential method chains (`*map.entry(k).or_insert_with(default)`).
- New model: borrow lifetime = "set of points in CFG where borrow is live" (MIR-based).
- Consequence: `if let Some(x) = map.get(&k) { ... } else { map.insert(k, ...); }` compiles (the `get` borrow ends at the else branch start).
- Two-phase borrows subsumed: `vec.push(vec.len())` type of patterns.

### RFC 2173 — Inner Attribute Syntax
- `#![...]` applies to enclosing item; `#[...]` applies to next item. Both stable since long ago; 2173 extended allowed inner-attr positions.
- Used prominently in `#![forbid(unsafe_code)]`, `#![warn(clippy::all)]` at crate root.

### RFC 2394 — async/await (stabilized 1.39)
- `async fn f(...) -> T` desugars to `fn f(...) -> impl Future<Output = T>`.
- `.await` compiles to a state machine resume point.
- `async { ... }` block: type is an anonymous `Future`.
- Implicit self-referential state → `!Unpin`.
- `.await` only valid inside async context.

### RFC 2456 — Edition 2018 Final
- `async`, `await`, `try` keywords reserved.
- Path clarity changes: `use` path root rules.
- Anonymous lifetimes `'_` in types.
- `dyn` keyword for trait objects required (previously optional).
- `?` operator for `Result`.

### RFC 3216 — Closure Capture Disjoint (Rust 2021)
- Closures capture only the subpath of a struct actually used.
- Example:
  ```rust
  struct Foo { x: i32, y: String }
  let mut f = Foo { x: 0, y: "a".into() };
  let c = || { let _ = &f.x; };   // Captures only f.x
  f.y.push('b');                   // OK — previously would have failed
  ```
- Migration lint `rust_2021_incompatible_closure_captures` for drop-order change cases.

### RFC 3425 — RPIT-in-Trait / async-fn-in-trait (stabilized 1.75)
- `trait T { fn f(&self) -> impl Future<Output = U>; }` and `async fn`.
- Each impl has its own opaque type.
- Trait is NOT dyn-compatible (each impl's hidden type differs).
- Workarounds for dyn: `async-trait` crate (boxes Future); manual `fn f(&self) -> Pin<Box<dyn Future>>`.

### RFC 3556 — RFC Index
- Meta RFC adding structure to the RFC book navigation. Usability improvement only.

### RFC 2533 — (pattern types?) / RFC 3535 — Inherent associated types — both unstable as of 1.95.

### RFC 1210 — Specialization (status)
- Originally accepted, implementation paused due to soundness.
- Dropck + specialization interaction: lifetime-dependent specialization is unsound.
- Only `min_specialization` (gated, rustc-internal) ships.
- No user-facing stable specialization.

### RFC 3137 — if-let temporary scope narrowing (Rust 2024)
- `if let p = e { ... }`: temporaries in `e` now drop at end of `if` (or before `else`), not end of statement.
- Fixes MutexGuard-across-arms footgun.

### RFC 2497 / Let Chains (stabilized 2024)
- `if let` and `if bool` conditions composable with `&&` (not `||`).
- Binding introduced by `let` is in scope in the rest of the chain and in the body.

### RFC 3425 (again) — `unsafe_op_in_unsafe_fn`
- Note: RFC *2585* governs the lint; edition 2024 warns by default.
- `unsafe fn` no longer implicitly makes its body an unsafe block.

### RFC 3407 — `let-else` (stabilized 1.65)
- `let P = e else { diverge };` — pattern P must match; else block must diverge.

### RFC 3425 — Never type fallback change (2024)
- `!` falls back to `!` instead of `()`.

### ATPIT / TAIT (various RFCs, stabilized 1.74-1.85)
- Type-alias `impl Trait`: `type X = impl Trait;` for named opaque types.
- Each `type X = impl Trait;` is a defining use tied to a function body.
- Associated TPIT: `type Item<'a> = impl Iterator<Item = &'a T>;` — stable 1.79+.

### RFC 2229 (basis for 3216) — fine-grained closure captures

### RFC 2945 — sendable `dyn Future` gates

### RFC 3513 — gen blocks (stabilized ~1.89)
- Synchronous generators via `gen { yield ... }` producing `impl Iterator`.

## [TAG: 12-modern-rust] Async Rust Idioms (1.75 - 1.95 era)

- Rule: never hold a `MutexGuard` across `.await`. Use `tokio::sync::Mutex` (async) or scope the guard.
- Rule: prefer `tokio::select!` / `futures::future::try_join!` over ad-hoc polling.
- Rule: spawn with `tokio::task::spawn` requires `'static` and `Send`. Use `spawn_local` (single-threaded runtime) for `!Send`.
- Rule: `async fn foo() -> Result<T, E>` and then `.await?` chains naturally. Prefer `anyhow::Error` or `thiserror` for error handling.
- Pattern: Cancel safety — async function should leave the world in a valid state if dropped mid-await.
  - Idiomatic: scope guards for cleanup, avoid partial mutations.
  - `tokio::select!` cancels losers — make sure losers' state is recoverable.
- Pattern: static dispatch of async traits via `impl Trait` returns (stable 1.75):
  ```rust
  trait Repo { async fn get(&self, id: u64) -> Option<User>; }
  // Returns concrete Future; can be boxed externally if needed.
  ```
- Pattern: dynamic dispatch of async — use the `async-trait` macro or build `Pin<Box<dyn Future>>` manually.
- Rule: `Stream` trait (from `futures`) — analogue of `Iterator` for async. Not yet in std (as of 1.95; under discussion). Use `futures_core::Stream`.
- Rule: `AsyncIterator` trait (nightly) — the stream-equivalent in std.
- Rule: `async` and `fn` together — `async fn` returns impl Future. `fn foo(...) -> impl Future + '_` is equivalent and more control.

## [TAG: 12-modern-rust] Send Bounds on RPITIT / async trait methods (RTN)

- Rule (Return-Type Notation — `Trait<method(..): Send>`): lets trait consumers bound the return type of specific methods.
- Example:
  ```rust
  trait Backend {
      async fn fetch(&self, id: u64) -> Data;
  }
  
  fn spawn_worker<B: Backend<fetch(..): Send> + Send + 'static>(b: B) {
      tokio::spawn(async move { let _ = b.fetch(0).await; });
  }
  ```
- Rule: available stable in 1.85+ for most cases. Syntax `Trait<method(..): Bound>` means "the future returned by `method` implements Bound".

## [TAG: 02-language-rules] Coherence Examples (Orphan Rule) in Practice

- Allowed:
  - `impl MyTrait for Vec<i32>` (MyTrait is local).
  - `impl Display for MyType` (MyType is local).
  - `impl<T> MyTrait for Box<T>` (Box<T> is fundamental → local if T might be; MyTrait local).
  - `impl<T: MyTrait> From<T> for MyNewtype` (MyNewtype local).
- Forbidden:
  - `impl Display for Vec<i32>` (neither local).
  - `impl<T> Display for Box<T>` (Display not local; Box fundamental but T uncovered).
  - `impl<T> MyTrait for T` — allowed only if MyTrait is local AND T is bounded tightly OR blanket impl is restricted by a sealed supertrait.
- Workaround: newtype pattern. `struct Wrapper(Vec<i32>);` and `impl Display for Wrapper`.
- Workaround: sealed-trait extension. Define a local trait that's only implementable by your crate's types, then blanket impl the foreign trait on `impl MyLocalTrait` bound.

## [TAG: 02-language-rules] Coherence + GATs: Dyn-Compat Trade-off

- GATs (Generic Associated Types, 1.65): `type Item<'a> where Self: 'a`.
- Powerful for lending iterators: `fn next<'a>(&'a mut self) -> Self::Item<'a>`.
- Cost: trait becomes non-dyn-compatible (can't create `&dyn Trait`).
- Workaround: keep the GAT trait static-dispatch-only. Provide a separate `dyn`-compatible flavor (boxed) if needed.

## [TAG: 08-unsafe-and-ffi] More UB Edge Cases (hard-won knowledge)

- UB: `slice::from_raw_parts(ptr, isize::MAX + 1 as usize / size_of::<T>())` — creates an over-sized slice; validity violated even if never dereferenced.
- UB: calling a `#[target_feature(enable = "avx")]` function without the feature enabled. The `unsafe fn` marking is there for this reason.
- UB: creating `NonZeroU8::new_unchecked(0)` — safety invariant immediately violates validity.
- UB: transmuting an invalid discriminant into an enum type, even if you never match on it. `transmute::<u8, MyEnum>(99)` where only 0..=2 are valid → UB.
- UB: using `ptr.offset(n)` where `n` takes you beyond one-past-end of the allocation. Use `ptr.wrapping_offset(n)` for arbitrary arithmetic without dereferencing.
- UB: creating a reference (`&T` or `&mut T`) to a bit-valid-but-safety-invariant-violating value. Example: `&String` where the inner bytes aren't UTF-8.
- UB: Calling `mem::swap(&mut a, &mut b)` where the two `&mut` overlap partially — impossible from safe code but dangerous via unsafe.
- UB: Holding a `&mut T` to a location while a parallel `*mut T` writes through it. Classic aliasing violation.
- UB: in-place mutation through `*const T` when the original provenance came from `&T`. Writes require write provenance.
- UB: floating-point NaN bits — any bit pattern is valid for f32/f64, so NaN is NOT UB, but comparing NaNs with Ord traits can cause logic bugs (not UB). Use `PartialOrd` / `total_cmp`.
- UB: Reading from `static mut` while another thread accesses it — even reads, if any other thread writes, is a data race.
- UB (subtle): creating `&mut T` from `&UnsafeCell<T>::get()` when you only have `&UnsafeCell<T>` but there's a competing `&UnsafeCell<T>` somewhere else that's been dereferenced to `&T` — aliasing depends on what's live right now.

## [TAG: 08-unsafe-and-ffi] Unsafe Idiom: `#[repr(C)]` Variant Type Tagging

- Pattern: roll your own discriminated union for FFI / serialization:
  ```rust
  #[repr(u32)]
  enum Tag { A = 0, B = 1 }
  
  #[repr(C)]
  pub union Data { a: A, b: B }
  
  #[repr(C)]
  pub struct Tagged { tag: Tag, data: Data }
  ```
- Rule: reading `tagged.data.a` is UB if `tagged.tag != Tag::A`. Always validate tag first.
- Rule: `Tag` must be `#[repr(u32)]` or similar so the ABI is known.
- Pattern (Rust-internal tagged enums via `#[repr(C, u32)]`): stable RFC 2195 layout — same as the struct+union above.

## [TAG: 08-unsafe-and-ffi] Error Handling in Unsafe

- Rule: unsafe operations that can fail should return `Option`/`Result` at the safe-API boundary, not silently do UB.
- Rule: distinguish "safe function with internal unsafe" (bounds-checked, returns Result) from "unsafe function" (caller asserts precondition).
- Rule: `unreachable_unchecked()` (unsafe intrinsic) tells the optimizer a branch never happens. Using it incorrectly is UB. Prefer `unreachable!()` unless profiling shows the branch check is hot.

## [TAG: 04-design-patterns] The "Safety-Boundary" API Pattern

- Anti-pattern: exposing raw unsafe pieces as a library API. User can misuse them.
- Pattern: inside the library, use unsafe freely; expose a narrow, safe API that enforces invariants via types.
  - Example: `std::slice::split_at_mut` — safe caller signature, internal unsafe with bounds assert.
  - Example: `String::from_utf8` — validates first, then unsafe construction.
  - Example: `Vec::retain` — safe public, internal slice manipulation may use unsafe.
- Pattern: for every `unsafe fn foo()` in your library, ask "can I provide a safe `foo_checked()` that returns Result?". If yes, offer it.

## [TAG: 04-design-patterns] The "Unsafe Trait + Invariant" Pattern

- Pattern: When unsafe code depends on a trait impl being correct, make it an unsafe trait.
- Example: `Send`, `Sync` — unsafe impl ensures the implementor has audited thread-safety.
- Example: `GlobalAlloc` — unsafe trait; impl must respect Layout contracts.
- Example: `TrustedLen` (nightly) — marker trait asserting an Iterator's `size_hint` returns exact upper = lower bound. Used by specialized collectors.
- Pattern: add unsafe trait + safe blanket impl on known-good types:
  ```rust
  unsafe trait KnownSize { const SIZE: usize; }
  unsafe impl KnownSize for u32 { const SIZE: usize = 4; }
  ```

## [TAG: 04-design-patterns] Specific Idiom: Type-State Closure Tricks

- Typestate pattern via phantom type parameter:
  ```rust
  struct Conn<State>(UnderlyingSocket, PhantomData<State>);
  struct Open; struct Closed;
  impl Conn<Closed> { fn open(self) -> Conn<Open> { ... } }
  impl Conn<Open> { fn send(&self) { ... } fn close(self) -> Conn<Closed> { ... } }
  ```
- Benefit: impossible to call `send` on `Conn<Closed>` — compile error, not runtime.

## [TAG: 09-performance] Fast Paths via Unsafe — Real Examples

- `memchr`-style: `core::slice::memchr::memchr(b, haystack)` — SIMD lookup for a byte, safe wrapper.
- Custom FFT: inline `target_feature = "avx2"` for batched ops; fallback via runtime detection.
- Bump allocators (`bumpalo` crate): unsafe internally for O(1) alloc, but safe API (scope-based reset).
- Lock-free queues: `crossbeam::deque` — epoch-based reclamation (crossbeam-epoch). Unsafe internally, safe public API.
- Rope / Gap buffer data structures: unsafe moves for fast insertion, safe iterators for consumption.
- `SmallVec<[T; N]>`: inline storage for small N, heap fallback. Stable-safe API, unsafe internals.

## [TAG: 08-unsafe-and-ffi] Reading Other People's Unsafe

- Checklist when reviewing an unsafe crate:
  1. Does every unsafe block have a SAFETY comment?
  2. Are invariants documented at the module/struct level?
  3. Does the public API allow construction of any safety-invariant-violating value? (If yes, it must be `unsafe fn`.)
  4. Is Miri run in CI?
  5. Are there `#[test]`-gated invariant checks at boundaries?
  6. Do raw pointer fields use `NonNull` + `PhantomData` or raw `*mut`/`*const`?
  7. Are Send/Sync impls present, and are they manually bounded or auto-derived?
  8. Is there a Drop impl? Does it handle partial initialization / failed allocation?
  9. Does the crate ever rely on destructors running?
  10. Are any dependencies also audited? (Leaky unsafe at dep boundary is common.)

## [TAG: 12-modern-rust] Diagnostics & Tooling Updates (1.70 - 1.95)

- `clippy::undocumented_unsafe_blocks` — warns on unsafe block without SAFETY comment.
- `clippy::missing_safety_doc` — warns on `unsafe fn` without `# Safety` doc.
- `clippy::multiple_unsafe_ops_per_block` — prefer one operation per unsafe block.
- `rustc_lint::unsafe_op_in_unsafe_fn` — deny for 2024 crates.
- `rustc_lint::improper_ctypes_definitions` — checks `extern` block signatures for FFI safety.
- `rustc_lint::fuzzy_provenance_casts` (nightly) — flags int→ptr under strict provenance mode.
- `#[diagnostic::on_unimplemented]` — customize trait-bound error messages.
- `cargo audit` — security advisories for dependencies.
- `cargo vet` — audit trails for dependency trust levels.
- `cargo miri` — nightly interpreter with UB detection.
- `rust-analyzer` inlay hints show inferred types; invaluable when tracking down inference-related unsafe issues.

## [TAG: 02-language-rules] Reference / Pointer Size and Alignment Quick Facts

- `&T`, `&mut T`, `*const T`, `*mut T`, `Box<T>`, `NonNull<T>` when T: Sized: one word (pointer size).
- `&dyn Trait`, `Box<dyn Trait>`, `&[T]`, `&str`: two words.
- `Rc<T>`, `Arc<T>`: one word; inner allocation has `[strong_count: AtomicUsize, weak_count: AtomicUsize, data: T]`.
- `Vec<T>`: three words (ptr, cap, len).
- `String`: three words (same as `Vec<u8>`).
- `HashMap<K, V, S>`: depends on backing; typically ~6-10 words.

## [TAG: 02-language-rules] When Variance Matters in Practice

- Problem: You have a `struct Cache<'a, T> { inner: HashMap<&'a str, T> }`. If you want to insert a short-lived key into a Cache that was constructed with a long-lived key, variance determines legality. HashMap is invariant in K (because `&mut HashMap<K, V>` allows writing) — conservative.
- Problem: `struct Client<'a> { conn: &'a Connection }`. If `Connection` has interior mutability (`UnsafeCell` inside), the client becomes invariant in `'a` — you can't shorten the lifetime.
- Fix: use `Cow<'a, T>` for on-demand-owned data to avoid lifetime propagation.
- Fix: `PhantomData<fn(&'a ())>` to force contravariance in `'a` if needed.

## [TAG: 08-unsafe-and-ffi] Canonical Unsafe Contracts

### `slice::from_raw_parts(ptr, len)`
Safety:
- `ptr` must be valid (aligned, non-null, points to `len * size_of::<T>()` bytes) for reads.
- Bytes must be initialized as `T` and aliasing-compatible with `&[T]`.
- `len * size_of::<T>() <= isize::MAX`.

### `String::from_utf8_unchecked(bytes)`
Safety: bytes must be valid UTF-8.

### `str::from_utf8_unchecked(&bytes)`
Safety: same.

### `Box::from_raw(ptr)`
Safety: `ptr` must have been obtained from `Box::into_raw` (or an equivalently-allocated pointer) with the correct layout. Exclusive ownership.

### `Arc::from_raw(ptr)`
Safety: obtained from `Arc::into_raw`. Consumes one strong reference.

### `NonNull::new_unchecked(ptr)`
Safety: `ptr` is non-null.

### `MaybeUninit::assume_init()`
Safety: the MaybeUninit has been fully initialized to a valid T.

### `mem::transmute(value)`
Safety: both types have the same size and `value`'s bits represent a valid target-type value.

### `ptr::read(src)`
Safety: src is aligned, valid for reads, pointee is valid T. Does not deinitialize; the caller should ensure no double-drop.

### `ptr::write(dst, val)`
Safety: dst is aligned, valid for writes. Does not drop the old value.

### `ptr::copy_nonoverlapping(src, dst, n)`
Safety: both aligned, both valid, regions disjoint.

### `ptr::drop_in_place(ptr)`
Safety: `*ptr` is a valid T; no further access to those bytes afterward.

### `slice::get_unchecked(i)` / `get_unchecked_mut(i)`
Safety: `i < len`.

### `char::from_u32_unchecked(v)`
Safety: `v` is a valid Unicode scalar value (not in 0xD800..=0xDFFF and <= 0x10FFFF).

### `NonZeroU32::new_unchecked(v)`
Safety: `v != 0`.

### `Box::new_zeroed().assume_init()`
Safety: all-zero bits are a valid T.

### `Pin::new_unchecked(ptr)`
Safety: the pointee will not be moved for as long as the Pin is alive (unless T: Unpin).

### `core::intrinsics::unreachable()`
Safety: control flow cannot actually reach this.

### `core::hint::unreachable_unchecked()`
Safety: same.

## [TAG: 12-modern-rust] "Everything is an Expression" — Rust 2024 touches

- Rule: blocks `{ expr }` are expressions. The last expression (without trailing `;`) is the block's value.
- Rule: `if`/`match`/`loop`/`unsafe {}`/`const {}`/`async {}` are all expressions.
- Rule (2024): tail expression temporary scope now dies immediately after being produced by the enclosing `match` / `if` / `block`. Pre-2024, the temporary lived to the end of the enclosing statement.
- Gotcha: `let v = &something.method_that_returns_ref();` — the temporary from `something.method()` dies at statement end (so the reference may dangle). Use `let tmp = something.method(); let v = &tmp;`.
- Rule: `let ... else` requires the else block to diverge (`!`-typed).
- Rule: expression-context attributes are limited: only `#[cfg(...)]`-family attrs; most custom attrs work only at item level.

## [TAG: 08-unsafe-and-ffi] Top 10 Unsafe Invariants to Internalize

1. `&mut T` is unique — violations anywhere → UB.
2. `&T` freezes memory (except `UnsafeCell`) — write invalidation → UB.
3. References and `Box` are non-null, aligned, dereferenceable.
4. Validity invariants (bool, char, NonZero, enum discriminant) must ALWAYS hold.
5. Uninit memory is only legal inside `MaybeUninit`, unions, padding.
6. Forget may leave destructors unrun — do not rely on Drop for soundness.
7. Panic can unwind at any expression; unsafe code must maintain invariants across panic points.
8. FFI: unwinding across non-`-unwind` ABI is UB. Catch panics at boundaries.
9. Layout of `repr(Rust)` is unstable; never transmute between Rust-repr types.
10. Aliasing UB is about live references — create `&mut T` while any other live ref exists → UB, even if never used.

## [TAG: 08-unsafe-and-ffi] UCG Glossary — Formal Terms (*Source-extracted*)

- **Abstract byte**: More than `0..256`; shadow state for the Rust Abstract Machine (initialization + provenance). Copies propagate uninit like `Option`-style states.
- **Aliasing**: Pointers/refs whose memory **spans** overlap. Span = base + `size_of_val` for refs; ZST spans are empty → pairs of ZST references never alias by this definition.
- **Allocation**: Contiguous address range; deallocated as a unit; provenance ties pointer arithmetic to the originating allocation.
- **Interior mutability**: Mutation while a live shared `&T` covers the same bytes, unless mutation uses `UnsafeCell`. Liveness propagates through nested references, **not** through raw pointers hiding under `&T`.
- **Layout vs representation vs ABI**: Layout = size, alignment, offsets. **ABI** = call-boundary passing — stricter than layout (e.g. `repr(C) struct S(i32)` vs `i32`).
- **Niche**: Invalid bit-pattern usable for enum layout optimization. Not every invalid pattern is a niche (uninit `&mut T` is invalid but not a niche).
- **Padding**: Compiler-inserted gaps; copying padding may yield **uninit** bytes in the destination (typed copy).
- **Pointer provenance**: Extra AM state; same address ≠ same pointer if provenance differs (`wrapping_offset` counterexample in UCG glossary).
- **Soundness**: Safe API cannot cause UB.
- **Validity vs safety invariant**: Validity must hold on every typed move/assign/call — compiler assumes. Safety invariant is what **safe** code may assume; unsafe may temporarily break safety (e.g. invalid UTF-8 in `String`) but **not** validity, without UB risk when safe methods run.

## [TAG: 02-language-rules] Rustc Dev Guide — Type Inference (*Source-extracted*)

- Based on **Hindley–Milner** + extensions: **subtyping**, **region inference**, **HRTB**.
- Inference variables: `?T` general; integral/float literals get constrained integral/float vars.
- Primary ops: `infcx.at(...).eq` / `.sub`; success returns `InferOk` with possible **trait obligations** to fulfill.
- `can_eq` / `can_sub`: probe without committing; **always modulo regions** — `&'a u32` vs `&'b u32` may look equatable before region solve.
- **Snapshots**: rollback/confirm inference state for backtracking (`probe`, `commit_if_ok`).
- **Subtyping with regions**: `?T <: &'a i32` → introduce `'?b`, unify `?T` with `&'?b i32`, emit constraint `'?b: 'a`.
- **Region constraints**: `'a: 'b` collected; solved **late**. Two solvers: **lexical** vs **NLL/MIR** type-checker — eventually one. NLL needs **where** constraints occur in CFG → `take_and_reset_region_constraints` + `get_region_var_infos`.
- **Leak-check** (HRTB / trait system): root-universe regions must not affect trait system (erased at codegen).

## [TAG: 02-language-rules] Rustc Dev Guide — Trait Resolution Overview (*Source-extracted*)

- **Obligation**: trait reference needing proof. **Selection**: pick impl / where-clause / builtin. **Fulfillment**: worklist until empty. **Evaluation**: holds without constraining inference?
- Selection result: definite impl, **ambiguous** (`None`, often due to inference vars), or error.
- **Candidate assembly** + **winnowing** + `candidate_should_be_dropped_in_favor_of` — resolves overlap when multiple impls unify.
- **Lifetime matching deferred** — selection infallible for regions; errors arise later in region check.
- **TypingMode**: stricter behavior during **coherence** — prevents treating goals as false or types unequal prematurely (soundness in overlap / orphan checks).

## [TAG: 02-language-rules] Rustc Dev Guide — MIR Basics (*Source-extracted*)

- MIR = CFG of **basic blocks**: **statements** (single successor) + **terminators** (calls, branches, unwind edges).
- No nested expressions; types explicit. **Places** vs **Rvalues**; **Operands** = copy/move **Place** or constant.
- **StorageLive** / **StorageDead**: stack slot lifetime for LLVM; optimized away if unused.
- Function calls are terminators (unwind edge possible). `body: Body` + promoted constants.
- **MIR borrow check** query: `mir_borrowck` — depends on NLL region computation (`rustc_borrowck::nll::compute_regions`, type_check, liveness).

## [TAG: 08-unsafe-and-ffi] Nomicon — Closed Universe of Unsafe Features (*Source-extracted*)

- Unsafe can **only**: deref raw pointers; call `unsafe` functions; `unsafe impl` traits; access `mut static`; access `union` fields.
- **Invalid values** (including wide metadata): wrong `dyn` vtable; bad slice length; **dangling** pointer/ref (null or bytes not in one allocation). **Producing** includes assign, pass, return.
- **Not** UB by Rust core rules: deadlock, races, leaks, int overflow on `+`, abort — still usually bugs.

## [TAG: 08-unsafe-and-ffi] Nomicon — Safety Non-Locality & Module Privacy (*Source-extracted*)

- Changing `idx < len` to `idx <= len` breaks soundness **outside** the `unsafe` block — safe code governs preconditions.
- Invariants on private fields must not be breachable by **safe** code in the same module — still unsound if safe `fn` mutates `cap` wrongly.

## [TAG: 02-language-rules] Nomicon — Variance Table & Struct Combining (*Source-extracted*)

- `&'a T`: covariant in `'a`, `T`. `&'a mut T`: covariant in `'a`, **invariant** in `T`. `*const T`: covariant in `T`. `*mut T`: invariant. `UnsafeCell<T>`: invariant. `fn(T)->U`: **contravariant** in `T`, covariant in `U`.
- Struct with multiple uses of `A`: all covariant → covariant; all contravariant → contravariant; **mixed → invariant**.

## [TAG: 02-language-rules] Nomicon — Drop Check & `may_dangle` (*Source-extracted*)

- Generic `Drop` may run after fields dropped — if destructor could read **references to fields**, require **strict outlives** relationships; dropck enforces conservative rules.
- `#[may_dangle]` on type/lifetime params: **unsafe** promise destructor won't read expired data; interacts badly with indirect calls (`Display`, closures), **future specialization**.

## [TAG: 04-design-patterns] Nomicon — PhantomData & Vec Dropck (*Source-extracted*)

- Unused lifetime/type params must appear in **PhantomData** (or fields) for variance and dropck.
- **RFC 1238**: `PhantomData<T>` no longer required solely for dropck when `impl Drop for Vec<T>` exists — `Drop` impl causes compiler to treat `T` as potentially dropped.
- Std `Vec` still uses `PhantomData` for variance + `#[may_dangle]` interaction + `NonNull` covariance.

## [TAG: 08-unsafe-and-ffi] Nomicon — FFI & Unwind (*Source-extracted*)

- **`extern` blocks are `unsafe`** in Rust 2024 — foreign calls require `unsafe { ... }`.
- ABI strings: use **`*-unwind`** when unwinding may cross; otherwise unwinding hits boundary → Rust `panic` **aborts**; **C++ exception entering non-unwind Rust** → **UB**. `catch_unwind` vs foreign exceptions unspecified/UB.
- **Opaque types**: `repr(C)` + private fields + marker — **do not** use empty enums for FFI placeholders.

## [TAG: 09-performance] Nomicon — `impl Vec` Tutorial: Unsafe Invariants (*Source-extracted*)

- `NonNull::dangling()` for empty non-ZST cap=0; **allocator** `alloc(0)` is UB — guard.
- ZST: capacity saturates; pointers as **integer counters** for iteration — mind alignment when calling `read` (use `NonNull::dangling()` for ZST reads).
- `ptr::write` on push; `ptr::read` on pop; `ptr::copy` for insert/remove.
- `Drain` + **`mem::forget`**: set `len = 0` first → leaks if forgotten — **no UAF**.

## [TAG: 04-design-patterns] Nomicon — `Arc` Toy Implementation (*Source-extracted*)

- Raw `*mut ArcInner<T>` alone → wrong variance + dropck — use **`NonNull<T>` + `PhantomData<ArcInner<T>>`**.
- `clone`: `fetch_add(..., Relaxed)` but **abort** if count reaches `isize::MAX` (defense against `mem::forget` explosion).
- `drop`: `fetch_sub(..., Release)`; if last, **`fence(Acquire)`** before `Box::from_raw` — synchronizes with clones using data via `Release` chains.

## [TAG: 12-modern-rust] RFC 2094 — NLL for LLMs (*Source-extracted*)

- Borrow **lifetime** = minimal CFG region covering all uses — **not** the enclosing lexical block when narrower suffices.
- Motivating failures: stored references extending borrows; `match` arms; `get_default` patterns — many fixed; some still need `entry`-style APIs or polonius.

## [TAG: 12-modern-rust] RFC 1210 — Specialization Design (*Source-extracted*)

- Allows **overlapping** impls when one is strictly **more specific**; enables `default` methods in traits specialized by more specific impls.
- **Performance + reuse**: blanket `Extend` + slice fast-path example in RFC text.
- **Status**: full specialization **not** stable; soundness (dropck/lifetime interactions) limits surface. Use **`min_specialization`** under nightly for limited cases.

## [TAG: 08-unsafe-and-ffi] The Reference — `behavior-considered-undefined` (normative snapshot)

*Source: [doc.rust-lang.org/reference/behavior-considered-undefined.html](https://doc.rust-lang.org/reference/behavior-considered-undefined.html). Список не исчерпывающий и может меняться между версиями компилятора.*

- **Meta**: UB возможен и внутри `unsafe`; `unsafe` только перекладывает ответственность на программиста. Список в Reference **не exhaustive** — формальной полной модели нет.
- **Data races** — UB.
- **Доступ (load/store)** к месту, которое **dangling** или основано на **misaligned** указателе — UB.
- **Place projection вне in-bounds арифметики** (`offset`-правила для цепочки полей/индексов) — UB.
- **Aliasing**: точные правила ещё не зафиксированы; концептуально `&T` — без мутаций пока жив (кроме `UnsafeCell`); `&mut T` — без чтений/записей указателями не из этого `&mut` и без других ссылок на тот же объект; **`Box<T>` как `&'static mut T`** для этих целей. Границы живости:
  - сверху ограничены временем жизни, выведенным borrow checker;
  - при каждом разыменовании/реборроу — жива;
  - при передаче в функцию / возврате — жива;
  - при передаче `&T` в функцию — жива **минимум** на время вызова (кроме `UnsafeCell` внутри `&T`).
  - То же для вложенных полей **составного** значения, **но не** за индирекцией по указателю.
- **Immutable bytes**: нельзя писать в байты, достижимые через const-promotion, `'static` lifetime-extended временные в инициализаторах, immutable binding/`static`; байты за **shared** ссылкой (транзитивно, в т.ч. через поля и `Box`) — immutable; **запись >0 байт**, пересекающихся с этими байтами, = мутация (даже no-op по значению).
- **Intrinsics** с UB, **target_feature** без поддержки на ЦП, **неверный ABI** или **unwind** через кадр без unwind — UB.
- **Invalid value** при присваивании/чтении/аргументах/возврате — UB.
- **Неверный inline asm**, нарушение предположений рантайма (в т.ч. **`longjmp`** без дропов фрейма Rust).
- **FFI**: UB в C — UB всей программы, и наоборот.

### Указательность и выравнивание (Reference)

- **Span** «куда указывает» указатель/ссылка: адрес + `size_of_val` по фактическому динамическому типу.
- **Misaligned place**: последняя `*` в **полном** place должна быть выровнена под **тип того указателя**, который разыменовывается (`*const S` с align 8 → `(*ptr).f` с `f: u8` всё равно требует 8-aligned `ptr`). UB только при фактическом load/store; **`&raw const` / `&raw mut`** от такого места — разрешены.
- **`&`/`&mut`** к полю требуют выравнивания под **тип поля** (часто слабее); при `repr(packed)` компилятор может запретить создать ссылку.
- **Dangling**: не все байты span в одной **живой** аллокации — UB. **Исключение: ZST** — при size 0 указатель **никогда** не считается dangling (даже null).
- **DST**: длина слайса/строки не должна делать `size_of_val > isize::MAX`.
- **Validity**: отдельные пункты для `bool`, `fn` ptr non-null, `char`, `!`, инициализированность скаляров/сырых указателей, `str`, enum/struct/array, спор по `union`, ref/`Box`/wide metadata, custom niche (`NonNull`, `NonZero`).
- **Const evaluation / provenance**: в const-контексте «чистые» целые (`i*`/`u*`/`f*`/`bool`/`char`, дискриминанты, metadata слайса) **не должны нести provenance**. Значения с указателями — либо без provenance, либо байты — фрагменты **одного** исходного указателя **в правильном порядке**. Иначе UB (в т.ч. «прочитать указатель как usize» через `read` сырых байт).

## [TAG: 08-unsafe-and-ffi] Tree Borrows (Miri) — рабочая модель алиасинга

*Source: [unsafe-code-guidelines `wip/tree-borrows.md`](https://github.com/rust-lang/unsafe-code-guidelines/blob/master/wip/tree-borrows.md) — **не нормативно**, отражает Miri; полная спецификация в [MiniRust](https://github.com/minirust/minirust/tree/master/spec/mem/tree_borrows) и [PLDI’25 paper](https://plf.inf.ethz.ch/research/pldi25-tree-borrows.html).*

- Альтернатива **Stacked Borrows** в Miri: `-Zmiri-tree-borrows`.
- На каждую аллокацию — **дерево**; у каждого указателя **tag** → узел; на каждый байт — **permission** (state machine: read/write, local/foreign, protector).
- **Retag**: как у Stacked Borrows; **сырой указатель** при retag — **NOP** (не отделяется от порождённых ссылок так же жёстко, как в SB).
- **Protectors** (strong/weak): ссылка должна оставаться «живой» на время вызова — как в SB.
- **Implicit accesses** при retag: в TB для `&mut` это **всегда read** (не write), в отличие от SB.
- **UnsafeCell**: тоньше, чем «где-то в структуре есть UnsafeCell» — отслеживается позиция полей; для interior mutability — отдельное **`Cell` permission** (доступы foreign+local разрешены); **не** как «сырой указатель».
- **Открытые вопросы**: exposed provenance в TB; protector end writes vs data races; поведение может ужесточиться до уровня SB по отдельным паттернам.

### Отличия TB от «хотелок оптимизатора» (из UCG wip)

- В TB **нет subobject provenance** — retag не сужает диапазон offset’ов (спорно для будущих оптимизаций).
- `&mut` **не** считается сразу writable — writable становится **после первой записи** (важно для порядка перемещения записей).

## [TAG: 08-unsafe-and-ffi] Stacked Borrows vs Tree Borrows — сжатая матрица (Miri)

*Оба: **не** спецификация языка, только то, что проверяет Miri. SB — режим по умолчанию; TB — `rustc -Zmiri-tree-borrows` (cargo: `MIRIFLAGS=-Zmiri-tree-borrows`). Источники: UCG `wip/stacked-borrows.md`, `wip/tree-borrows.md`.*

| Аспект | Stacked Borrows | Tree Borrows |
|--------|-----------------|--------------|
| Структура состояния | На **каждый байт** — **стек** заимствований (`Item`: permission + tag + optional protector) | На **каждую аллокацию** — **дерево** узлов; на байт — permission + «accessed» bit |
| Сырой указатель | Участвует в модели; **Raw retag** после `&T as *const T` | Retag сырого — **NOP**: сырой почти не отделяется от родительских ссылок |
| `&mut` и первая запись | После retag `&mut` ведёт себя как **writable** (через permissions на стеке) | **Writable только после первой записи** по этому заимствованию — иначе read-only до поры |
| Subobject / сужение span | Retag **сужает** offsets, с которыми может ходить ссылка (интуиция «подобъект») | **Нет** subobject provenance — ссылка формально не «сужена» по сравнению с SB |
| Interior mutability / `UnsafeCell` | `SharedReadWrite` вокруг `UnsafeCell`; reborrow shared не лезет в варианты enum (как union) | Отдельное **`Cell` permission**; трекинг полей структуры точнее, чем «где-то есть UnsafeCell» |
| Implicit при retag | Для `&mut` могут быть **writes** при некоторых retag (исторически SB) | Для `&mut` при retag — **только reads**; отдельно «protector end» может писать (см. paper) |
| Что **запрещено в SB**, но **разрешено в TB** (сейчас) | Ряд C-подобных паттернов с сырыми указателями и «лишними» чтениями | См. список *Imprecisions* в `tree-borrows.md` — **может стать UB позже** |

**Stacked Borrows — минимальная операционная схема для LLM**

- Каждое значение указателя несёт **tag** (`PtrId`); память помнит **стек** разрешений на доступ для каждого байта.
- **Permission**: `Unique` | `SharedReadWrite` | `SharedReadOnly` | `Disabled` (разделяет группы SRW).
- **Protector**: strong / weak + `CallId` — ссылка не должна быть инвалидирована пока жив вызов (обоснование `noalias`/`dereferenceable` в LLVM).
- **Retag** (вставляется в MIR): виды `FnEntry` (аргументы при входе), `TwoPhase`, `Raw` (после cast ref→raw), `Default` (присваивания ref/box, возврат из вызова, и т.д.). Без retag по пути `Deref` в LHS присваивания — известное ограничение реализации.
- **Вывод для unsafe-кода**: даже «законный» C-стиль может быть **UB в Miri SB**; прогон с **`-Zmiri-tree-borrows`** часто ближе к ожиданиям низкоуровневых авторов, но это **не** обещание будущего языка.

## [TAG: 08-unsafe-and-ffi] `union` — Reference + зона валидности

*Sources: [Reference — Union types](https://doc.rust-lang.org/reference/types/union.html), [behavior-considered-undefined — validity](https://doc.rust-lang.org/reference/behavior-considered-undefined.html).*

- **Нет «активного поля»** в семантике языка: каждое чтение поля — это **частичный transmute** содержимого в тип поля → **чтение поля union** в общем случае требует `unsafe` (см. [Union item](https://doc.rust-lang.org/reference/items/unions.html) — нюансы доступа к отдельным полям и `Copy`).
- **Типы полей ограничены**: только такие `T`, которым **никогда не нужен Drop** при разрушении поля (детали — в chapter items/unions).
- **Layout по умолчанию не зафиксирован** (в т.ч. поля не гарантированы на offset 0); для FFI/предсказуемости — `#[repr(C)]` или другой `repr`.
- **Validity (Reference)**: для `union` полные правила **ещё не решены**; однозначно валидны значения, конструируемые **целиком из safe-кода**; ZST-поле → «любая» битовая картина может быть валидна; остальное — [UCG #438](https://github.com/rust-lang/unsafe-code-guidelines/issues/438) и обсуждения.
- **Практика для LLM**: не делать `transmute` в `union` без документированного инварианта; не предполагать, что «я только что записал вариант A» делает чтение B безопасным без дополнительных гарантий инициализации; для tagged union в Rust предпочтительны **`enum`** с `repr`, если нужна доказуемая дисциплина.

## [TAG: 08-unsafe-and-ffi] Miri — флаги, влияющие на интерпретацию указателей

- По умолчанию: **Stacked Borrows**.
- `-Zmiri-tree-borrows` — **Tree Borrows** (мягче к ряду raw-паттернов; см. матрицу выше).
- `-Zmiri-strict-provenance` / permissive provenance: согласовать с документацией Miri на вашей версии nightly — TB **не** поддерживает полный permissive provenance (открытый вопрос в UCG).
- Рекомендация CI: прогонять **оба** режима на критичном `unsafe`, если цель — не «зелёный Miri любой ценой», а проверка под обе модели.

## [TAG: 02-language-rules] `Send`, `Sync`, auto traits — Reference

*Source: [special-types-and-traits](https://doc.rust-lang.org/reference/special-types-and-traits.html).*

- **`Send`**: значение безопасно **передавать** между потоками (владение).
- **`Sync`**: значение безопасно **разделять** через `&T` между потоками; эквивалентно `T: Send` для `&T`. Для всех типов в **immutable `static`** требуется `Sync`.
- **Auto traits**: `Send`, `Sync`, `Unpin`, `UnwindSafe`, `RefUnwindSafe` — автовывод по полям замыкания/агрегатов; для дженериков **нет** автогенерации, если есть ручной `impl` в std с более узкими границами (пример в Reference: `Send` для `&T` только если `T: Sync`).
- **Negative impl**: стабильно только в std (`*mut T` !Send и т.д.); свои `!Trait` — нестабильно.
- **Trait objects**: к одному `dyn Primary` можно добавить **дополнительные** auto-trait bounds: `dyn Primary + Send + Sync`.
- **`PhantomData<T>`**: для компилятора «владеет `T`» → variance, dropck, auto traits (см. тот же раздел Reference).

## [TAG: 08-unsafe-and-ffi] Const evaluation — границы для `unsafe` / указателей

*Source: [const_eval](https://doc.rust-lang.org/reference/const_eval.html).*

- В **const context** выражения **всегда** считаются на этапе компиляции; OOB индекс и overflow — **ошибка компиляции**, не «panic в рантайме».
- **`const fn`**, вызванная **вне** const context, ведёт себя как обычная функция (может выполниться в рантайме).
- Интерпретация const **на целевой архитектуре** (`usize` = размер цели, не хоста).
- **Запреты на borrow в const**: нельзя держать `&mut` / shared `&` к **interior mutability** / mutable borrow временного, если временное **lifetime-extended** до конца программы (tail position). Допустимы borrow **transient** (локальная переменная, временный без extension), **indirect** (`&mut *...`), **static** (`static` / `static mut` с оговорками).
- **Чтение `extern static`** в const запрещено; **запись в любой `static`** в const запрещена.
- Указатель→integer casts разрешены в const expr (с ограничениями provenance — см. также раздел Reference про validity в const).

## [TAG: 08-unsafe-and-ffi] Panic, unwind, FFI — нормативные правила

*Source: [panic](https://doc.rust-lang.org/reference/panic.html), пересечение с [linkage / unwinding](https://doc.rust-lang.org/reference/linkage.html).*

- **`panic=abort`**: оптимизатор может предполагать **нет** unwinding через Rust-кадры → меньше кода; нельзя полагаться на `catch_unwind` для перехвата.
- **Смешение panic strategies** при линковке: crate с `unwind` может линковаться с `abort` handler; **наоборот** — ограничения (см. Reference linkage).
- **UB (Reference)**: unwind в Rust из функции, объявленной как **не-unwinding** ABI (`"C"`, `"system"`, …), если иностранная сторона бросила исключение / размотка дошла до Rust.
- **UB**: вызов Rust-функции с **unwinding** ABI из кода **без** поддержки исключений (например GCC `-fno-exceptions`).
- **Не специфицировано**: перехват **иностранного** unwind через `catch_unwind` / `join` / выход за `main` — либо **abort**, либо `Err( opaque )`.
- **Исключение «чужого» std**: разные инстансы libstd ≈ «foreign exception» — возможен abort даже в дочернем потоке.
- **Нет гарантий** на dispose/rethrow payload Rust-паники иностранным рантаймом — паника Rust должна завершить процесс или быть поймана **тем же** Rust runtime.

## [TAG: 02-language-rules] `Pin` / `Unpin` — контракт для API и Future

*Sources: [std::pin::Pin](https://doc.rust-lang.org/std/pin/struct.Pin.html), [pin module](https://doc.rust-lang.org/std/pin/index.html).*

- **`Pin<Ptr>`** оборачивает **указатель** `Ptr`, обещая: pointee **не перемещают** и не **инвалидируют** на этом месте в памяти, пока живёт контракт, **если** `T: !Unpin`.
- **`Unpin`**: auto trait; для `T: Unpin` **`Pin::new`** безопасен и pinning — no-op семантически.
- **`Future::poll` принимает `Pin<&mut Self>`** — чтобы состояние async state machine могло содержать self-references.
- Безопасное создание pinning для `!Unpin`: **`Box::pin`**, **`pin!` macro** (stack), и т.д.; **`Pin::new_unchecked`** — `unsafe`: обещание, что значение не будет перемещено до выхода из `Drop` / снятия Pin.
- **`Pin` layout = layout `Ptr`** (прозрачно для ABI); сноска в доке: возможная тонкость **aliasing** `Pin<&mut T>` vs `&mut T` ещё обсуждается — не опираться на «это точно то же», что сырые алиасы в `unsafe`.

## [TAG: 04-design-patterns] `Box` — особые правила языка (orphan / receiver)

*Source: [special-types-and-traits — Box](https://doc.rust-lang.org/reference/special-types-and-traits.html).*

- `Box<T>` может иметь **`impl Trait for Box<T>` в том же крейте, что и `T`**, тогда как для других обобщённых контейнеров orphan rule это запрещает — важно для дизайна extension traits вокруг бокса.

## [TAG: 08-unsafe-and-ffi] `Vec` / `String` / `Box`: `into_raw` / `into_raw_parts` — контракты владения

*Для ревью FFI, кастомных аллокаторов и «вытащил указатель — верни назад».*

### `Vec<T>`

- **`Vec::into_raw_parts(self) -> (*mut T, usize, usize)`** (stable): возвращает `(ptr, len, cap)` с тем же смыслом, что у внутреннего буфера `Vec` после возможного reallocate.
- **`Vec::from_raw_parts(ptr, len, cap) -> Vec<T>`** — **`unsafe`**. Смысл полей **тот же**, что у живого `Vec`: `len` — число **инициализированных** элементов `T`, `cap` — ёмкость выделенного буфера в элементах.
- **Обязательства вызывающего `from_raw_parts`:**
  1. `ptr` получен из **`into_raw_parts`** этого же типа **или** эквивалентного однократного выделения через **глобальный** аллокатор с **`Layout::array::<T>(cap).unwrap()`** (тот же аллокатор, что и `Vec` — по умолчанию `Global`), с **выравниванием** `align_of::<T>()`.
  2. `len <= cap`; `len` элементов по адресу `ptr` — **валидны как `T`** (инициализированы); элементы с индексами `len..cap` могут быть неинициализированы (слот под будущий `push`).
  3. `cap` соответствует **ровно одному** выделению той же длины, что ожидает `Vec` (не склеивать два куска, не `cap` от другого типа).
  4. **ZST (`size_of::<T>() == 0`)**: указатель — **арифметический** sentinel (часто `align_of::<T>()` / non-null dummy); не вызывать `dealloc` на нём, если `cap == 0` — следовать доке `Vec`/`Layout::array` для ZST.
- **`Vec::from_raw_parts_in(ptr, len, cap, alloc)`** — то же, но аллокатор **должен совпадать** с тем, которым создан буфер; при `drop` `Vec` использует этот allocator для `dealloc`.
- **Анти-паттерн**: взять `ptr` из чужого `malloc`, передать как `from_raw_parts` без согласования layout/alignment — **UB** при `drop` или ранее.

### `String`

- **`String::into_raw_parts(self) -> (*mut u8, usize, usize)`** — как `Vec<u8>`; буфер должен содержать **валидный UTF-8** в первых `len` байтах.
- **`String::from_raw_parts(buf, len, cap)`** — **`unsafe`**: `buf` должен указывать на `len` байт **корректной UTF-8** последовательности и удовлетворять тем же правилам аллокации, что `Vec<u8>::from_raw_parts`.

### `Box<T>`

- **`Box::into_raw(b: Box<T>) -> *mut T`**: передаёт **владение** кучей на один объект `T`; `Box` не ранит `drop` — вызывающий обязан **`drop` через `ptr::drop_in_place`** или **`Box::from_raw`**.
- **`Box::from_raw(raw: *mut T) -> Box<T>`** — **`unsafe`**: `raw` — ровно тот указатель из **этого** `into_raw` (или эквивалентное выделение `Layout::new::<T>()` тем же аллокатором), **exclusive** владение; иначе double-free / use-after-free.

## [TAG: 04-design-patterns] Ownership inversion (Nomicon) — суть для LLM

- **Идея**: API **безопасно** возвращает ссылки/итераторы, которые внутри используют **сырые указатели** и временно «инвертируют» обычный порядок владения (например `split_at_mut`, `IterMut` по связному списку/дереву), пока **инвариант** «не два живых `&mut` на пересекающиеся байты» соблюдён **логикой типа**, а не только borrow checker.
- **Риск**: компилятор не доказывает корректность — автор **unsafe** обязан доказать отсутствие aliasing `&mut` и корректность при **panic/unwind** на границах.
- **Связь с `into_raw_parts`**: если «инверсия» держит сырой `*mut T` на буфер, при разрушении абстракции нужно либо собрать `Vec::from_raw_parts`, либо явно `dealloc` с тем же `Layout` — не смешивать контракты.

## [TAG: 02-language-rules] Borrow checker: NLL vs Polonius (одна страница)

- **NLL** (по умолчанию): регионы живости займов считаются на **MIR** + граф потока управления; большинство «глупых» lexical ошибок снято.
- **Polonius** (экспериментально, `-Zpolonius`): другой движок для **части** ограничений; цель — принять ещё больше **sound** программ (особенно связанные с **займами и условиями**), где NLL консервативен.
- **Практика для агента**: если код **sound**, но не компилируется — сначала **перестроить API** (`entry`, отдельные блоки, `split_at_mut` с доказанными границами); не полагаться на включение Polonius в проде без явной политики проекта.

## [TAG: 08-unsafe-and-ffi] Final Operating Principles

- Minimize unsafe. If you can express it safely at equal or slightly reduced performance, do so.
- Encapsulate unsafe. Safe API → unsafe implementation → private invariants at module level.
- Document every invariant. `# Safety` sections at function doc, `// SAFETY:` at blocks.
- Test with Miri. UB that slips past the compiler often slips past tests too; Miri catches most of it.
- Prefer standard library primitives. `Vec`, `Box`, `Arc`, `Mutex`, `OnceLock`, `MaybeUninit`, `NonNull` are expertly audited.
- Treat `mem::forget` as an adversary — your unsafe must be leak-safe.
- Treat `panic` as universal — any safe function call may panic and unwind.
- Treat compilers as adversaries — they WILL exploit any UB to misoptimize.
- Treat concurrency as an adversary — if the happens-before graph allows a reordering, assume it happens.
- Default memory ordering unclear? Use SeqCst. Optimize only with proof (Miri + Loom + code review).
- Match standard library patterns when possible (RawVec, Unique, NonNull, structural Pin projection, drop guards, leak amplification). They've been vetted across millions of LOC.
