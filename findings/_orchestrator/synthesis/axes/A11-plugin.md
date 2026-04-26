# A11 — Plugin System (BUILD + EXEC): Deep Cross-Project Analysis

**Strategic verdict for Nebula**: Plugin sandboxing is an **industry weakness**. No competitor has shipped strong sandboxing for plugin EXECUTION. WASM sandbox + capability security + Plugin Fund commercial model is **Nebula's clearest defensible advantage** — but requires shipping the implementation, not just the spec.

## A11.1-A11.4 — Plugin BUILD: aggregate findings

### Plugin format taxonomy

| Format | Count | Projects | Comment |
|--------|------:|----------|---------|
| **None / no plugin system** | 14/27 | temporalio-sdk, duroxide, orka, dataflow-rs, dagx, runner_q, raftoral, kotoba-workflow, fluxus, aqueducts-utils, dag_exec, ebi_bpmn, durable-lambda-core, deltaflow | most projects: extension = own crate / fork |
| **In-process compile-time (inventory + traits)** | 5/27 | acts (`inventory::submit!`), acts-next, runtara-core (static linkage), rust-rule-engine, aofctl (workspace member) | simplest extension model |
| **WASM target** | 2/27 | z8run (wasmtime v42), runtara-core (compile-to-WASM is the WHOLE engine, not plugins) | only z8run has plugins-as-WASM |
| **Subprocess + stdio/IPC bus** | 2/27 | emergent-engine (Unix-socket pub-sub MessagePack), flowlang (multi-language pyo3/deno_core/JNI dispatch in-process) | OS-process delegation |
| **MCP subprocess (JSON-RPC)** | 4/27 | rayclaw, aofctl, orchestral, cloudllm | AI-first projects use MCP for tool extension |
| **Docker container** | 1/27 | aofctl (sandbox via bollard) | only project with strong container isolation |
| **Git-repo registry** | 1/27 | emergent-engine (git-repo with TOML manifests + GitHub Releases binary distribution) | distribution mechanism, not sandbox |

### Manifest schema

