# Nebula client

First-party egui/eframe workbench for an **existing** Nebula HTTP server. Native:
`task client:run`. Browser: `task client:web`, which needs `cargo binstall trunk` and the
`wasm32-unknown-unknown` target. It serves on port 8090 and proxies `/api` and `/version`
to the server on port 8080, so browser password sign-in stays same-origin. Deployment can
serve the static build (`task client:web:build`) under the same origin as `/api/v1`; the
client does not start a server or worker.

## What it does

- **Sign-in.** A welcome page beside the sign-in card. Password or personal access token; the
  authenticator-code field appears only when the server asks for a second factor, and the
  password is kept for that retry. Sign in stays disabled until the chosen credentials are
  filled in, and Enter submits.
- **Demo workspace.** "Explore the demo workspace" opens a workspace that needs no server. It
  answers through the same typed API as a server (`api::Backend`) with the server's semantics:
  revisions and 409 conflicts, publication checks, idempotent starts, cursor paging, schema
  checks on credentials, the last owner kept. Its catalog and credential types are snapshots
  of the bundled server's answers, and a test decodes every demo reply with the contract types.
  Runs execute on a simulated executor that plans the graph (ports of `if`/`switch`, error
  routes, fail-fast) and evaluates the core actions, so node statuses change live. Signing out
  discards the demo's changes.
- **Workspace.** Organization and workspace slug or ID, or one click on a recent workspace. The
  API has no endpoint that lists a user's workspaces, so the app remembers the ones that
  answered.
- **Shell.** A compact top bar with the workspace breadcrumb, a Demo badge in the demo, account,
  a busy spinner, the shortcut sheet, Switch workspace and Sign out. A navigation rail lists the
  pages. Outcomes appear as toasts: information fades, a failure stays until dismissed.
- **Pages.** Every page reads its data when shown and draws the same four states: loading,
  failed with Try again, empty with the next step, and the data, which stays on screen while
  it is read again after a change.
  - *Workflows*: search over the listed page, inline creation, an "Unsaved" marker for drafts
    with local edits, paging. A card opens the workflow in the editor.
  - *Executions*: the workspace's history filtered by status and workflow, newest first, with
    older pages on demand. A run shows its timeline, each node's attempts, output and error,
    and its input; a running run can be cancelled, an ended one run again.
  - *Node catalog*: actions grouped by plugin, each with its description and its parameter
    form to try out, and a button that adds it to the open workflow.
  - *Triggers*: the trigger bindings of every workflow. A webhook trigger is added, removed and
    registered here; registration shows the address and the signing secret once.
  - *Credentials*: stored credentials with their state, deletion behind a confirmation, and
    creation through a form built from the type's JSON Schema (secrets masked, expressions
    allowed where the type allows them, a union of grant types as one Type choice).
  - *Team*: workspace members and their roles, changed in place, and the organization's
    members; adding and removing, with the last owner kept by the server.
  - *Settings*: profile, this connection, and personal access tokens with scopes and a
    lifetime; a new token is shown once.
- **Editor.** In the style of node editors: an action bar (name, state, Undo, Redo, Reload,
  Publish, Save), then a full-height canvas. Nodes are square cards; drag a card to place it
  (stored in `ui_metadata`, undoable), drag an output port onto an input port to connect, or
  use the "+" after a port to add a connected node. Zoom, Add node and Execute workflow float
  over the canvas. A side panel shows the add-node palette (the action catalog when the
  server publishes one, or a typed action key) or the inspector of the selected node: rename,
  literal parameters as JSON, connections with their ports, removal.
- **Node sidebar.** Selecting a node slides in a panel with Parameters, Settings and Output.
  Parameters is a form built from the action's schema (`GET /actions/{key}/parameters`, the
  `nebula-schema` wire format): every field kind and widget, groups, conditional visibility and
  requirement evaluated as the server evaluates them, Fixed/Expression per field, Reset to the
  default, a readiness line, checks from the field's rules and advice from its format hint. Typed
  text reaches the draft as it is typed, and one focus is one undo step. A field type newer than
  the client, or an object whose schema declares no fields (a condition tree), is edited as
  JSON; an action with free-form input, or without a schema, gets the parameters as JSON with
  the reason.
