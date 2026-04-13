# Phase 5 — Input devices (desktop)

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~4-5 weeks
**Blocks on:** Phase 1 (broker & capability format), Phase 3 (cross-platform + Tauri-aware desktop path)
**Blocks:** "desktop plugins with device access" shipping story

## Goal

Extend sandboxed plugin execution to **input devices** — camera, microphone, clipboard, global shortcuts, keyboard/mouse streams, notifications, geolocation — with OS consent delegated to the platform and streaming data plane that is **not** JSON-RPC. Same `[permissions]` manifest format (Phase 1), same broker control plane, same `absence = denied` invariant. What's new is a **second transport** for bulk data (shared memory rings) and a delegation path through **Tauri** when running under the desktop app.

At the end of Phase 5, a desktop plugin can read camera frames or microphone audio at realtime rates through a zero-copy shm ring, with consent handled by the OS and revocation live.

## Non-goals

- Mobile-only surfaces (NFC, barcode scanner, biometric, haptics). They're in Tauri's `plugins-workspace` but nebula's desktop app is desktop-first. Defer to a separate track.
- Gamepad / MIDI / USB HID — not in scope.
- Screen sharing to remote peers (WebRTC) — plugin can capture frames and push them somewhere, but the broker doesn't ship a sharing primitive.
- 3D scene / GPU compute — out of scope.
- Cross-platform parity with Linux for device workflows. Devices are **Tier 2 (desktop)** territory; Tier 1 headless servers rarely need them.

## Why devices break the clean broker model

Everything in Phase 1 — network, fs, env, credentials — fits **request/response over JSON-RPC on stdio**. Devices don't, for three reasons:

1. **Data rate.** A 1080p30 YUV420 camera stream is ~93 MB/s. Base64 + serde_json round-trip on stdio is ~10× slower than zero-copy shared memory and wastes CPU on both sides.
2. **Latency.** Keyboard events need <10 ms end-to-end. Any line-oriented IPC with framing + parsing is borderline.
3. **Backpressure.** A plugin stalled for 100 ms shouldn't cause the host to buffer 10 frames — it should drop older ones. That's a ring-buffer semantic, not a message queue.

Solution: **control plane is gRPC, data plane is shared memory (live streams) or one-way envelopes (event streams)**. No manifest-declared device permissions — plugins that need devices just call the broker verb; broker enforces sandbox-wide invariants (OS consent must exist for the host app, audit log, tray indicators).

## Device classification by data shape

| Class | Examples | Transport | Phase |
|---|---|---|---|
| **Small blobs (request/response)** | `clipboard.read/write`, `geolocation.query`, `notifications.emit` | gRPC unary RPC (same as Phase 1 broker) | 5 |
| **Control-event streams** | `global-shortcut.events`, `input.keyboard.events`, `input.mouse.events` | gRPC server-streaming, rate-limited | 5 |
| **Live data streams** | `media.camera.open`, `media.microphone.open`, `media.screen.open` | shared memory ring, control via gRPC | 5 |

## Broker verb surface

All device verbs are **default-allow** from the sandbox perspective (see roadmap §D4 — no manifest permissions). The real gate is **OS-level consent to nebula-desktop as a whole application** — if the host app doesn't have camera TCC, no plugin can get camera, because the broker itself can't open the device. Plugin-level finer grain is deferred.

```
# Small blobs (gRPC unary)
clipboard.read                    → { format, data }
clipboard.write { format, data }  → ok
notifications.emit { ... }        → ok
geolocation.query                 → { lat, lon, accuracy }

# Control-event streams (gRPC server-streaming)
global_shortcut.register { combo }  → stream of fired events
input.keyboard.subscribe { scope }  → stream of key events (scope = app|global)
input.mouse.subscribe { scope }     → stream of mouse events

# Live data streams (gRPC returns MediaStreamRef, data flows via shared memory)
media.camera.open { format, resolution, fps }    → MediaStreamRef
media.microphone.open { format, sample_rate }    → MediaStreamRef
media.screen.open { mode }                       → MediaStreamRef
```

