# Cluster 07 — Embedded, WebAssembly, CLI, and Secure Rust (LLM knowledge base)

**Sources (collated; verify upstream before audit sign-off; snapshot refreshed 2026-04, ANSSI `print.html` where available):**

| Source | URL |
|--------|-----|
| Embedded Rust Book | https://docs.rust-embedded.org/book/ |
| Embedonomicon | https://docs.rust-embedded.org/embedonomicon/ |
| Rust and WebAssembly | https://rustwasm.github.io/docs/book/ |
| Command Line Applications in Rust | https://rust-cli.github.io/book/ |
| Secure Rust Guidelines (ANSSI) | https://anssi-fr.github.io/rust-guide/ |

**Scope note:** The ANSSI guide's public `print.html` snapshot used here includes Introduction, Development environment, Libraries, Naming, Integer operations, Error handling, Language guarantees, Unsafe Rust (generalities + memory + FFI), Standard library, and a **master Checklist** of rule IDs. Some topic areas mentioned in older ANSSI outlines (dedicated cryptography / input-validation chapters) may appear only as cross-cutting rules inside FFI, integers, and error handling in this revision—reconcile with the live site before audit sign-off.

**Consumer:** coding LLMs (Claude, Cursor). Dense rules, minimal prose, canonical examples.

---

# Appendix L — ANSSI rules: expanded operator notes (one card per ID)

The following subsections restate each checklist identifier with operational guidance for LLM application. Cite IDs in reviews.

## L.1 DENV-*

### DENV-STABLE
- **MUST** compile production security code with stable `rustc`/`cargo`, not nightly/beta defaults.
- **Pattern:** CI matrix uses `rustup default stable` and fails if `rustc --version` channel != stable unless job is explicitly `allow_nightly_tools`.

### DENV-TIERS
- **MUST** select Tier-1 `rustc` target triple for safety-critical deployments.
- **Anti-pattern:** verify current rustc book tier table before claiming production readiness for a specific triple.

### DENV-CARGO-LOCK
- **MUST** commit `Cargo.lock` for binaries and reproducible builds; reject PRs that delete lock without regeneration story.

### DENV-CARGO-OPTS
- **MUST NOT** set `overflow-checks = false` in dev/test profiles.

### DENV-CARGO-ENVVARS
- **MUST NOT** rely on ambient `RUSTFLAGS`—encode in `[build]`/`[profile]` or documented CI env with review.

### DENV-FORMAT
- **SHOULD** run `cargo fmt --check` in CI.

### DENV-LINTER
- **MUST** run `cargo clippy -- -D warnings` (or agreed policy) in CI.

### DENV-AUTOFIX
- **MUST** eyeball every `rustfix`/`clippy --fix` hunk—especially changes that alter control flow.

## L.2 LIBS-*

### LIBS-VETTING-DIRECT
- **MUST** record (e.g. spreadsheet/issue) why each direct dependency is acceptable.

### LIBS-VETTING-TRANSITIVE
- **SHOULD** spot-check high-risk transitive crates (crypto, `unsafe`, proc-macro).

### LIBS-OUTDATED / LIBS-AUDIT
- **MUST** automate in CI; failures need waiver with owner + expiry date.

## L.3 LANG-NAMING / LANG-ARITH
- **LANG-NAMING:** automated lint + API review for public items.
- **LANG-ARITH:** audit every `as` cast and `/` `%` on user-influenced integers.

## L.4 Error / panic family
- **LANG-ERRWRAP:** libraries export `thiserror` enums; map IO errors at boundary.
- **LANG-LIMIT-PANIC:** `panic!` only for "impossible if API contract holds" branches.
- **LANG-LIMIT-PANIC-SRC:** treat `unwrap` in tests `#[cfg(test)]` only—still dangerous in integration tests touching env.
- **LANG-ARRINDEXING:** prefer `slice.get(i)` when `i` is external.

## L.5 UNSAFE / MEM / FFI
- See Parts A.6–A.8 in main body—use these cards as PR checklist.

## L.6 STD traits
- **LANG-SYNC-TRAITS:** newtypes wrapping raw pointers must not derive `Send` unless proven.
- **LANG-CMP-INV:** manual `PartialEq` for security tokens may need constant-time compare—use `subtle` crate when comparing secrets (defense in depth; not explicit in ANSSI snapshot).
- **LANG-DROP-SEC:** call `zeroize` or explicit wipe in `Drop` **and** on error paths before panic.

---

# Appendix M — Embedded: PAC/HAL register patterns (generalized)

## M.1 svd2rust-style API
- `periph.register.read().field()` for read; `write(|w| {...})` for write—RMW is explicit.
- **Unsafe `bits()`:** only when datasheet says not all bit patterns are legal—wrap in safe module with validation.

## M.2 HAL typestate
- Example intent: `Serial<Enabled>` vs `Serial<Disabled>`—cannot `write` until `enable()` consumes state.
- **Pattern:** clocks `Clocks` token required to compute baud—cannot construct UART without PLL config.

## M.3 `embedded-hal` traits
- `digital::OutputPin`, `serial::Write`, `blocking::i2c::Write`—swap silicon by changing type parameters in board support.

## M.4 Interrupt service routine checklist
1. Minimal work in ISR—set flag or queue event.
2. No `alloc`, no `std::sync::Mutex` (use `critical_section` or appropriate mutex).
3. Clear peripheral interrupt flag before exit.
4. Document priority ceiling vs locks.

## M.5 `no_std` alloc pitfalls
- Heap allocator for Cortex-M—initialize heap region once; OOM hooks must match product policy.

---

# Appendix N — Embedonomicon: linker and boot deep notes

## N.1 EXTERN and KEEP
- **Problem:** vector table symbols not referenced from `Reset`—linker drops them.
- **Fix:** `EXTERN(RESET_VECTOR);` and `KEEP(*(.vector_table*))`.

