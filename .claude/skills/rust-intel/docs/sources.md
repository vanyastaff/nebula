# Sources

The empirical basis for `rust-intel.md`. Grouped by source type.

> **Discipline:** every numeric figure or categorical claim in the spec must be traceable to an entry below. If a source is missing here, the claim is considered under-grounded and belongs in [`roadmap.md`](roadmap.md), not the main ruleset.

## Academic benchmarks

### RustEvo²
Post-cutoff API drift benchmark for Rust.
- **Source:** arXiv:2503.16922 — <https://arxiv.org/abs/2503.16922>
- **Key figure:** pass@1 drops from **56.1% → 32.5%** on APIs that changed after the model's knowledge cutoff.
- **Used in:** §A1 (API hallucinations), Operating mode step 1 (pin the world).

### SafeTrans
Safety benchmark for Rust transpilation / generation.
- **Source:** arXiv:2505.10708 — <https://arxiv.org/abs/2505.10708>
- **Key figures:**
  - E0277 + E0308 together account for **>18% of all errors** in LLM-generated Rust, up to **30%** for some models.
  - Tier A errors land in 18–30% of generations.
- **Used in:** §A2 (trait bounds), Tier A intro.

### CRUST-Bench
C→Rust translation benchmark with test coverage.
- **Source:** arXiv:2504.15254 — <https://arxiv.org/abs/2504.15254>
- **Used in:** Tier B context (the gap between "compiles" and "correct").

### SafeGenBench
Safety benchmark for generated code, including crypto.
- **Source:** arXiv:2506.05692 — <https://arxiv.org/abs/2506.05692>
- **Key figure:** **~57% of vulnerabilities** in LLM-generated crypto Rust that compiles are missed by static analyzers (CodeQL and similar).
- **Used in:** §B12 (crypto silent insecurity), Tier B intro.

### Rust-SWE-Bench
Benchmark of 500 real-world repository-level Rust issues from 34 popular crates.
- **Source:** "Evaluating and Improving Automated Repository-Level Rust Issue Resolution with LLM-based Agents", arXiv:2602.22764 — <https://arxiv.org/abs/2602.22764>
- **Key figure (compilation-failure distribution):** **76.3%** of all compilation failures from LLM agents fall into just two categories:
  - 43.7% — failure to model project organization (E0433, E0432, E0425, E0412, E0405).
  - 32.6% — failure to respect type/trait semantics (E0599, E0308, E0277, E0407).
- **Key figure (task resolution):** ReAct-style agents resolve up to 21.2% of issues; RustForger with Claude Sonnet 3.7 reaches 28.6% (34.9% over the strongest baseline).
- **Used in:** justifies the §A1 / §A2 priority.

### AkiraRust
LLM-aided Rust repair framework with a feedback-guided thinking switch (FSM-driven dual-mode reasoning).
- **Source:** "AkiraRust: Re-thinking LLM-aided Rust Repair Using a Feedback-guided Thinking Switch", arXiv:2602.21681 — <https://arxiv.org/abs/2602.21681>
- **Key figure:** GPT-5 alone reaches 75% pass rate on the benchmark; AkiraRust's repair loop reaches 100%, isolating the qualitative gap between raw LLM and feedback-guided repair on Rust ownership/lifetime/aliasing issues.
- **Used in:** general taxonomy context; supports the "compile-fix loop is required" framing behind /rust-fix.

## Field report (published)

**"Я заставил LLM писать Rust полгода. Вот что они стабильно ломают"** — uproger.com, 2026-05-16.
- **Source:** <https://uproger.com/ya-zastavil-llm-pisat-rust-polgoda-vot-chto-oni-stabilno-lomayut/>
- **Setup:** 6-month observation of Claude / GPT / Cursor generating Rust in production. ~80k LOC of streaming-data backend; stack: tokio + sqlx + unsafe hot paths. Roughly 40% of commits contained AI-generated code. Failures were classified across 50 benchmark tasks against four major models.
- **Status:** published field report, not a peer-reviewed study. Cited here because the numeric findings are documented and reproducible by anyone running the same benchmark; treat as directional but anchored to a public artifact.

Key findings used in the spec:
- **§B1 (lifetime laundering):** reproduced in 34 of 50 tasks that return a reference.
- **§B2 (Mutex across .await):** this category was the proximate cause of failure in roughly half of async tasks observed; pinning crate versions in the prompt cut the rate sharply.
- **§B2 / `await_holding_lock`:** clippy caught only ~7 of 23 cases (i.e. about 30%) — misses guards in closures, `if let`, early-return blocks. Confirmable independently by inspecting the lint's source.
- **§B3 (cancel safety):** **zero** models spontaneously mentioned cancel-safety across the timeout-using tasks; when asked directly, models answered "yes, it's cancel-safe" confidently and incorrectly in ~50% of cases.
- **§B5 (unsafe UB):** of 40 LLM-generated `unsafe` blocks — 13 definite UB, 9 conditional UB (alignment, OOB, Stacked Borrows), 18 correct. So **~55% of LLM-generated unsafe is a powder keg**. (Directionally consistent with SafeGenBench findings.)

