> **⚠ DEFERRED 2026-04-13**
>
> The `[permissions]` manifest model described in this file is **not being implemented in Phase 1–4** of the nebula-sandbox roadmap. See `sandbox-prior-art.md` → "Deferred decisions" and `2026-04-13-sandbox-roadmap.md` §D4 for the rationale.
>
> The research here remains useful if and when a real operator or community-plugin requirement drives the design. Until then, nebula ships with **no plugin-declared permission scope** — only process isolation, broker RPC, anti-SSRF, audit log, OS jail, and signed manifest.
>
> Do not cite this file as the current design. It is background research preserved for future revisits.

---

# Sandbox permission format — comparative research

Research done 2026-04-13 while designing nebula-sandbox capability model. Five systems surveyed. Kept here so the next session doesn't re-derive.

## The five data points

| System | Authored in | Form | Scopes | Deny | Philosophy |
|--------|-------------|------|--------|------|------------|
| **Deno** | CLI flags + `deno.json` | `--allow-net=host,*.x.com,1.1.1.1:443` paired with `--deny-net=...` | yes, inline in flag value | yes, explicit `--deny-*` | fail-closed, flags open |
| **Chrome MV3** | `manifest.json` | split: `permissions: ["storage", "scripting"]` + `host_permissions: ["https://*.x.com/*"]` | host_permissions only | no | named toggles ⊕ match-patterns |
| **WASI preview 2** | WIT `world` imports | `import wasi:cli/environment@0.2.4` | none in manifest; host picks impl | n/a | didn't import → linker-unreachable |
| **Flatpak** | `finish-args` in YAML/JSON | `--share=network`, `--socket=wayland`, `--filesystem=home:ro` | suffix-based `:ro`/`:rw`/`:create` | no (`=nothing` negates) | categories, flat strings |
| **GH Actions** | workflow YAML | `contents: read`, `issues: write`, `actions: none` | no (scope = repo) | three levels | three levels, implicit scope |

## What each gets right

- **Deno**: scope values as flat strings (`*.example.com`, `1.1.1.1:443`, `[2606:4700::1111]`) — no nested tables, readable one-liners. Paired `--allow-*` / `--deny-*` makes "allowlist with holes" ergonomic.
- **Chrome MV3**: splits *named toggles* from *scope lists* because they have different UX needs (install-time vs runtime prompts, high-risk vs low-risk). Hides the split from code but surfaces it at grant time.
- **WASI**: manifest is an *import list*, not an *allow/deny* set. "Not imported" is a stronger guarantee than "denied" — the former is a linker error, the latter a runtime check. Also: verbs are versioned (`@0.2.4`) so semantics can evolve without breaking old plugins.
- **Flatpak**: category prefixes (`share`, `socket`, `device`, `filesystem`) group related perms, which drives both manifest organization and portal UX.
- **GH Actions**: three-level coarse grant (`read`/`write`/`none`) is terse for the common case where you don't need scope.

## What each gets wrong (for our use case)

- **Deno**: CLI flags are for app operators, not plugin authors. Doesn't scale to a declarative manifest beyond `deno.json` (which is basically the same flat shape).
- **Chrome MV3**: two parallel arrays (`permissions` + `host_permissions`) means plugin authors have to think about which bucket a thing goes in. Fine for a five-permission standard set, awkward at scale.
- **WASI**: pure linker-level capability gating is great for pure-WASM; breaks down when we need **scoped** versions of the same verb (not "import fs", but "import fs restricted to `$APPDATA`"). We still need scopes at our layer.
- **Flatpak**: no structured scope objects — everything is string suffixes (`home:ro`). Extensibility suffers: adding new options to an existing permission means new suffix grammar.
- **GH Actions**: three-level model is too coarse for file paths and hosts.

## Synthesis — what nebula takes

### From Deno: flat strings for scope values
Scope is a string or list of strings, not a nested table. Tauri-style `{scheme, host, paths}` is reserved for the rare cases that need it.

### From Chrome: split by risk, but hidden from the author
One flat `[permissions]` table in the manifest. The grant UI groups by namespace prefix (`network:*` = net, `fs:*` = fs, `camera:*`/`microphone:*`/`clipboard:*` = devices, `log:*`/`time:*` = core). Author writes flat, user sees grouped.

### From WASI: absence = denied
The manifest is an import list, not an allow/deny pair. If an identifier isn't in `[permissions]`, the broker verb is unreachable by construction. Deny lists exist only as inline long-form for "everything except X" corner cases.

### From WASI: versioned verbs
Identifiers carry an implicit version. Short form `"network:http" = ["..."]` = v1. Long form `{ version = 2, allow = [...] }` for explicit. Broker register-time check: plugin declares version N, broker supports v1..vM, must satisfy.

### From Flatpak: namespace prefixes
`namespace:verb` identifier scheme (`network:http`, `fs:read`, `camera:capture`). Namespaces group related verbs for UI and policy. No separate "category" field — the prefix *is* the category.

## Rejected

- **Three-level coarse grants** (GH Actions). Our risk surface is too broad — "read all files" vs "write all files" isn't enough granularity.
- **Two-array split** (Chrome). Adds an author-facing taxonomy question for no benefit when we can group at render time.
- **CLI-flag authoring** (Deno). Plugin manifest is not a CLI.
- **Suffix-based modifiers** (Flatpak `:ro`). Inline tables are more extensible.
- **Pure linker-level capability** (WASI). We're not WASI; we have scoped versions of the same verb and need to express that in the manifest.

## Final format (for reference)

```toml
[plugin]
key     = "com.author.telegram"
version = "1.2.3"
author  = "alice@example.com"

[runtime]
min-protocol = 2
min-tier     = 1

[permissions]
# four forms, serde untagged
"log:emit"     = true                                        # bool switch
"env:get"      = "TZ"                                        # single scope
"network:http" = ["api.telegram.org", "*.telegram.org"]      # scope list
"fs:write"     = { allow = ["$APPDATA/*"], deny = [], version = 1 }  # full

[signing]
algorithm  = "ed25519"
public-key = "..."
signature  = "..."
```

## Invariants

1. **Absence = denied.** Manifest is an import list. Not declared → unreachable.
2. **Four value forms.** `true | "str" | [str] | {allow, deny, version, features}`. One serde untagged enum.
3. **Namespace prefix drives grouping.** Grant UI groups by prefix; authors write flat.
4. **Version default = 1.** Explicit via long form. Mismatch fails register.
5. **Credentials / resources / log / time / rand / cancel are NOT in `[permissions]`.** They come from `ActionContext` and are bound per workflow-node, not per plugin.
6. **`[permissions.dev]`** (cargo `[dev-dependencies]` analogue) may declare permissions active only under `nebula plugin run --dev`. Ignored by production registries.

## Links for future sessions

- Deno permissions: https://docs.deno.com/runtime/fundamentals/security
- Chrome MV3 manifest: https://developer.chrome.com/docs/extensions/reference/manifest
- WASI component model: https://component-model.bytecodealliance.org/design/worlds.html
- Flatpak manifest: `man flatpak-manifest`, `finish-args` section
- GH Actions permissions: https://docs.github.com/actions/writing-workflows/choosing-what-your-workflow-does/controlling-permissions-for-github_token