## N.2 Stack pointer symbol
- Linker computes stack at RAM end—must match CPU full-descending stack model.

## N.3 Custom target JSON
- For unsupported chips, JSON specifies `llvm-target`, `data-layout`, `arch`, `panic-strategy`—must align with LLVM + debugger config.

## N.4 build.rs snippet pattern

```rust
println!("cargo:rustc-link-arg=-Tlink.x");
println!("cargo:rustc-link-search={}", out_dir);
```

## N.5 global_asm for reset
- Assembly calling `Reset` after data load—do not duplicate `Reset` symbol—ensure single entry.

---

# Appendix O — WebAssembly: JS boundary and copies

## O.1 String copies
- Every `String` returned to JS allocates wasm and JS copies—prefer pointer + length to UTF-8 in linear memory with wasm-bindgen unsafe view patterns.

## O.2 wasm_bindgen module paths
- Organize imports; tree-shake unused JS.

## O.3 serde-wasm-bindgen vs JSON
- Binary protocols smaller than JSON stringification—measure.

## O.4 wasm-bindgen-test
- Headless browser tests—use for DOM; keep pure tests native.

---

# Appendix P — CLI: human communication and pipes

## P.1 stdout vs stderr
- Structured data (grep-like matches) to stdout; diagnostics to stderr—enables shell pipelines.

## P.2 Exit codes
- Document mapping in README—`1` generic failure; `sysexits` for categories.

## P.3 tracing spans
- Use `tracing::instrument` on subcommands for structured logs.

## P.4 Config precedence
- CLI flags greater than env greater than file greater than defaults—document order.

---

# Appendix Q — Additional anti-patterns (collated)

1. Using `todo!` in production paths—compiles but panics—treat as `panic!`.
2. `dbg!` left in release—leaks data in stderr.
3. `serde` without `deny_unknown_fields` for security configs—can ignore misspelled keys.
4. Glob imports in FFI modules—hide unsafe `extern` declarations—use explicit paths.
5. Assuming little-endian in wire formats—specify endianness in protocol layer.

---

# Appendix R — Tooling: Miri, geiger, deny (supply chain)

These are not enumerated as mandatory in the ANSSI print snapshot but align with DENV and LIBS intent:

- `cargo miri test` — detects UB in unsafe code paths when run under Miri.
- `cargo geiger` — counts unsafe lines—track regressions.
- `cargo deny check` — license bans plus advisory aggregation (complements cargo-audit).

---

# Appendix S — Example: serde in security-sensitive configs

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
```

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    token_path: PathBuf,
}
```

- **Pattern:** `deny_unknown_fields` prevents silent misconfiguration.

---

# Appendix T — cortex-m / nix quick map

| Need | Crate |
|------|-------|
| NVIC enable/disable | `cortex-m::peripheral::NVIC` |
| Baseline delay | `cortex_m::asm::nop` loops or `systick` |
| SysTick | `cortex_m::peripheral::SYST` |
| Unix syscalls | `nix` safe wrappers over `libc` |

---

# Appendix U — 100 micro-rules (flashcards)

01. Stable toolchain for secure builds.
02. Lockfile in git.
03. No overflow-check override in dev.
04. No env rustc override in CI without audit.
05. Clippy clean.
06. Verify autofix.
07. Vet deps.
08. cargo-audit green.
09. cargo-outdated reviewed.
10. Naming RFC430.
11. checked_add for counts.
12. No unwrap on argv.
13. No unwrap on network.
14. Result must propagate or log.
15. Library typed errors.
16. Panic only on contract break.
17. assert only invariants.
18. get() for slices.
19. abort panic for fail-fast safety.
20. catch_unwind at FFI edge.
21. panic_handler on no_std.
22. Zero UB.
23. forbid unsafe unless justified.
24. encapsulate unsafe.
25. no mem::forget.
26. no Box::leak.
27. ManuallyDrop finalize.
28. no raw escape without pairing.
29. MaybeUninit justified.
30. repr(C) on FFI structs.
31. bindgen for headers.
32. c_char not i8 blindly.
33. Validate bool from C.
34. Check pointers for null.
35. No Rust enum from foreign.
36. Option NonNull niches.
37. Mark fn ptrs unsafe extern.
38. Check callbacks.
39. Opaque struct pointers.
40. No aliasing mut via FFI.
41. Single owner of allocation.
42. Drop glue for foreign pointers.
43. No Drop by value across FFI.
44. Send Sync justified.
45. Cmp laws hold.
46. derive cmp when possible.
47. Drop never panics.
48. no rc cycles.
49. secrets not only in Drop.
50. no static mut races.
51. PAC singleton take.
52. HAL constrain clocks.
53. ISR short.
54. clear IRQ flags.
55. wasm LTO.
56. wasm-opt.
57. avoid allocator in wasm hot path.
58. twiggy watch.
59. assert_cmd integration.
60. proptest parsers.
61. fuzz file inputs.
62. exitcode sysexits.
63. human-panic UX.
64. tracing for servers.
65. signal-hook unix.
66. ctrlc portable.
67. confy simple config.
68. cbindgen export.
69. bindgen import.
70. wee_alloc feature.
71. console_error_panic_hook.
72. wasm-bindgen-test DOM.
73. native unit tests first.
74. cargo-deny policy.
75. cargo-geiger budget.
76. miri unsafe PRs.
77. no serde unknown fields on configs.
78. stderr diagnostics.
79. stdout machine readable.
80. document exit codes.
81. README install paths.
82. musl static linux bin.
83. GitHub releases artifacts.
84. completions from clap.
85. man pages generated.
86. tier1 target only safety crit.
87. reproducible builds with lock.
88. no nightly default.
89. rustfmt CI.
90. review transitive crypto.
91. zeroize on error paths.
92. no dbg release.
93. no todo in prod.
94. endian explicit.
95. FFI docs for safety.
96. panic strategy documented.
97. QEMU embedded tests.
98. probe-rs automation.
99. OpenOCD fallback.
100. Re-read ANSSI checklist quarterly.