## Industry reports

### Faros AI (2026)
- **Source:** <https://www.faros.ai/blog/ai-acceleration-whiplash-takeaways>
- **Key figure:** AI-generated PRs have a **+242.7% incident rate** relative to human-authored ones.

### Lightrun — State of AI-Powered Engineering 2026
- **Source:** <https://lightrun.com/ebooks/state-of-ai-powered-engineering-2026/>
- **Key figure:** **43%** of AI-generated PRs require post-merge debugging. Among surveyed senior engineers — **zero** rated themselves "very confident" in AI-generated Rust.

### Codestral / DeepSeek-Coder studies
Method-existence hallucination rates in major code-generation models.
- **Source basis:** SafeTrans and RustEvo² breakdowns (see entries above) plus per-model evaluation cards.
- **Key figure:** LLMs generate non-existent methods (E0599) in **up to 22%** of cases for Rust.
- **Used in:** §A1.

### Slopsquatting / package-hallucination studies
- **Key figure:** crate-name hallucination rate in Rust reported as **elevated relative to other major-language ecosystems** in published slopsquatting research; primary citation is the Lanyado/Spracklen-style "Hallucinated Package Imports" line of work — verify the specific Rust figure against the source paper before quoting precisely.
- **Used in:** §A1 (slopsquatting defense).

## Documented incidents

### CrateDepression (2022)
Malicious crate `rustdecimal` — typosquat of the legitimate `rust_decimal` (~3.5M downloads). Targeted CI pipelines.
- **Source:** Rust Security Response WG advisory, 2022-05-10 — <https://blog.rust-lang.org/2022/05/10/malicious-crate-rustdecimal/>
- **Used in:** §A1.

### `faster_log` / `async_println` (2025)
Malicious crates that scan for and exfiltrate Solana/Ethereum private keys. Reached thousands of downloads before takedown.
- **Source:** Rust Security Response WG advisory, 2025-09-24 — <https://blog.rust-lang.org/2025/09/24/crates.io-malicious-crates-fasterlog-and-asyncprintln/>
- **Used in:** §A1.

### Supply-chain trend in the Rust ecosystem (2025)
- **Observation:** attacks against crates.io rose materially in 2025 — beyond the two named incidents above, several smaller malicious-crate takedowns occurred. Order-of-magnitude estimates from industry reports range around +100–130% year-over-year; treat as directional.
- **Used in:** §A1 (slopsquatting context).

### Cargo issue #2524
Known gotcha: `features = [...]` inside `[target.'cfg(...)'.dependencies]` activates globally, not per-target. <https://github.com/rust-lang/cargo/issues/2524>
- **Used in:** §C7.

## Standards and documentation (normative sources)

- **`rand` crate security policy** — `ThreadRng` is a CSPRNG (ChaCha12, seeded from `OsRng`). For keys/nonces, prefer `OsRng` directly to remove ambiguity about seeding chains. <https://github.com/rust-random/rand/blob/master/SECURITY.md>, <https://rust-random.github.io/book/guide-rngs.html>. §B12.
- **Rust 1.80 `--check-cfg` automation** — after 1.80, declared features in `Cargo.toml` automatically generate `unexpected_cfgs` warnings for typo'd `cfg(feature = "…")`. <https://blog.rust-lang.org/2024/05/06/check-cfg/>, <https://blog.rust-lang.org/2024/07/25/Rust-1.80.0/>. §C7.
- **Native async fn in traits (RPITIT)** — stabilized in Rust 1.75; idiomatic 2025–2026 pattern is `fn bar(&self) -> impl Future<Output = T> + Send`, with `trait-variant` for Send/non-Send variants and `async-trait` only for `dyn Trait` cases. <https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/>. §B15.
- **Tokio docs** — per-function cancel-safety guarantees (`AsyncReadExt::read` is cancel-safe, `read_exact` is not, etc.). §B3.
- **Rust Reference / Nomicon** — Stacked Borrows, `repr(Rust)` vs `repr(C)`, Pin contracts. §B5, §B15.
- **`clippy` lints:** `await_holding_lock`, `clone_on_copy`, `unwrap_used`, `expect_used`, `missing_safety_doc`, `undocumented_unsafe_blocks`, `redundant_clone`. Post-flight checklist.
- **`miri`** — required in CI for any file containing `unsafe`. §B5.
- **`loom`** — model checking for multi-lock code. §B9.
- **`tokio-console`** — runtime visibility for §B9, §B11.
- **`cargo-hack` + `--feature-powerset`** — §C7.

## How to add a source

1. State the claim in the spec that depends on this source.
2. Add an entry here with (a) type, (b) key figure/observation, (c) link to the spec paragraph, (d) URL/arXiv/DOI.
3. For an academic benchmark or industry report, include the year — the LLM landscape moves fast and time-anchoring matters.
4. For a production observation, state the scale (LOC, time period, stack) and label it as observation, not study, unless it is formally published.