**Tray indicator is non-optional** — while any `media.camera.*`, `media.microphone.*`, `media.screen.*`, or `input.keyboard.subscribe { scope = global }` stream is open, the desktop app shows a persistent tray indicator with plugin name. Plugin cannot suppress it. This is the primary user-visible signal that a trusted-by-signature plugin is using a sensitive device.

**High-risk operations get an audit event at broker level**: `SandboxHighRiskOp { plugin, op }` fires for `input.keyboard.subscribe { scope = "global" }` and `media.screen.open { mode = "full" }`. Operators can subscribe to this event and alert or block.

**Unprivileged Linux refuses `global` keyboard/mouse scope** — no `uaccess`, no grant. Plugin gets error, plugin declares inability to handle it, and operator knows.

## Single-layer OS permission model

Devices only need **one layer of permission** — the OS-level grant to nebula-desktop as a whole application. There is no second per-plugin layer (see roadmap §D4 — permission manifests deferred).

```
┌─────────────────────────────────────────────────────────────┐
│ OS permission (app-level, granted to nebula-desktop)        │
│   macOS:   TCC (Camera, Microphone, Screen Recording, ...)  │
│   Windows: Privacy → Camera/Mic/... + UAC                   │
│   Linux:   xdg-desktop-portal (camera, microphone, shortcut)│
│                                                             │
│   ONE prompt per capability, per app. User grants to        │
│   nebula-desktop as a whole application, not per plugin.    │
│   Outside our control — handled by the OS itself.           │
└─────────────────────────────────────────────────────────────┘
         │
         │ nebula-desktop has camera grant.
         │ Any installed plugin can request camera via broker.
         ▼
    Broker opens device, streams to plugin via shm.
    Audit log records which plugin used the camera.
    Tray indicator shows "camera in use by plugin X".
```

### Why this is still safe

Plugin cannot open the camera directly because:

- **macOS**: plugin child is spawned with `responsibility_spawnattrs_setdisclaim(1)`, which makes it its own TCC responsible process (not inheriting nebula-desktop's grant). The OS refuses any AVCaptureDevice open from the disclaimed child.
- **Windows**: AppContainer children don't inherit parent's privacy-surface grants — fresh security principal.
- **Linux**: plugin child in empty network namespace + no portal session — cannot even reach `xdg-desktop-portal`.

So the plugin's *only* path to the camera is through broker RPC. Broker runs in the host process (which holds the OS grant), opens the device, writes frames to a shared memory ring, plugin reads. If the plugin sandbox-escapes and tries to call `AVCaptureDevice` directly, the OS refuses because the disclaimed child has no grant.

**The defense is structural** — disclaim + OS consent means plugin cannot touch devices without going through broker, and broker has the audit log. A compromised plugin can use a device it was presumably installed to use (we signed it, we trust it to do what it does) but cannot hide from the tray indicator or the audit log.

### Why not a per-plugin grant?

It was the speculative Phase 5 design. Rejected because:

1. **Grant UI fatigue**: "click allow" becomes autopilot, defeating the purpose.
2. **We have no real requirements yet** — no operator has asked "plugin X should NOT be able to use the camera even though the user installed it knowing it processes images".
3. **Trust = signing, not manifest declaration**. User installed a signed plugin → user approved it. Adding a separate "are you sure?" UI after install is theater.
4. **Tray indicator + audit log catch abuse post-hoc** — user sees tray, checks log, uninstalls plugin. Fast enough for the threat model.

Revisit when operators actually ask for finer grain.

## Tauri delegation mode (desktop)

When running under `apps/desktop` (Tauri), nebula-sandbox does **not** call `AVFoundation` / `Windows.Media.Capture` / `PipeWire` directly. Instead:

1. On sandbox init, detect Tauri runtime (env var `NEBULA_RUNTIME=tauri-desktop` or compile-time feature `tauri-host`).
2. Broker device verbs route to a **Tauri IPC bridge** (`DeviceBroker::Tauri`) instead of the native backend (`DeviceBroker::Native`).
3. Tauri's own capability system has already granted the desktop shell access to camera/mic/clipboard/etc. at the Tauri-ACL level (the desktop app's `capabilities/*.json`). The OS-level TCC / Windows Privacy / Portal consent was handled by Tauri at its first use.
4. Broker passes RPC verbs through `tauri::AppHandle::emit_to`. **No plugin-level check** — if the plugin calls `media.camera.open` and nebula-desktop holds the TCC grant at Tauri level, the call succeeds. Audit log records it.