---
# Appendix V — ANSSI IDs line repetition (drill)

Earlier versions listed the same drill sentence hundreds of times. **One repetition is enough:** walk the ANSSI master checklist rule IDs against the diff (see Appendix L for ID semantics).

---
# Appendix W — Cross-domain numbered examples (01–120)
001. no_std: use core::fmt::Write for UART logger
002. no_std: avoid HashMap default hasher
003. PAC: one Peripherals::take owner
004. HAL: constrain peripheral before use
005. ISR: defer work to main loop
006. critical_section: protect shared static
007. linker: KEEP vector table
008. linker: ENTRY Reset
009. panic abort embedded firmware
010. catch_unwind FFI export
011. repr C struct FFI
012. Option unsafe extern fn callback
013. validate C bool as u8 then map
014. no enum from C int without match
015. wasm: wasm-pack build pipeline
016. wasm: wasm-opt -Oz post
017. wasm: twiggy top functions
018. wasm: avoid format in hot loop
019. wasm-bindgen: import console
020. js-sys: Object in wasm
021. web-sys: canvas get_context
022. wee_alloc behind feature
023. no allocator static universe
024. linear memory view from JS
025. CLI: clap derive Parser
026. CLI: main returns Result
027. CLI: exitcode crate
028. CLI: assert_cmd success stdout
029. CLI: tempfile for integration
030. CLI: stderr for errors
031. CLI: tracing levels
032. signal ctrlc handler flag
033. signal-hook stream
034. confy load path
035. cargo publish metadata
036. binary release musl
037. cross docker builds
038. ANSSI: stable toolchain
039. ANSSI: lockfile
040. ANSSI: no rustflags env
041. ANSSI: clippy
042. ANSSI: audit
043. ANSSI: outdated
044. ANSSI: naming
045. ANSSI: checked math
046. ANSSI: no unwrap
047. ANSSI: Result must use
048. ANSSI: no UB
049. ANSSI: encapsulate unsafe
050. ANSSI: no forget
051. ANSSI: no leak
052. ANSSI: ManuallyDrop
053. ANSSI: raw pointer pairing
054. ANSSI: MaybeUninit
055. ANSSI: FFI C types
056. ANSSI: FFI consistent
057. ANSSI: bindgen
058. ANSSI: c_int portability
059. ANSSI: validate pointers
060. ANSSI: validate fn ptr
061. ANSSI: no Rust enum FFI
062. ANSSI: opaque types
063. ANSSI: aliasing model
064. ANSSI: ownership single
065. ANSSI: Drop FFI rules
066. ANSSI: panic FFI
067. ANSSI: C API export
068. ANSSI: Send Sync
069. ANSSI: comparison laws
070. ANSSI: derive cmp
071. ANSSI: Drop panic free
072. ANSSI: no rc cycle
073. ANSSI: secret wipe not drop only
074. Supply chain: cargo deny
075. Supply chain: cargo geiger
076. Supply chain: miri
077. Fuzz: cargo fuzz parsers
078. Proptest: CLI args
079. Serde: deny unknown fields
080. Logging: stderr only errors user
081. Docs: man page ship
082. Shell: completions ship
083. CI: fmt check
084. CI: test matrix
085. CI: audit job
086. Review: ANSSI checklist link
087. Review: threat model
088. Review: data classification
089. Crypto: use audited crate
090. Crypto: zeroize on err
091. Network: timeout all IO
092. Network: TLS verify certs
093. File: path canonicalize cautiously
094. Env: no secrets in argv
095. Env: load dotenv carefully
096. Process: min privileges
097. Container: read-only root fs
098. Wasm: CSP headers
099. Browser: sanitize DOM output
100. Embedded: WDT feed
101. Embedded: MPU if available
102. FFI: document thread safety
103. FFI: document reentrancy
104. Concurrency: Arc for shared
105. Concurrency: Mutex poisoning policy


## Taxonomy map

| Tag | Focus |
|-----|-------|
| `02-language-rules` | `#![no_std]`, `panic_handler`, profiles, editions |
| `03-idioms` | CLI patterns, wasm-bindgen, embedded HAL usage |
| `04-design-patterns` | HAL/PAC layering, interrupt-safe design, FFI layering |
| `05-anti-patterns` | Security mistakes (mostly ANSSI) |
| `06-error-handling` | `Result`, panics, CLI exit codes |
| `08-unsafe-and-ffi` | ANSSI FFI + Embedonomicon low-level |
| `09-performance` | wasm size, no-alloc, LTO |
| `10-testing-and-tooling` | `assert_cmd`, audits, fuzzing hooks |
| `11-ecosystem-crate-picks` | Curated crates |

---

# Part A — ANSSI Secure Rust Guidelines (rulebook)

## A.0 Notation and philosophy

- **must** in ANSSI is stricter than “it is recommended” elsewhere.
- Multiple solutions may exist with different security levels; pick the strongest your context allows.
- **Async Rust** is explicitly out of scope in the guide’s preface—do not infer async rules from ANSSI.
- Re-assess applicability periodically (risk management process).

## A.1 Development environment

### Rustup

- Downloads use HTTPS; **signature validation / pinning against downgrade are still WIP**—alternative install methods may be preferable for high-threat models.
- **Editions:** no edition is mandated; follow recommendations for *features* you use.
- **Channels:** secure application development **MUST** use a **fully stable** toolchain (`DENV-STABLE`).
- For nightly-only tools (`rustfmt`, etc.), invoke **`rustup run nightly cargo fmt`** or **`cargo +nightly fmt`**—do not switch the default workspace toolchain to nightly.
- Check **`rustup override list`**—overrides can silently change behavior per directory.