| Project | Format | Versioned schema | Capability declaration | Permission grants | Registry |
|---------|--------|:----------------:|:----------------------:|:----------------:|----------|
| z8run | TOML | ◐ unversioned | ✓ (`network`, `filesystem`, `memory_limit_mb`) | ✓ but **unenforced** | local dir |
| acts/acts-next | n/a (compile-time) | ✗ | ✗ | ✗ | crates.io |
| emergent-engine | TOML manifest | ✓ | ◐ in primitive metadata | ✗ | git repo + GH Releases |
| aofctl | YAML config | ✓ | ◐ in agent config | Docker-level only | future MCP catalog (P0 issue #71) |
| runtara-core | n/a (inventory) | ✗ | ✗ | ✗ | crates.io |
| rust-rule-engine | `PluginMetadata` struct | ✓ | string dependency validation | ✗ | own implementation |
| Nebula plan | TOML (plugin-v2) | ✓ | ✓ | ✓ | local + remote (planned) |

**Industry signal**: capability/permission declaration in manifest is rare (only z8run, partially). Even when declared, **enforcement is the failing point**.

### Build toolchain support

| Project | SDK / scaffolding | Cross-compilation | Reproducibility |
|---------|-------------------|:-----------------:|:---------------:|
| z8run | none | depends on host | cargo-default |
| emergent-engine | language-agnostic (CLI = primitive) | ✓ via OS subprocess | ✓ |
| aofctl | YAML-first (no Rust required for users) | n/a (Docker images) | Docker-level |
| Nebula plan | plugin-v2 spec planned | ✓ (WASM target portable) | ✓ via lockfile |

## A11.5-A11.9 — Plugin EXEC: aggregate findings

### Sandbox enforcement reality

| Project | Sandbox claim | Actual enforcement | Evidence |
|---------|---------------|--------------------|---|
| **z8run** | "WASM sandbox via wasmtime v42 with capabilities" | **NOT enforced** — capabilities (network/filesystem/memory_limit_mb) declared in manifest, but Linker has no WASI imports linked. Sandbox runs WASM but plugin can't actually access anything (which is technically safe but renders capability declarations meaningless). | architecture.md cites Linker setup |
| **acts** / acts-next | "in-process compile-time plugin" | **No isolation** — `acts.transform.code` runs arbitrary JS via QuickJS, `acts.app.shell` runs arbitrary shell. **NO sandboxing**. | grep evidence cited |
| **runtara-core** | inventory linkage | no isolation (static linkage) | by design |
| **rust-rule-engine** | "PluginMetadata with safety_checks" | **String dependency check only** — `safety_checks` field is just metadata-level dependency string validation, not runtime isolation | architecture.md |
| **emergent-engine** | "OS subprocess primitive" | **No sandbox** — bare OS subprocess with full user permissions. Issue #25 explicitly tracks missing sandbox as a gap. SHA256 verification code exists but is `#[allow(dead_code)]` — not called at install. | architecture.md cites issue #25 + dead_code attribute |
| **aofctl** | "Docker container sandbox" | **Real isolation** via bollard — Docker container limits CPU/memory, network policies, filesystem mounts. Capability/seccomp profile details not fully verified in research. | architecture.md cites bollard usage |
| **Nebula plan** | WASM + capability security | not yet shipped | spec only |

**Strategic finding**: **Only aofctl has shipped real plugin EXEC isolation** in the entire 27-project set (and that via Docker, which is heavyweight + adds infrastructure dependency). z8run attempted WASM-with-capabilities but the capability layer is non-functional. Everyone else either has no plugin system or runs plugins in-process with full host permissions.

### Trust boundary patterns

| Pattern | Projects | Trade-off |
|---------|----------|-----------|
| **No trust boundary (in-process)** | acts, runtara-core, rust-rule-engine, runner_q, etc. | trivial to extend; plugin can do anything host can |
| **OS subprocess (kernel boundary)** | emergent-engine, flowlang, MCP-using AI projects | OS-level isolation; needs OS-side hardening (seccomp, namespaces) for true safety |
| **Container** | aofctl | strong; deployment overhead |
| **WASM (planned strong)** | z8run (attempted), Nebula (spec) | best-effort capability-based isolation; portable; growing ecosystem |

### Host ↔ plugin marshaling

| Marshaling | Projects | Comment |
|------------|----------|---------|
| **JSON via stdio** | emergent (MessagePack), flowlang (DataObject), MCP-using projects | universal but loses Rust type system |
| **Direct trait calls** (in-process) | acts, runtara-core, rust-rule-engine | type-safe but no isolation |
| **WASM ABI (raw ptr/len)** | z8run | low-level; needs wit-bindgen for ergonomics |
| **WASM Component Model + WIT** | (none observed) | future direction; no project has shipped this |
| **gRPC / Protobuf** | (sidecar pattern, raftoral) | distributed semantics; heavy |

## Verdict for Nebula's strategy

### Position vs industry

Nebula's plugin-v2 spec + WASM sandbox + capability-based security is **architecturally correct and strategically differentiated** but **not yet shipped**. Industry is in one of three states:

1. **No plugin system** (most projects) — extension = own crate / fork
2. **In-process plugin** (acts, runtara, rust-rule-engine) — easy to use, no isolation
3. **MCP subprocess** (AI-first projects) — OS-level, language-agnostic, adequate for tool calling but no per-plugin policy

**No competitor** has yet shipped what Nebula plans: capability-based WASM sandbox with manifest-declared permissions enforced at runtime. z8run's attempt validates the architectural direction (declare capabilities in manifest) but illustrates the **enforcement gap** — declaring capabilities is easy; enforcing them via WASI imports is the work.

### Concrete recommendations

1. **Ship MVP capability enforcement before adding new capabilities to spec.** z8run's failure mode (declared capabilities, unenforced) is worse than no capabilities (false sense of security). Nebula should:
   - Pick the smallest viable capability set: `network: deny | allow_list[host]`, `filesystem: deny | tmp_only | allow_list[path]`, `memory_limit_mb: u64`, `wall_time_ms: u64`.
   - Wire each into the wasmtime Linker with concrete WASI import implementations.
   - Test that violations actually trap the plugin, not just log a warning.

2. **Adopt aofctl's Supervisor primitive at the EXEC level** — when a plugin crashes (panic, OOM, deadline), the engine should restart with backoff, not propagate the crash to the workflow. This is currently absent from Nebula's resilience surface (which targets outgoing calls, not plugin crashes).

3. **MCP subprocess as a SECOND plugin transport** — even with WASM as the primary, an MCP-bridge transport is valuable for:
   - Polyglot plugins (Python, Node.js, Go) where rewriting in Rust is unrealistic
   - LLM tool integrations (which expect MCP-over-stdio)
   - Existing tools that already speak MCP (Claude Desktop, Cursor, Cline, etc.)
   
   Recommend: WASM transport for sandboxed compute-heavy plugins; MCP transport for tool-style plugins where capability-based security is provided by the subprocess OS boundary.

4. **Reproducible plugin builds via lockfile in manifest** — none of the competitors have addressed this. Nebula's plugin-v2 spec should include a Cargo.lock fingerprint or equivalent so plugins are reproducible and signable. This is a Plugin Fund (commercial) requirement: customers will need provenance for paid plugins.

5. **Plugin Fund is genuinely defensible** — no competitor has a commercial model for plugin authors. Most projects are MIT/Apache and have no monetization story. Nebula's open-core + Plugin Fund is unique in this space. **Recommendation**: keep Plugin Fund design closely tied to the WASM transport (capability-based licensing per-capability, e.g., "network: api.openai.com" requires a paid plugin tier).

### What NOT to do

- **Don't ship plugin v2 spec without enforcement.** z8run's lesson: declared capabilities without enforcement actively misleads users. Better to ship v0 with `memory_limit_mb` only than v1 with 5 declared-but-unenforced capabilities.
- **Don't attempt Docker as primary transport.** aofctl's choice is reasonable for a DevOps tool but adds heavy operational cost for typical workflow use cases.
- **Don't replicate emergent-engine's git-repo registry** as the primary distribution. It's elegant for hobbyist plugins but has zero security model. Plugin Fund needs a controlled registry with signing.

## Borrow candidates ranked

| Pattern | Source | Effort | Strategic value |
|---------|--------|-------:|-----------------|
| Capability enforcement smallest-viable-set | (industry gap; z8run exposed it) | 4-8w | ⭐⭐⭐ Core differentiator activation |
| MCP subprocess transport (secondary) | rayclaw, aofctl, orchestral, cloudllm | 2-4w | ⭐⭐⭐ Polyglot + LLM tool integration |
| Supervisor primitive for plugin crashes | aofctl `Supervisor` | 1-2w | ⭐⭐ Resilience surface gap |
| Lockfile-in-manifest for reproducibility | (industry gap) | 1-2w | ⭐⭐ Plugin Fund prerequisite |
| `inventory::submit!` for built-in actions | acts, runtara-core | 1w | ⭐ Boilerplate reduction (compatible with WASM-for-third-party model) |