**The chain**: nebula-desktop (signed app) holds Tauri-level grant → Tauri holds OS-level grant. Plugin calls broker → broker calls Tauri → Tauri calls OS → OS accepts because nebula-desktop is the responsible process.

If nebula-desktop doesn't have the Tauri grant for a device, the broker call fails with `DEVICE_NOT_AVAILABLE { reason: "host_not_granted" }`. Plugin surfaces this as "this workflow needs nebula-desktop to have camera access — please grant it in Tauri".

### Native mode (headless server, or desktop without Tauri)

Broker goes directly to:

- **Linux**: PipeWire for camera/mic (xdg-desktop-portal flow for consent), `wl-clipboard`/`xclip` for clipboard, `evdev` for keyboard (requires explicit privileged install), `libnotify` for notifications. `camera:*` and `microphone:*` on Linux *require* PipeWire; no `/dev/video0` fallback for untrusted plugins.
- **macOS**: AVFoundation, NSPasteboard, Core Graphics event taps (requires Accessibility TCC), NSUserNotificationCenter, CoreLocation.
- **Windows**: Windows.Media.Capture, Windows.ApplicationModel.DataTransfer.Clipboard, SetWindowsHookEx (low-level keyboard hook), Windows.UI.Notifications, Windows.Devices.Geolocation.

Headless server mode is mostly used with `clipboard:*`, `notifications:emit`, `geolocation:query` — camera / mic / keyboard hooks rarely make sense server-side.

## Shared memory ring protocol (live streams)

For `media.camera.open`, `media.microphone.open`, `media.screen.open`.

### Primitive: `memfd_create` on Linux, POSIX shm on macOS, named FileMapping on Windows

- Linux: `memfd_create("nebula-shm-<uuid>", MFD_CLOEXEC | MFD_ALLOW_SEALING)`. Then `ftruncate` to ring size. Passed to child via `SCM_RIGHTS` on UDS. **Preferred** — no `/dev/shm` race, no filesystem naming, sealed against resize/write by parent after plugin has mapped.
- macOS: `shm_open` + `ftruncate` + `mmap`. Named path `nebula.shm.<uuid>` in namespace, unlinked immediately after both parent and child map.
- Windows: `CreateFileMapping(INVALID_HANDLE_VALUE, PAGE_READWRITE, ...)` with named `Local\nebula-shm-<uuid>`, inherited by child via handle duplication.

### Layout

One `memfd` / shm segment per media stream, allocated by the broker. Layout:

```
┌─────────────────────────────────────────┐  offset 0
│ RingHeader (cache-line aligned, 64B)    │
│   magic: u32 = 0xNEB_1                  │
│   version: u16 = 1                      │
│   slot_count: u16                       │
│   slot_size: u32                        │
│   write_idx: AtomicU64   (host-written) │
│   read_idx:  AtomicU64   (plugin-written) │
│   format: StreamFormat                  │
│   _pad: [u8]                            │
├─────────────────────────────────────────┤
│ Slot 0                                  │
│   SlotHeader (32B)                      │
│     sequence: u64                       │
│     timestamp_ns: u64                   │
│     length: u32                         │
│     flags: u32 (KEYFRAME, DROPPED, ...) │
│     _pad: u64                           │
│   payload: [u8; slot_size - 32]         │
├─────────────────────────────────────────┤
│ Slot 1                                  │
│ ...                                     │
└─────────────────────────────────────────┘
```