### Target tiers (`DENV-TIERS`)

- **Tier 1:** full tests, regression testing, stable ABI expectations—“guaranteed to work.”
- **Tier 2:** builds; less testing; may break.
- **Tier 3:** unsupported.
- **Safety-critical systems MUST use Tier 1 targets and certified toolchains.**

### Cargo & lockfiles (`DENV-CARGO-LOCK`)

- **`Cargo.lock` MUST be tracked in VCS**—checksum mismatches fail the build (TOFU on first resolve).
- First download security still depends on crates.io / index integrity—consider mirrors and policies consciously.

### Profiles (`DENV-CARGO-OPTS`)

- **`debug-assertions` and `overflow-checks` MUST NOT be disabled/overridden** in **`[profile.dev]`** and **`[profile.test]`**—silent loss of checks is a security bug.

### Environment (`DENV-CARGO-ENVVARS`)

- **`RUSTC`, `RUSTC_WRAPPER`, `RUSTFLAGS` MUST NOT be overridden** when building—prefer centralized flags in `Cargo.toml`; use build scripts instead of opaque wrappers for reproducibility.

### Formatting & linting

- **`rustfmt` SHOULD** be used (`DENV-FORMAT`).
- **`clippy` MUST** be used regularly (`DENV-LINTER`).
- **`cargo fix` / `clippy --fix` / `rustfix`:** automatic fixes **MUST** be manually verified (`DENV-AUTOFIX`)—edition-idiom fixes can change semantics or break builds.

### Supply chain tools mentioned

- **`cargo-outdated` MUST** be used; outdated deps **SHOULD** be updated or **MUST** be justified (`LIBS-OUTDATED`).
- **`cargo-audit` MUST** be used against RustSec (`LIBS-AUDIT`).

## A.2 Libraries & naming

- **Each direct third-party dependency MUST be validated; validation MUST be tracked** (`LIBS-VETTING-DIRECT`).
- **Transitive dependencies SHOULD** be validated individually (`LIBS-VETTING-TRANSITIVE`).
- **Naming MUST** follow Rust API Guidelines (`LANG-NAMING`)—compiler enforces `nonstandard_style`; Clippy `style` helps with `C-CONV`, etc.

## A.3 Integer arithmetic (`LANG-ARITH`)

- **Debug:** overflow panics. **Release:** unchecked wrap **unless** `overflow-checks = true` in profile.
- **Rule:** when overflow is possible, **do not** use bare `+ - * /`—use `checked_*`, `saturating_*`, `wrapping_*`, `overflowing_*`, or `Wrapping<T>` / `Saturating<T>` intentionally.

```rust
let sum = a.checked_add(b).ok_or(Error::Overflow)?;
```

## A.4 Errors & panics

- **`Result` must never be ignored.**
- Library **`Error` types MUST** implement `Error + Send + Sync + 'static` + `Display`; be **exception-safe** (RFC 1236).
- **`anyhow`-style erasure SHOULD NOT** be used in **libraries**—prefer `thiserror` / `snafu` for typed errors (`LANG-ERRWRAP` recommendation).
- **Functions MUST NOT panic unless preconditions are violated** (`LANG-LIMIT-PANIC`).
- **`unwrap` / `expect` / `assert!` MUST** only appear where the spec **forbids** the error case (`LANG-LIMIT-PANIC-SRC`).
- **Array indexing:** test bounds or use `.get()` (`LANG-ARRINDEXING`).
- **Safety-critical “fail-fast”:** `panic = 'abort'` in `[profile.release]` may be required so a redundant system can take over.
- **FFI:** panics across language boundaries require `catch_unwind` / `panic_handler` discipline—see Part A.7.

## A.5 Language guarantees & UB (`UNSAFE-NOUB`)

- **No undefined behavior.**
- Safe Rust avoids UB **if no `unsafe`**, but still allows **data races**, **leaks**, **numeric surprises** (overflow in release), **logic bugs**.
- Distinguish **UB** from **panic** (`unwrap` on `None` is defined panic, not UB).

## A.6 Unsafe generalities (`LANG-UNSAFE`, `LANG-UNSAFE-ENCP`)

- **`unsafe` marks API contracts** where caller must uphold invariants; **`unsafe` blocks** take compiler’s responsibility.
- **2024+:** `unsafe` needed for some `extern` blocks and attributes—see current Reference.
- Prefer **`#![forbid(unsafe_code)]`** at crate root **unless** FFI, MMIO, or measured hot-path need justify it.
- **Unsafe MUST be encapsulated:** safe API cannot expose UB; or unsafe API documents **all** preconditions.
- **Type invariants** (e.g. custom `Vec`) must not be breakable through safe methods—otherwise mark `unsafe` or hide the method.

## A.7 Memory (`MEM-*`)

- **No memory leaks** (`MEM-NO-LEAK`).
- **`mem::forget` MUST NOT** be used (`MEM-FORGET`); use `deny(clippy::mem_forget)` / `MEM-FORGET-LINT`.
- **`Box::leak` MUST NOT** (`MEM-LEAK`).
- **`ManuallyDrop` MUST** be finalized (`into_inner` or `drop`) (`MEM-MANUALLYDROP`).
- **Avoid converting smart pointers to raw** in safe code (`MEM-NORAWPOINTER`); if used, document+justify.
- **`from_raw` pairing:** every `into_raw` **MUST** eventually pair with `from_raw` **only** on that pointer (`MEM-INTOFROMRAWALWAYS`, `MEM-INTOFROMRAWONLY`).
- **`mem::uninitialized` MUST NOT**; **`MaybeUninit` MUST** be justified (`MEM-UNINIT`).

## A.8 Foreign Function Interface (complete rule list)