- **Live status.** The run the visible page shows streams its states until it ends: the demo
  sends each change as it happens, a server is read once a second. Every place that shows the
  run (the executions list and detail, the editor's runs panel) takes the same state. A failed
  read stops the stream with a message until the run is opened again, so a server in trouble is
  not asked in a loop.
- **Runs.** Recent runs and the chosen run load when a workflow opens and after every start,
  with each node's status, output preview and failure reason. A read that fails says so in the
  panel.
- **Keyboard.** Alt+1 … Alt+7 open the pages, `?` shows the shortcut sheet, Escape closes it or
  the side panel. In the editor Ctrl+S saves, Ctrl+Enter runs, Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y
  undo and redo, Delete removes the selected node. Cards and rows take focus and open with
  Enter or Space.
- **Accessibility.** AccessKit is on: every control, card and timeline bar carries a role and a
  name for screen readers.
- **Layouts.** Below 760 points the rail becomes a row of tabs and the editor, side panel and
  runs stack in one scrolling column.

A rejected save or publication names the server's first validation paths. The API does not
report which revision is published, so the editor says "Published" only for a revision it saw
the server publish in this session.

The bundled server lists the actions of its plugin release at `/actions`, so the palette offers
them and every node gets its form. A server without a catalog answers 503: the palette then asks
for a typed key and the node form falls back to JSON.

Not in this release: editing template and reference parameters (they are named, and Reset
replaces them), select options and inputs that a server loader resolves, file uploads,
interactive credential flows (OAuth consent), service accounts and listing organizations (the
server answers 501), member names (the API names members by principal id), managed local
launch, packaging and updates.

## Structure

- `api/`: the typed API the app talks to. `Backend` is a server connection or the demo, with
  the same methods and the contract's request and response types.
- `demo/`: the demo workspace: its world and server rules (`mod`), seed data and fixtures,
  the simulated executor (`executor`) and the core actions it evaluates (`eval`).
- `workbench/`: pure state and reducers for sign-in, workspace, drafts, pages and replies
  (`pages`, `page_replies`). No egui or network types, so every transition is unit-tested with
  fabricated or demo replies.
- `effects`: runs requests off the render thread, and the execution watch beside them. Each
  request carries its session stamp and each watched state its session generation, and late
  answers from an earlier session are dropped.
- `app/`: the shell. `mod` lays out the bar, the navigation and the page, runs the editor's
  intents and keeps the watch on the run the page shows; `pages` turns the pages' intents into
  requests.
- `views/`: rendering only. Views change local UI state directly and return `Intent`s for
  anything that needs the network. `shell` is the top bar and toasts, `nav` the rail, tabs,
  shortcuts and their sheet, `states` the shared loading, failure and empty states,
  `connection` the welcome and workspace pages, one module per page (`workflows`,
  `executions`, `catalog`, `triggers`, `credentials`, `team`, `settings`), `editor` the action
  bar, canvas controls, shortcuts and side panel, `canvas` the graph, `inspector` the selected
  node, `form` the schema-driven form, `runs` the runs panel.
- `schema/`: parameter schemas (`nebula-schema` wire format) and credential type schemas (JSON
  Schema) as one `Form` model.
- `theme` and `widgets`: design tokens (colors, spacing, radii, type scale) and the shared
  controls every view uses. Views take their colors and spacing from here only.
- `document/`: the draft and its undo history. `graph` holds the edits, the recorded changes
  and their replay as pure functions over the definition. Presentation never edits the
  definition directly.
- `session`: draft ownership and generation fences, independent of presentation.
- `transport/`: HTTP adapter with bounded requests and explicit reconciliation; `resources`
  covers executions, credentials, webhooks, the account and members.
- `clock`: wall time, RFC 3339 and relative times, the same natively and in the browser.

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
Closing/reloading the app clears them. Between launches the app keeps only the server
address, email, sign-in mode, recent workspaces and panel sizes (eframe persistence: a file
natively, localStorage in the browser); no secret, draft or parameter value is written. The
browser build always signs in against its own origin. Network work runs outside rendering. Session generation and request
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
in memory. A token is taken out of its field when sent; a password and code stay until the
sign-in settles, so a second-factor retry need not ask again, and are wiped when it succeeds
or fails.

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