- **Single producer (host), single consumer (plugin)**. Lock-free via atomic indices.
- **Drop-on-overflow:** if host's `write_idx - read_idx == slot_count`, host increments both and marks the new slot `DROPPED`. Never blocks on the plugin.
- **Backpressure signal:** plugin reads `write_idx`, walks from `read_idx` forward; if `write_idx - read_idx > high_water`, plugin is lagging and host emits `sandbox_media_frame_dropped_total` metric.
- **Sealing (Linux):** after child maps the ring, parent calls `F_ADD_SEALS | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL` so the child cannot resize or reseal. Child gets read-only mapping via `mmap(MAP_SHARED | PROT_READ)`.
- **Wake primitive:** `eventfd` on Linux, `dispatch_source` on macOS, `SetEvent` on Windows. One eventfd per ring. Host signals on `write_idx` advance; plugin blocks on read with timeout. Plugin never polls the ring header in a hot loop.

### Open sequence

```
Plugin           Broker                        OS / Tauri
  │                │                               │
  │ rpc_call       │                               │
  │ media.camera.  │                               │
  │   open{...} ──►│ check OS app-level grant      │
  │                │ (scope, fps, resolution caps) │
  │                │                               │
  │                │ device.open() ───────────────►│ TCC / Privacy / Portal
  │                │                               │ (may prompt user)
  │                │◄─────────────── ok(handle) ───│
  │                │                               │
  │                │ memfd_create, ftruncate,      │
  │                │ seal, eventfd_create          │
  │                │                               │
  │                │ spawn capture task →          │ read frames,
  │                │                               │ write to ring,
  │                │                               │ signal eventfd
  │                │                               │
  │ rpc_response   │                               │
  │ { ref, fds:    │                               │
  │   [shm,        │                               │
  │    eventfd] } ◄│                               │
  │                │                               │
  │ mmap(ro)       │                               │
  │ wait(eventfd)  │                               │
  │ read frame                                     │
  │ ...                                            │
```

`MediaStreamRef` is an opaque handle in the plugin (like `CredentialRef` from Phase 1). Under the hood it carries the fd indices that the plugin received via `SCM_RIGHTS` (Linux/macOS) or `DuplicateHandle` (Windows).

### Close / revocation

Always host-driven. Four triggers:

1. **Plugin closes:** `media.camera.close { ref }` RPC → broker drops capture task, unmaps, unlinks.
2. **Cancellation:** `ActionContext` token fires → broker tears down all open streams for this invocation.
3. **Grant revoked mid-stream:** user hits "revoke camera" in desktop UI → broker tears down, plugin receives one-way `stream_ended { ref, reason: "revoked" }` envelope. Any subsequent `media.camera.read` returns `STREAM_CLOSED`.
4. **Invocation ends:** `action_result` arrives → all refs cleared.

`nebula-plugin-sdk` wraps this for authors: `MediaStream::next_frame() -> Result<Frame, StreamEnded>`. `StreamEnded::Revoked` is how plugins learn about live revocation; plugins must handle it gracefully (return `ActionError::Cancelled` or retry differently).

## Control-event streams (keyboard, mouse, global shortcuts)

Different from media: low volume, need low latency but not high throughput. No shm. Use the broker's existing one-way `event` envelope from Phase 1, with backpressure by bounded mpsc channel (drop oldest).

### Global shortcuts (`global_shortcut.register`)

- Scope: allowlist of key combos in manifest.
- `global-shortcut.register { combo }` — broker registers with OS (macOS `registerEventHotKey`, Windows `RegisterHotKey`, Linux via portal), returns a handle.
- Triggered combo emits one-way `event { kind: "global-shortcut.fired", combo, timestamp_ns }` to the plugin.
- Unregister on invocation end.

### Keyboard / mouse

**Two sub-scopes:** `app` and `global`, passed as argument to `input.keyboard.subscribe { scope }`:

```rust
ctx.input().keyboard_subscribe(KeyboardScope::App).await?;     // focused-window only
ctx.input().keyboard_subscribe(KeyboardScope::Global).await?;  // 🚨 keylogger territory
```