**Structure**

- Split **`-sys` low-level** `extern` blocks from **safe wrapper** modules (`FFI-SAFEWRAPPING`).

**Types**

- **Only `repr(C)` / documented opaque / robust primitives** at boundaries (`FFI-CTYPE`).
- **Layouts MUST match** C side (`FFI-TCONS`); use **bindgen/cbindgen** per target (`FFI-AUTOMATE`).
- **Platform C types → `core::ffi::c_*` or `libc`** (`FFI-PFTYPE`).

**Robust vs “non-robust” values (`FFI-CKNONROBUST`)**

- Non-robust: `bool`, references, fn pointers, enums, floats, composite with such fields.
- **Never trust unchecked foreign non-robust values**—validate or receive guarantees from producer.
- **Prefer validating in Rust** (`FFI-CKINRUST`).

**Pointers & references**

- **Dereference only after null/range/alignment checks** (`FFI-CK-PTR-VALID`).
- **Encode foreign pointers as raw `*const T`/`*mut T`** when possible (`FFI-INPUT-PTR`).
- **Foreign `&T`/`&mut T` MUST** be validated on foreign side if not opaque (`FFI-CK-INPUT-REF-VALID`).

**Function pointers**

- **Mark `extern "C" fn` + `unsafe` in types** (`FFI-MARKEDFUNPTR`).
- **Check every callback** at boundary (`FFI-CKFUNPTR`).

**Enums**

- **Do not pass Rust `enum` over FFI**—use integers + checked conversion (`FFI-NOENUM`); exceptions: opaque-to-C or C++ `enum class` with verified ABI.

**Opaque types**

- Prefer **dedicated ZST / incomplete struct** over `*mut c_void` (`FFI-R-OPAQUE`, `FFI-C-OPAQUE`).

**Memory model / aliasing (`FFI-CK-REF-MODEL`)**

- Example UB: C calls `swap(&a, &a)` on Rust `swap` that builds `&mut` from raw pointers—violates uniqueness.
- Example UB: pass pointer to `const` value to C that mutates—violates Rust mutability.

**Allocation ownership (`FFI-MEM-NODROP`, `FFI-MEM-OWNER`, `FFI-MEM-WRAPPING`)**

- **Do not implement `Drop` on types passed by value across FFI.**
- **Single language allocates + frees** a given object; other side uses **exported ctor/dtor** pairs.
- **Foreign allocations → Rust `Drop` wrappers** that call foreign free (`FFI-MEM-WRAPPING`).
- Sensitive wiping **cannot rely on `Drop` alone** if panics possible—use `panic_handler` / abort strategies.

**Panics (`FFI-NOPANIC`, `no_std`)**

- **`catch_unwind` does not catch abort-on-panic.**
- **`no_std`:** must supply **`#[panic_handler]`**—implement carefully; consider `panic-never` / `no-panic` style link-time enforcement for critical firmware.

**Library export (`FFI-CAPI`)**

- Expose **Rust to other languages** only through **stable C-compatible surface**; **cbindgen** for headers.

## A.9 Standard library traits

- **`Send`/`Sync` manual impls MUST** be justified (`LANG-SYNC-TRAITS`).
- **`PartialEq`/`Ord`/… MUST** meet documented algebraic laws (`LANG-CMP-INV`); **derive** when structural equality works (`LANG-CMP-DERIVE`); prefer **not** overriding defaulted methods wrongly (`LANG-CMP-DEFAULTS`).
- **`Drop` MUST** be justified (`LANG-DROP`), **must not panic** (`LANG-DROP-NO-PANIC`), **must not** form **Rc cycles** with interior mutability (`LANG-DROP-NO-CYCLE`, `MEM-MUT-REC-RC`).
- **Security-critical cleanup** (crypto zeroize) **MUST NOT** rely **only** on `Drop` (`LANG-DROP-SEC`).

---

## A.10 ANSSI checklist — canonical ID list (every rule)

