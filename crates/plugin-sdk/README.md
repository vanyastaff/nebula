# nebula-plugin-sdk

Plugin-author SDK for writing community plugins against the Nebula duplex broker protocol.

**Layer:** Business (SDK for external authors)
**Canon:** §7.1 (plugin packaging), §12.6 (plugin IPC is the trust model)

## Status

**Overall:** `implemented` — the wire protocol and `run_duplex` entry point are usable for authoring out-of-process plugins today.

**Works today:**

- `PluginHandler` trait — plugin authors implement this
- `PluginCtx` — execution context passed into actions
- `PluginMeta` — static metadata declared by the plugin
- `PluginError` — typed error crossing the broker boundary
- `run_duplex` — `main`-callable entry point that takes a `PluginHandler` and runs the wire protocol
- `protocol` submodule — envelope types imported by the host (`nebula-sandbox`) to (de)serialize messages
- `transport` module — line framing over stdio
- `bin/` — runnable example / reference plugin
- Integration test binary (1)

**Known gaps / deferred:**

- **Async runtime is `tokio` only.** No attempt at runtime-agnostic design; by design for the initial release.
- **Protocol versioning** — `PluginMeta` carries version strings, but cross-version compatibility of the wire envelope is not yet a tested contract. See canon §7.2 and `docs/UPGRADE_COMPAT.md`.
- **No capability negotiation in the handshake** — the `capabilities` model lives in `nebula-sandbox` and is not yet wired through the SDK handshake (related to `nebula-sandbox` capability TODO).
- **1 panic site** — review for typed error.
- **2 unit test markers, 1 integration test** — light coverage; the handshake path is the most exercised surface.

## Architecture notes

- **Zero intra-workspace dependencies.** This is the **correct** choice for an SDK that external plugin authors will link against — the plugin-side crate should not depend on engine-side infrastructure. Any future cross-imports should be questioned hard.
- **Wire envelope types live here, not in `nebula-plugin`**, because plugin authors must link against them; the host side (`nebula-sandbox`) imports them back to speak the same protocol. This is intentional directional coupling, not DRY violation.
- **No dead code or compat shims.**
- **Separate from `nebula-plugin`.** `nebula-plugin` is the host-side registry/trait; `nebula-plugin-sdk` is the plugin-author-side SDK. These are different audiences — keep them separate even though both mention "plugin" in the name.

## What this crate provides

| Type / fn | Role |
| --- | --- |
| `PluginHandler` | Trait plugin authors implement. |
| `PluginCtx` | Action execution context. |
| `PluginMeta` | Plugin metadata + action declarations. |
| `PluginError` | Typed error crossing the protocol boundary. |
| `run_duplex` | `main`-callable entry point; handles framing + dispatch. |
| `protocol` | Wire envelope types. Host imports these to (de)serialize. |
| `transport` | Stdio line framing. |

## Where the contract lives

- Source: `src/lib.rs`, `src/protocol.rs`, `src/transport.rs`
- Example: `src/bin/`
- Canon: `docs/PRODUCT_CANON.md` §7.1, §12.6
- Satellite: `docs/PLUGIN_MODEL.md`
- Upgrade story: `docs/UPGRADE_COMPAT.md`

## See also

- `nebula-sandbox` — the host side of the duplex broker
- `nebula-plugin` — host-side registry / trait