- **`app` scope** — only valid for plugins that own a window (rare for server plugins, relevant for desktop plugins that are UI surfaces). Events delivered when the plugin's own Tauri webview has focus. Low risk.
- **`global` scope** — high risk. Broker emits `SandboxHighRiskOp { plugin, op: "input.keyboard.global" }` on every subscribe — operators can monitor/alert. Tray indicator permanently visible while active. One-click uninstall from the plugin-management view.
- Events are rate-limited: max 1000/s per stream, drop-oldest on overflow. Keystroke events are coalesced into batches of 10 or 16 ms windows.
- **Linux `global` scope requires root or uaccess** — most distros deny by default. Broker refuses with `DEVICE_NOT_AVAILABLE { reason: "uaccess_missing" }` on unprivileged Linux.
- **macOS `global` scope requires Accessibility TCC** — the host app (nebula-desktop) must have been granted Accessibility in System Settings → Privacy. If not, broker refuses. No per-plugin prompt.
- **Windows `global` scope uses SetWindowsHookEx(WH_KEYBOARD_LL)** — flagged by some AV software as suspicious. Document the false positive path.

Tray indicator: **always visible** while any `global` input stream is active, with plugin name. Click to view plugin info and uninstall.

## Tray / menubar indicators

Owned by the desktop app, not the plugin. The broker emits `indicator:set / clear { kind, plugin_key }` events that desktop consumes. Kinds:

- `recording_camera` — camera active, red dot in tray
- `recording_microphone` — mic active, red dot
- `recording_screen` — screen capture, amber dot
- `reading_keyboard_global` — 🚨 red flag
- `reading_mouse_global` — 🚨 red flag
- `registered_global_shortcut` — small icon, click for list

Indicator shows plugin name, last active time, and revoke button. Plugin cannot hide or suppress the indicator — it's a host-level UI primitive.

## OS consent matrix

Required OS-level grants for nebula-desktop as the host app — these are **app-level**, not per-plugin:

| Verb | macOS | Windows | Linux (desktop) | Linux (headless) |
|---|---|---|---|---|
| `media.camera.open` | TCC (Camera) | Privacy → Camera | Portal (PipeWire) | — (refuse) |
| `media.microphone.open` | TCC (Microphone) | Privacy → Microphone | Portal (PipeWire) | — (refuse) |
| `media.screen.open` | TCC (Screen Recording) | Graphics Capture API | Portal (screencast) | — (refuse) |
| `clipboard.read` | none | none | none | via daemon if avail |
| `clipboard.write` | none | none | none | via daemon if avail |
| `global_shortcut.register` | none | none | Portal (global-shortcuts) | — |
| `input.keyboard.subscribe` (app) | none | none | Wayland input method | — |
| `input.keyboard.subscribe` (global) | TCC (Accessibility) | (no prompt, hook) | uaccess / root (refuse if none) | — (refuse) |
| `input.mouse.subscribe` (global) | TCC (Accessibility) | (no prompt, hook) | uaccess / root (refuse if none) | — (refuse) |
| `notifications.emit` | none | none | none | libnotify if avail |
| `geolocation.query` | TCC (Location) | Privacy → Location | Portal (location) | — (refuse) |

**We never replicate OS consent UI.** If the OS prompts, the OS prompts. We wait for the result and surface it to the plugin as either a live `MediaStreamRef` (on grant) or `DEVICE_NOT_AVAILABLE` (on denial). Operator is responsible for granting OS consent to nebula-desktop at the right time (usually first use).

## Work breakdown