| ID | Kind | Summary |
|----|------|---------|
| DENV-STABLE | Rule | Stable toolchain only |
| DENV-TIERS | Rule | Tier 1 + certified for safety-critical |
| DENV-CARGO-LOCK | Rule | Track `Cargo.lock` in VCS |
| DENV-CARGO-OPTS | Rule | Do not override dev/test debug-assertions/overflow-checks |
| DENV-CARGO-ENVVARS | Rule | Do not override `RUSTC`/`RUSTC_WRAPPER`/`RUSTFLAGS` |
| DENV-FORMAT | Rec | Use `rustfmt` |
| DENV-LINTER | Rule | Use `clippy` regularly |
| DENV-AUTOFIX | Rule | Verify automatic fixes |
| LIBS-VETTING-DIRECT | Rule | Validate + track direct deps |
| LIBS-VETTING-TRANSITIVE | Rec | Validate transitive deps |
| LIBS-OUTDATED | Rule | Use `cargo-outdated`; update or justify |
| LIBS-AUDIT | Rule | Use `cargo-audit` |
| LANG-NAMING | Rule | Rust API Guidelines naming |
| LANG-ARITH | Rule | Explicit overflow semantics |
| LANG-ERRWRAP | Rec | Custom error wrapping all cases |
| LANG-LIMIT-PANIC | Rule | No panic unless contract violation |
| LANG-LIMIT-PANIC-SRC | Rule | Restrict `unwrap`/`expect`/`assert!` |
| LANG-ARRINDEXING | Rule | Bounds or `.get()` |
| UNSAFE-NOUB | Rule | Zero UB |
| LANG-UNSAFE | Rule | Avoid `unsafe` / justify |
| LANG-UNSAFE-ENCP | Rule | Encapsulate unsafe |
| MEM-NO-LEAK | Rule | No leaks |
| MEM-FORGET | Rule | No `mem::forget` |
| MEM-FORGET-LINT | Rec | `clippy::mem_forget` |
| MEM-LEAK | Rule | No `Box::leak` |
| MEM-MANUALLYDROP | Rule | Finalize `ManuallyDrop` |
| MEM-NORAWPOINTER | Rule | No casual `into_raw` in safe code |
| MEM-INTOFROMRAWALWAYS | Rule | Pair `into_raw` → `from_raw` |
| MEM-INTOFROMRAWONLY | Rule | `from_raw` only on `into_raw` values |
| MEM-UNINIT | Rule | No `uninitialized`; justify `MaybeUninit` |
| FFI-SAFEWRAPPING | Rec | Safe wrapper layer |
| FFI-CTYPE | Rule | C-compatible types only |
| FFI-TCONS | Rule | Consistent types both sides |
| FFI-AUTOMATE | Rec | Binding generators |
| FFI-PFTYPE | Rule | Portable `c_*` aliases |
| FFI-CKNONROBUST | Rule | Validate non-robust values |
| FFI-CKINRUST | Rec | Validate in Rust when possible |
| FFI-CK-PTR-VALID | Rule | Check foreign pointers |
| FFI-INPUT-PTR | Rec | Raw pointers for foreign pointers |
| FFI-CK-INPUT-REF-VALID | Rule | Foreign references validated |
| FFI-MARKEDFUNPTR | Rule | `extern`+`unsafe` fn ptr types |
| FFI-CKFUNPTR | Rule | Check function pointers |
| FFI-NOENUM | Rule | No Rust `enum` at boundary |
| FFI-R-OPAQUE | Rec | Dedicated opaque structs |
| FFI-C-OPAQUE | Rec | Incomplete C structs for Rust opaques |
| FFI-CK-REF-MODEL | Rule | Preserve aliasing/mutability model |
| FFI-MEM-NODROP | Rule | No `Drop` types by value across FFI |
| FFI-MEM-OWNER | Rule | Single allocator per object |
| FFI-MEM-WRAPPING | Rec | `Drop` wrappers for foreign allocs |
| FFI-NOPANIC | Rec | No panics across FFI / use `catch_unwind` |
| FFI-CAPI | Rule | Export only C-compatible API |
| LANG-SYNC-TRAITS | Rule | Justify `Send`/`Sync` |
| LANG-CMP-INV | Rule | Comparison trait laws |
| LANG-CMP-DEFAULTS | Rec | Minimal manual overrides |
| LANG-CMP-DERIVE | Rec | Derive comparisons when structural |
| LANG-DROP | Rule | Justify `Drop` |
| LANG-DROP-NO-PANIC | Rule | No panic in `Drop` |
| LANG-DROP-NO-CYCLE | Rule | No `Rc`/`Arc` cycles with interior mut |
| LANG-DROP-SEC | Rule | Don’t rely only on `Drop` for secrets |
| MEM-MUT-REC-RC | Rule | No interior mut + recursive `Rc` |

---

# Part B — Embedded Rust Book (generalized patterns)

## B.1 Hosted vs bare metal (`02-language-rules`)

- **Hosted:** POSIX-like OS, `std`, filesystem, threads—feels like constrained desktop.
- **Bare metal:** firmware/kernel/bootloader—**no `std`**, only **`core`** (+ optional **`alloc`**).
- **`std` runtime** sets stack probes, parses args, main thread—**absent** in bare metal; you own boot order, stack pointer, vector table.

## B.2 `no_std` capability matrix

| Feature | `no_std` | `std` |
|---------|----------|-------|
| Heap | Only with `alloc` + global allocator | Yes |
| `Vec`/`BTreeMap` | With `alloc` | Yes |
| `HashMap`/`HashSet` | **No** (needs secure RNG for default hasher) | Yes |
| Stack overflow protection | No (unless you build it) | Yes |
| `libcore` | Yes | Yes |

## B.3 Memory-mapped I/O & crate stack (`04-design-patterns`, `11-ecosystem`)

1. **Micro-architecture** (e.g. `cortex-m`): core interrupts, SysTick—shared by all chips of that core.
2. **PAC** (Peripheral Access Crate): register-level, generated (e.g. `svd2rust`)—**read/modify/write** with closure APIs; **some accessors `unsafe`** when SVD is ambiguous.
3. **HAL**: implements **`embedded-hal`** traits; **`split()`/`constrain()`** consumes PAC structs to enforce ordering (configure clocks before UART).
4. **Board crate**: opinionated pin wiring—skip in portable code.

**Pattern — single ownership of peripherals:** `Peripherals::take()` returns `Option`—only one handle; prevents accidental aliasing of entire peripheral set.

## B.4 Interrupts & preemption (`04-design-patterns`, `05-anti-patterns`)

- **`static mut` in handlers is non-reentrant**—UB if main + ISR or nested ISRs conflict—use **`critical_section`**, **`Mutex<core::cell::RefCell<...>>`** (cortex-m), or message passing.
- Clear interrupt source flags to avoid infinite re-entry.
- Use device crate’s **`interrupt` attribute** re-export (not raw `cortex-m-rt::interrupt` without device features)—vector table must match silicon.

## B.5 Panic strategies (`02-language-rules`)

- **`panic-halt`**, **`panic-abort`**, **`panic-itm`** (instrumentation), etc.—choose explicit behavior.
- For **hosted tests on QEMU**, `panic = exit(EXIT_FAILURE)` pattern enables run-pass tests.

## B.6 Cross compilation

- `thumbv*-*-eabi*` targets; **`rustup target add`**; **`cargo-binutils`** (`llvm-objdump`, `nm`, `size`) for inspection.

---

# Part C — Embedonomicon (low-level / `no_std` / FFI-adjacent)

## C.1 Smallest `no_std` binary (`02-language-rules`)

```rust
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {}
}
```

