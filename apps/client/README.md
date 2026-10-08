# Nebula client

The workbench uses a light palette, Inter body text, resizable navigation and runs
panes, a graph canvas with an inspector for the selected node, and a stacked layout in
narrow windows.

First-party egui/eframe workbench for an **existing** Nebula HTTP server. Native:
`task client:run`. Browser: `task client:web`, which needs `cargo binstall trunk` and the
`wasm32-unknown-unknown` target. It serves on port 8090 and proxies `/api` and `/version`
to the server on port 8080, so browser password sign-in stays same-origin. Deployment can
serve the static build (`task client:web:build`) under the same origin as `/api/v1`; the
client does not start a server or worker.

Sign in with email/password (optional TOTP) or a PAT. Enter organization and workspace
slugs/IDs, then create a blank workflow or select one. The graph canvas shows nodes and
connections. Add a node by action key, drag from an output port onto an input port to connect,
select a node to rename it, edit its literal parameters, disconnect it or remove it. Undo and
redo cover every graph and parameter edit. Save changes, publish, run the server's current
publication, then read persisted execution status. Recent runs make accepted work discoverable
after reconnect. A rejected save or publication names the server's first validation paths.

Not in this release: moving nodes on the canvas (positions are derived from connections),
editing expression or template parameters, choosing actions from a catalog (the server does
not attach an action registry in its current composition, so the catalog answers 503 and
action keys are typed), managed local launch, packaging and updates.

## Structure

- `workbench/`: pure state and reducers for sign-in, workspace, drafts and replies. No egui
  or network types, so every transition is unit-tested with fabricated replies.
- `effects`: runs requests off the render thread. Each request carries its session stamp,
  and late replies from an earlier session are dropped.
- `views/`: rendering only. Views change local UI state directly and return `Intent`s for
  anything that needs the network; `app` runs them after the frame. `canvas` draws the graph,
  `editor` holds the toolbar and reconciliation, `inspector` edits the selected node.
- `theme` and `widgets`: design tokens (colors, spacing, radii, type scale) and the shared
  controls every view uses. Views take their colors and spacing from here only.
- `document/`: the draft and its undo history. `graph` holds the edits, the recorded changes
  and their replay as pure functions over the definition. Presentation never edits the
  definition directly.
- `session`: draft ownership and generation fences, independent of presentation.
- `transport/`: HTTP adapter with bounded requests and explicit reconciliation.

The library denies `unwrap`, `expect`, `panic`, `todo` and `unimplemented` through
`#![deny]` in `lib.rs`. Tests keep them through `clippy.toml`.

The editor requires the workflow detail `definition` and storage `revision`, added
alongside this app. Old metadata-only servers are explicitly unsupported for editing.
Save supplies `expected_revision`; 409 preserves the draft. Read the server version,
compare nodes, then explicitly discard the draft or reapply the recorded edits to it.
Reapply replays graph and parameter edits in order. An edit whose outcome the server
already has, such as an existing connection, counts as applied. An edit that cannot be
honored, such as a connection to a node the server removed, stops the replay and keeps the
draft. Publication is also revision-fenced. Run captures the server's current publication
at server admission; it is not an atomic save/publish/run transaction.

Drafts survive disconnects and server/workspace switches **in memory**, keyed by
normalized endpoint, authenticated principal, organization, workspace and workflow.
Closing/reloading the app clears them; no secrets or parameter values are written to
disk/localStorage. Network work runs outside rendering. Session generation and request
sequence reject stale replies before state changes. Indeterminate saves/publications
require a server read and explicit draft recovery. An indeterminate run retains its
Idempotency-Key; **Reconcile pending run** repeats that same intent, never a new key.
Disconnect does not cancel or stop server workflows and does not revoke a server session.

HTTPS is required except on loopback. Native password auth captures only the two
Nebula cookies and sends their CSRF token with mutations. Browser password auth requires
same-origin hosting; cookies retain their server-owned Secure/HttpOnly/SameSite policy.
PAT requests omit browser cookies, and remote browser access requires operator-approved
CORS. Both transports reject redirects, bound responses to 1 MiB and requests to 30s,
and never automatically replay writes. Failures exclude raw URLs, tokens, bodies and
provider error prose. TLS certificates must be trusted by the host. Credentials stay
in memory; password/token inputs are cleared after submission.

Checks from the workspace root:

```text
task client:check
cargo nextest run -p nebula-client --lib
cargo clippy -p nebula-client --all-targets -- -D warnings
cargo build -p nebula-client --bin nebula-client
cargo build -p nebula-client --target wasm32-unknown-unknown --lib
```

The ignored live acceptance test requires a **fresh isolated** ordinary SQLite server
enrolled through `nebula-server setup begin` with email `client-acceptance@example.test`,
display name `Client acceptance`, organization name `Client acceptance`, and the public
fixture password `isolated-client-acceptance-password` supplied through piped stdin.
The enrollment creates `personal/default`. Configure the server's test SQLite path,
loopback bind, credential dev key and synthetic worker artifact digest according to
[server instructions](../server/README.md). Do not run this fixture on a real deployment.
Set `NEBULA_CLIENT_TEST_ENDPOINT` to that isolated endpoint, then run:

```text
cargo nextest run -p nebula-client --lib --run-ignored ignored-only live_existing_server
```

This uses the app's HTTP adapter and pure document commands against the ordinary
server/worker: password session, list/load, competing save, 409, explicit reapply, save,
publish, keyed start/replay, completed persisted status and the edited node output.
It creates one test workflow per run. Windows/native and WASM compilation are separate
checks from runtime verification on macOS/Linux, browser auth/CORS, keyboard/IME,
clipboard, accessibility, packaging and updates. Those require their own release evidence.

The UI uses Inter from the Google Fonts `ofl/inter` distribution (SIL Open Font
License, retained in `licenses/Inter-OFL.txt`) and unmodified fallback fonts from
`epaint_default_fonts 0.36.2`. Their original
notices are retained in `licenses/` and copied into the browser bundle. Native
distribution must include those notices alongside the executable.