1. **Device verb definitions** — add `media.*`, `clipboard.*`, `input.*`, `global_shortcut.*`, `notifications.emit`, `geolocation.query` to the gRPC proto in `nebula-plugin-protocol`. Plugin-SDK wrappers for typed invocation. 1-2 days.
2. **Shared memory ring abstraction** — new `crates/sandbox/src/shm.rs`. `MemfdRing` on Linux, `ShmRing` on macOS, `FileMappingRing` on Windows. Lock-free SPSC with atomic indices. 4-5 days.
3. **Eventfd / dispatch_source / event wake primitive** — platform shim. 2 days.
4. **Fd passing over UDS** — `sendmsg(SCM_RIGHTS)` on Linux/macOS, `DuplicateHandle` on Windows. Integrate with Phase 1 stdio transport as a sidechannel (parallel UDS to child for fd handover, only for device verbs). 3 days.
5. **`DeviceBroker` trait + `Native` and `Tauri` impls** — `crates/sandbox/src/devices/mod.rs`. 2-3 days.
6. **Camera native backend** — PipeWire on Linux, AVCaptureSession on macOS, Windows.Media.Capture on Windows. 5-7 days (hardest).
7. **Microphone native backend** — CoreAudio / WASAPI / PipeWire. 3-4 days.
8. **Clipboard native backend** — 1-2 days.
9. **Global-shortcut native backend** — 2-3 days.
10. **Keyboard / mouse event backend** — Core Graphics event tap / SetWindowsHookEx / evdev. Rate limiting, coalescing. 4-5 days.
11. **Notifications native backend** — 1 day.
12. **Geolocation native backend** — 1-2 days.
13. **Tauri delegation layer** — detect runtime, route to `AppHandle`, verify permission subset. 3-4 days.
14. **Tray indicator protocol** — broker emits, desktop renders. 2 days.
15. **Grant UI red-flag rendering** — Tauri frontend work. 2-3 days.
16. **Revocation plumbing** — live teardown of shm rings and event streams on grant-revoked. 2 days.
17. **`nebula-plugin-sdk`** — ergonomic wrappers for `MediaStream`, `ShortcutSubscription`, `ClipboardHandle`. 2-3 days.
18. **Adversarial device test suite** — request un-granted devices, resize shm after seal, overflow ring faster than plugin reads, revoke mid-stream, TCC denial handling. 3-4 days.
19. **Example plugins** under `examples/`:
    - `sandbox-qr-scanner` — camera + barcode processing (headless barcode logic, shm frames)
    - `sandbox-voice-note` — microphone + transcription via broker `network.http_request`
    - `sandbox-clipboard-history` — clipboard read/write
    - `sandbox-global-hotkey` — global shortcut
    3-4 days total.
20. **Docs** — Phase 5 guarantees, consent flow, tray indicators, revocation semantics. 2 days.

**Total:** ~45-55 working days. Biggest unknowns are camera backend (platform matrix) and Tauri delegation integration (depends on desktop app state).

## Acceptance criteria

- [ ] Adversarial suite: host app without OS TCC grant, plugin calls `media.camera.open` → broker returns `DEVICE_NOT_AVAILABLE { reason: "host_not_granted" }`. Plugin receives shm fd, calls `mmap(PROT_WRITE)` → EACCES (sealed). Plugin reads at 30 fps while host writes at 60 fps → drop metric increments, no ordering violations. Plugin uninstalled mid-stream → plugin gets `StreamEnded::Revoked` within 100 ms.
- [ ] Consent flow: on macOS, first `media.camera.open` from any plugin triggers TCC prompt for nebula-desktop (not for the plugin). Denial propagates as `DEVICE_NOT_AVAILABLE { reason: "os_consent_refused" }`.
- [ ] Tray indicator appears within 200 ms of any camera/mic/screen/global-keyboard stream activation, persists until all streams of that kind are closed, and shows the plugin name.
- [ ] Broker refuses device verb when the host app (nebula-desktop) does not hold the corresponding OS-level grant; plugin receives `DEVICE_NOT_AVAILABLE { reason: "host_not_granted" }` with actionable message.
- [ ] Keyboard `global` scope cannot be enabled on unprivileged Linux; `nebula plugin install` refuses with a clear message.
- [ ] At least one example plugin works end-to-end on each of Linux, macOS, Windows in desktop mode.
- [ ] 30 fps, 1080p camera stream through shm ring uses <2 % CPU on the plugin side (zero-copy verification).
- [ ] Live revocation tears down the shm ring and emits `SandboxMediaRevoked` event on the EventBus; in-flight `media.camera.read` calls return `STREAM_CLOSED` within the configured grace window.
- [ ] `cargo nextest run --workspace` green on the CI matrix (Linux, macOS, Windows) for all device backends with mocked OS APIs.

## Risks