- **`#![no_main]`** because Rust’s `main` ABI assumes runtime services.
- **`#[panic_handler]`** is **mandatory**—defines **all** panics (including language panics).

## C.2 `eh_personality` vs `panic = "abort"`

- If target does not abort-on-panic, you need **`[profile.*] panic = "abort"`** or nightly **`eh_personality`** lang item—avoid unwinding on bare metal.

## C.3 Linker control (`08-unsafe-and-ffi`)

- **Sections:** `.text`, `.rodata`, `.data`, `.bss`, custom `.vector_table`.
- **Stable symbols:** `#[unsafe(no_mangle)]`, `#[unsafe(export_name = "...")]`, `#[unsafe(link_section = ".section")]`.
- **Safety:** duplicate `no_mangle` symbols → **UB**; wrong section → executing data or vice versa.
- **Linker script `MEMORY`/`SECTIONS`:** place vector table at flash base; **`ENTRY(Reset)`** keeps linker from GC’ing roots; **`EXTERN` + `KEEP`** for vector symbols not reached by call graph.
- **Stack:** full-descending—initial SP from end of RAM region.

## C.4 Reset vector & `extern "C"`

- Hardware expects **C ABI** for reset handler and function pointers in vector table—**Rust ABI is unstable**.

## C.5 `.data` / `.bss` initialization

- Prefer **assembly** (`global_asm!`) for reset routine that copies `.data` and zeroes `.bss`—pure Rust init raised soundness concerns historically.

## C.6 `global_asm!` vs `asm!`

- Stable since **1.59**—use for vector stubs, early boot trampoline, careful memory operations.

## C.7 Build scripts

- Emit **`cargo:rustc-link-arg=-Tlink.x`** and **`cargo:rustc-link-search`** so dependent crates find linker scripts.

---

# Part D — Rust and WebAssembly

## D.1 Why Rust + wasm (`09-performance`)

- **Small binary:** no GC runtime bundled; pay for what you import.
- **Deterministic perf:** no JS GC pauses in Rust code.
- **Interop:** ECMAScript modules, npm ecosystem.

## D.2 Machine model (`03-idioms`)

- **Single linear memory** (64KiB page growth; no shrink).
- **wasm-bindgen** generates JS glue + typed `.d.ts`.

## D.3 `wasm-pack` pipeline (`10-testing-and-tooling`)

1. `rustc` → `.wasm`
2. `wasm-bindgen` CLI → JS + TS defs + package.json
3. Optional **`wasm-opt -Oz`** (binaryen) for size/speed

## D.4 Size optimization (`09-performance`)

- **`lto = true`**, **`opt-level = "z"`** in `Cargo.toml`
- **`wasm-opt -Oz`**
- **`wasm-snip`** to remove panic strings (measure carefully—debuggability vs size)
- **`wee_alloc`** (~1KiB) when allocator needed; template **feature-gated**
- **Eliminate allocator:** static `Universe`, `FixedBitSet`, expose pointers into linear memory for JS to read directly—**avoid copying** via `String` bridges
- **`twiggy`** for retained-size profiling

## D.5 Debugging (`10-testing-and-tooling`)

- Debug **DWARF for wasm** still immature—often step through **wasm instructions**, not Rust lines.
- **`console_error_panic_hook`** for readable panics in browser console
- **`web_sys::console::log_*`** for tracing
- **Isolate logic:** pure Rust `#[test]` on host for algorithm bugs; **`wasm-bindgen-test`** for JS/DOM interactions

## D.6 `wasm-bindgen` patterns (`03-idioms`, `11-ecosystem`)

```rust
#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn greet(name: &str) {
    alert(&format!("Hello, {}!", name));
}
```

- **`js-sys`**: JS builtins
- **`web-sys`**: Web APIs
- **`wasm-bindgen-futures`**: Promise ↔ Future

---

# Part E — Command Line Applications in Rust

## E.1 Argument modeling (`03-idioms`)

- Treat CLI args as **`struct Cli`** — `clap` **derive** with doc comments → `--help` & validation.

```rust
#[derive(Parser)]
struct Cli {
    pattern: String,
    path: PathBuf,
}
```

## E.2 Error handling (`06-error-handling`)

- **`main` → `Result<(), E>`** with **`?`**
- Libraries: **typed errors**; binaries: **`anyhow`** or **`color-eyre`** for context chains
- Replace **`.expect` on files** with user-facing `Err`—ANSSI forbids casual unwrap/expect

## E.3 Exit codes (`06-error-handling`)

- **`0` success**; **`101` Rust panic**; common failures **often `1`**
- **`exitcode` crate** maps to BSD **`sysexits.h`** conventions (`CONFIG`, `DATAERR`, …)

## E.4 Testing (`10-testing-and-tooling`)

- **Unit:** extract `find_matches<W: Write>(pattern, contents, &mut W)`—test with **`Vec<u8>`**
- **Integration:** `tests/cli.rs` with **`assert_cmd` + `predicates` + `assert_fs::NamedTempFile`**
- **`Command::cargo_bin("grrs")`** compiles binary once per test run
- **Don’t snapshot clap `--help` verbatim**—assert presence of keywords only
- **Property tests:** `proptest` for parsers; **fuzz** binary inputs (`cargo-fuzz`)

## E.5 Human-facing output (`03-idioms`)

- **`log` + `env_logger`/`tracing-subscriber`** with levels; **`RUST_LOG`**
- **`human-panic`** for friendly crash reports in production CLIs
- Progress output: structured steps (see wasm-pack style)

## E.6 Signals & config (`11-ecosystem`)