| Risk | Mitigation |
|---|---|
| PipeWire dependency on Linux is heavy and version-sensitive | Gate `camera:*`/`microphone:*` behind availability probe; refuse cleanly if absent |
| `memfd_create` not present on older Linux (pre-3.17) | Already require 5.13+ for landlock; memfd is much older — non-issue |
| macOS TCC prompts are per-app and nebula-desktop is the app — prompts appear on first use, not at plugin install | Document explicitly in plugin-user guide; plugin install dialog notes "first use of camera/mic/screen may trigger an OS prompt for nebula-desktop" |
| Windows `SetWindowsHookEx` flagged by AV | Document false-positive path; ship signed binary (Phase 4 flow) |
| Sealed memfd not supported on older kernels | Fallback to unsealed + defensive mapping (host re-checks size after every write); known limitation on kernel < 3.17 (irrelevant per above) |
| Shm ring write/read indices drift under contention | SPSC invariant — single writer, single reader, atomic fences. Property test in adversarial suite. |
| Plugin maps shm then fd-passes to a child it somehow spawned | Child processes are seccomp/namespace-denied from Phase 2; can't happen |
| Tauri capability not yet granted to desktop shell when plugin tries to use it | Grant UI refuses to enable the plugin permission; user must first grant at the app level |
| Revocation race: plugin is mid-read when host unmaps | Host signals `stream_ended` before unmapping; grace window (configurable, default 500 ms) for plugin to stop; then host unmaps anyway, plugin's read returns `SIGBUS` handled as `STREAM_CLOSED` (with `SA_SIGINFO` handler on Unix, structured exception on Windows) |
| Global keyboard is a keylogger by design | Red-flag UI, persistent tray indicator, one-click revoke, unprivileged-Linux refusal, no ability for plugin to suppress the indicator. Document threat model. |

## Deliverables

- `crates/sandbox/src/shm.rs` — cross-platform SPSC shm ring.
- `crates/sandbox/src/devices/` — `mod.rs`, `native/{camera,microphone,clipboard,shortcut,input,notification,geolocation}.rs`, `tauri.rs`.
- `crates/sandbox/src/indicator.rs` — tray indicator protocol (events only; desktop app renders).
- `nebula-plugin-sdk` additions: `MediaStream`, `ClipboardHandle`, `ShortcutSubscription`, `InputStream`.
- `examples/sandbox-qr-scanner/`, `examples/sandbox-voice-note/`, `examples/sandbox-clipboard-history/`, `examples/sandbox-global-hotkey/`.
- Updated plugin-management view in `apps/desktop` with tray indicators for active device streams, per-stream "in use by" panel, and one-click uninstall.
- `.project/context/crates/sandbox.md` — Phase 5 guarantees section, consent matrix, revocation semantics.
- Adversarial test suite: `crates/sandbox/tests/adversarial_devices.rs`.

## Open questions to resolve during implementation

1. **Screen capture mode granularity.** `screen:capture = { mode = "window" | "region" | "full" }` — do we expose per-window selection UI (user picks which window each invocation), or grant-time fixed? Recommend: grant-time scope is `window|region|full`, per-invocation user picks which specific window via OS picker (macOS ScreenCaptureKit window picker, Windows Graphics Capture picker, Portal screencast dialog).
2. **Audio format negotiation.** Plugin declares `sample-rates = [16000, 44100, 48000]`, hardware provides 48000. Do we resample in broker or return error? Recommend: broker resamples to first listed rate the plugin accepts; cost is host-side, measured.
3. **Multi-stream coordination.** Can a plugin hold camera + mic + screen simultaneously? Recommend: yes, each is a separate `MediaStreamRef`, indicators stack, revocation is per-stream.
4. **`nebula plugin install --dev`** — dev mode can bypass `global` keyboard refusal on Linux? Recommend: yes, but with prominent `--unsafe-global-input` flag + runtime warning every 60s. Production plugins cannot ship with this.
5. **Headless server `screen:capture`** — does this make sense? Recommend: refuse outright. `camera:*`/`microphone:*`/`screen:*` all refuse in headless Linux without a running portal service.