- **`ctrlc`** cross-platform; **`signal-hook`** for Unix breadth; **`tokio` + signal-hook`**
- **`confy`** for simple `Serialize` config load; respect XDG dirs

## E.7 Packaging (`10-testing-and-tooling`)

- **`cargo publish`** for Rust devs; **binary releases** on GitHub for end users; **musl** static Linux builds; **`cross`** for cross-compilation
- **`cargo-deb` / `cargo-aur` / Homebrew formulas** for OS integrators
- Ship **man pages + shell completions**—**clap** can generate

---

# Part F — Security anti-patterns catalog (`05-anti-patterns`)

Cross-reference ANSSI IDs; add domain-specific ones:

| Anti-pattern | Why it hurts | ANSSI / note |
|--------------|--------------|--------------|
| `unwrap`/`expect` on external input | Panic → DoS or info leak via stack trace | LANG-LIMIT-PANIC-SRC |
| Release overflow on security counters | Silent wrap → logic bypass | LANG-ARITH |
| `mem::forget` on secrets | Keys remain in RAM | MEM-FORGET |
| Passing `&mut` to C that aliases | UB, optimization breaks | FFI-CK-REF-MODEL |
| Trusting C `enum` bits as Rust `enum` | Invalid discriminant → UB | FFI-NOENUM |
| `Rc<RefCell<>>` cycles + `Drop` | Leak + resource exhaustion | MEM-MUT-REC-RC |
| `Drop` only zeroization | Panic skips drop | LANG-DROP-SEC |
| `static mut` ISR + main | Data race | Embedded book |
| Oversized wasm `String` shuffling | Copies + binary bloat | wasm book |
| Ignoring `cargo-audit` | Known CVEs | LIBS-AUDIT |

---

# Part G — Performance synthesis (`09-performance`)

| Domain | Tactic |
|--------|--------|
| wasm | LTO + `opt-level=z` + `wasm-opt` + remove allocator |
| embedded | Fixed buffers, `heapless`, ISR-friendly lock-free queues |
| CLI | mmap readers (`memmap2`), streaming, avoid `read_to_string` on huge files |

---

# Part H — Testing & tooling matrix (`10-testing-and-tooling`)

| Tool | Role |
|------|------|
| `cargo test` / `assert_cmd` | CLI integration |
| `wasm-bindgen-test` | Browser/JS boundary |
| `cargo-audit` | vuln DB |
| `cargo-outdated` | drift |
| `cargo-deny` | policy (licenses, bans) — **not in ANSSI text but common in secure pipelines** |
| `cargo-geiger` | count `unsafe` — **supply chain visibility** |
| `miri` | UB detection in `unsafe`/`unsafe`-heavy libs — **recommended for `unsafe` audits** |
| `twiggy` / `wasm-opt` | wasm size |
| QEMU + `openocd`/`probe-rs` | embedded CI |

---

# Part I — Ecosystem crate picks (`11-ecosystem`)

| Crate | Use |
|-------|-----|
| `clap` | CLI parsing (derive) |
| `clap_complete` | shell completions |
| `anyhow` / `color-eyre` | binary error reporting |
| `thiserror` | library errors |
| `human-panic` | friendly panics |
| `tracing` / `log` | observability |
| `assert_cmd` / `predicates` / `assert_fs` | CLI tests |
| `exitcode` | portable exit reasons |
| `wasm-bindgen` / `js-sys` / `web-sys` | wasm interop |
| `wee_alloc` | tiny wasm allocator |
| `serde` / `serde_json` | config & data (with `no_std` feature flags as needed) |
| `cortex-m` / `cortex-m-rt` | ARM Cortex-M bare metal |
| `embedded-hal` | trait portability |
| `nix` / `libc` | Unix FFI & syscalls |
| `bindgen` / `cbindgen` | FFI generation |

---

# Part J — Quick-reference code snippets

## J.1 FFI: nullable pointer → `Option<&mut T>`

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_in_place(a: *mut u32, b: u32) {
    if let Some(a) = unsafe { a.as_mut() } {
        *a += b;
    }
}
```

## J.2 CLI: infallible `main` pattern

```rust
fn main() -> anyhow::Result<()> {
    run()
}
```

## J.3 wasm: feature-gated wee alloc

```toml
[features]
default = ["wee_alloc"]
```

```rust
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
```

## J.4 Embedded: `critical_section` pattern (conceptual)

- Protect shared `static` state between ISR and main using the HAL/BSP’s mutex or `critical_section::with`—never raw `static mut` without synchronization.

---

# Part K — Revision checklist for LLM consumers

Before suggesting merges to security-sensitive code:

1. Enumerate **ANSSI IDs** touched; cite MUST vs SHOULD.
2. For **FFI**, list **types**, **ownership**, **panic strategy**, **validations**.
3. For **embedded**, list **interrupt concurrency**, **peripheral ownership**, **panic handler**.
4. For **wasm**, measure **`.wasm` size** after changes; note **allocator** use.
5. For **CLI**, verify **exit codes**, **signal handling**, **no unwrap on I/O**.

---

# Appendix X — Dense pattern library (rule / sketch / note)

Earlier revisions stacked dozens of near-identical `ANSSI-MEM` checkpoint cards (placeholder anti-pattern numbers only). That added noise without new semantics. Use **Appendix L** for stable rule IDs, **Appendix U** for breadth, and this **single card shape** when you add a new checkpoint:

- **Focus:** checklist family (e.g. ANSSI-MEM, WASM-SIZE, TOOL-AUDIT).
- **Bad:** concrete anti-pattern (panic surface, unchecked boundary, UB risk, silent misconfig).
- **Good:** enforce invariant with types, `Result`, checked arithmetic, or documented `unsafe` contract.
- **Sketch:** required `SAFETY:` / `deny_unknown_fields` / other mechanized guard when applicable.
- **Note:** cross-link stable IDs from Appendix L in PR text.
- **Test:** regression or property test that would catch the failure mode.

*End of cluster 07 — Embedded, WebAssembly, CLI, Secure Rust.*
