# Guard Hooks Subsystem Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add harness-enforced, evasion-hardened guard hooks to `.claude/` so the agent cannot weaken tests, suppress lints, bypass `lefthook`, or claim "done" without a verified gate.

**Architecture:** Six Node.js `.mjs` hooks wired in committed `.claude/settings.json` (args[] exec form), sharing a turn-state file under the git common-dir. `PreToolUse/Bash` denies bypass commands; `PreToolUse/Edit` denies cheat/costyl symptoms; `PostToolUse/Bash` records green gates; `Stop` blocks completion when impl changed without a recorded green gate; `UserPromptSubmit` resets turn-state; `PostToolUse/Edit` formats the touched file. Each hook fails open on internal error and is covered by `node --test`.

**Tech Stack:** Node.js 22 (built-in `node --test`, ESM `.mjs`), Claude Code hooks (`permissionDecision`/exit-2 contracts), git, Taskfile.

**Plan series (this is Plan 1 of 4 — spec `docs/superpowers/specs/2026-05-16-agent-discipline-and-curation-design.md` §10):**
1. **Guard Hooks Subsystem** (this plan) — A0/A/A2/B/C/D + tests + wiring. Self-contained; ships `task hooks:test`.
2. D8 doc-canon inversion (CLAUDE.md canonical, AGENTS.md pointer, `.cursor`/`.github` cross-refs).
3. Skill curation G + subagent curation H (joint).
4. `lefthook.yml` commit-granularity (F) + `nebula-pitfalls` symptom skill (E).

This plan does **not** depend on Plans 2–4. The §9 "Enforced Discipline" doc section is added to `CLAUDE.md` here as a plain file edit; Plan 2 restructures canon around it.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `.claude/hooks/guard-lib.mjs` | Shared: stdin read, Bash tokenizer + evasion-strip, turn-state path/load/save, deny helpers, rust-file classification |
| `.claude/hooks/nebula-guard-turn-reset.mjs` | A0 — `UserPromptSubmit`: reset turn-state |
| `.claude/hooks/nebula-guard-bash.mjs` | A — `PreToolUse/Bash`: deny `--no-verify`, clippy `-A`, `cargo fmt --all`, force-push |
| `.claude/hooks/nebula-guard-record.mjs` | A2 — `PostToolUse/Bash`: record green clippy+nextest per crate |
| `.claude/hooks/nebula-guard-edit.mjs` | B — `PreToolUse/Edit\|Write\|MultiEdit`: deny cheat/costyl symptoms + test-weakening |
| `.claude/hooks/nebula-guard-stop.mjs` | C — `Stop`: block done without recorded green gate |
| `.claude/hooks/nebula-guard-fmt.mjs` | D — `PostToolUse/Edit\|Write\|MultiEdit`: format touched file |
| `.claude/hooks/__tests__/*.test.mjs` | `node --test` deny-bad / allow-good per hook |
| `.claude/settings.json` | Committed wiring (args[] form) + `$schema` + curated permissions |
| `Taskfile.yml` | `task hooks:test` target |
| `CLAUDE.md` | "Enforced Discipline" section (rule → guard map) |

**Convention for all hooks:** read stdin JSON; on any internal exception print nothing useful and `process.exit(0)` (fail open — a broken guard must never wedge the session); never exceed ~2 s. Deny = structured output, never a thrown error.

---

### Task 1: Shared library `guard-lib.mjs`

**Files:**
- Create: `.claude/hooks/guard-lib.mjs`
- Test: `.claude/hooks/__tests__/guard-lib.test.mjs`

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/guard-lib.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseBash, crateOf, isLibRust } from "../guard-lib.mjs";

test("parseBash strips inline env and wrappers", () => {
  const r = parseBash('FOO=1 env BAR=2 sudo cargo clippy -- -D warnings');
  assert.equal(r.argv0, "cargo");
  assert.deepEqual(r.args, ["clippy", "--", "-D", "warnings"]);
});

test("parseBash cuts at redirect and pipe", () => {
  const r = parseBash('cargo fmt --all 2>&1 | tee log.txt');
  assert.equal(r.argv0, "cargo");
  assert.deepEqual(r.args, ["fmt", "--all"]);
});

test("crateOf extracts crate name", () => {
  assert.equal(crateOf("crates/engine/src/engine.rs"), "engine");
  assert.equal(crateOf("README.md"), null);
});

test("isLibRust excludes tests/benches/examples", () => {
  assert.equal(isLibRust("crates/engine/src/state.rs"), true);
  assert.equal(isLibRust("crates/engine/tests/retry.rs"), false);
  assert.equal(isLibRust("crates/engine/src/main.rs"), false);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/guard-lib.test.mjs`
Expected: FAIL — `Cannot find module '../guard-lib.mjs'`.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/guard-lib.mjs
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";

export async function readStdin() {
  try {
    const chunks = [];
    for await (const c of process.stdin) chunks.push(c);
    return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
  } catch {
    return {};
  }
}

const WRAPPERS = new Set(["env", "sudo", "nice", "timeout", "watch", "xargs", "command", "stdbuf", "nohup"]);
const CUTTERS = new Set(["|", "||", "&&", ";", "&", ">", ">>", "<", "2>", "2>&1", "1>&2", "|&"]);

function tokenize(cmd) {
  const out = [];
  let cur = "";
  let q = null;
  for (let i = 0; i < cmd.length; i++) {
    const ch = cmd[i];
    if (q) {
      if (ch === q) q = null;
      else cur += ch;
    } else if (ch === '"' || ch === "'") {
      q = ch;
    } else if (/\s/.test(ch)) {
      if (cur) { out.push(cur); cur = ""; }
    } else {
      cur += ch;
    }
  }
  if (cur) out.push(cur);
  return out;
}

export function parseBash(command) {
  let toks = tokenize(String(command || ""));
  // cut at first shell control / redirect operator
  const cut = toks.findIndex((t) => CUTTERS.has(t) || t.startsWith(">") || t.startsWith("2>"));
  if (cut !== -1) toks = toks.slice(0, cut);
  // strip leading VAR=val and wrapper commands (incl. env VAR=val)
  let i = 0;
  while (i < toks.length) {
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(toks[i]) && !toks[i].includes("/")) { i++; continue; }
    const base = toks[i].split("/").pop();
    if (WRAPPERS.has(base)) {
      i++;
      while (i < toks.length && toks[i].startsWith("-")) i++; // wrapper flags
      continue;
    }
    break;
  }
  toks = toks.slice(i);
  return { argv0: (toks[0] || "").split("/").pop(), args: toks.slice(1), raw: String(command || "") };
}

export function crateOf(p) {
  const m = String(p || "").replace(/\\/g, "/").match(/(?:^|\/)crates\/([^/]+)\//);
  return m ? m[1] : null;
}

export function isLibRust(p) {
  const f = String(p || "").replace(/\\/g, "/");
  if (!f.endsWith(".rs")) return false;
  if (!/\/crates\/[^/]+\/src\//.test(f) && !/^crates\/[^/]+\/src\//.test(f)) return false;
  if (/\/(tests|benches|examples)\//.test(f)) return false;
  if (/\/(main|build)\.rs$/.test(f)) return false;
  return true;
}

export function turnStatePath(sessionId, cwd) {
  let base;
  try {
    const g = execFileSync("git", ["rev-parse", "--git-common-dir"], {
      cwd: cwd || process.cwd(), encoding: "utf8",
    }).trim();
    base = isAbsolute(g) ? g : resolve(cwd || process.cwd(), g);
  } catch {
    base = join(tmpdir(), "nebula-guard");
  }
  return join(base, ".nebula-guard", `turn-${sessionId || "unknown"}.json`);
}

export function loadState(p) {
  try { return JSON.parse(readFileSync(p, "utf8")); }
  catch { return { impl_files_edited: [], gate_green: [] }; }
}

export function saveState(p, s) {
  try { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(s)); }
  catch { /* fail open */ }
}

export function denyPre(reason) {
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: reason,
    },
  }));
  process.exit(0);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/guard-lib.test.mjs`
Expected: PASS — 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/guard-lib.mjs .claude/hooks/__tests__/guard-lib.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): guard-lib shared hook utilities"
```

Expected lefthook: `typos` runs (pass); fmt-check/clippy/taplo/cargo-deny skip (no `.rs`/`.toml`); `convco` passes.

---

### Task 2: A0 — turn-reset hook (`UserPromptSubmit`)

**Files:**
- Create: `.claude/hooks/nebula-guard-turn-reset.mjs`
- Test: `.claude/hooks/__tests__/turn-reset.test.mjs`

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/turn-reset.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { turnStatePath } from "../guard-lib.mjs";

test("turn-reset clears prior state for the session", () => {
  const sid = "test-sess-A0";
  const p = turnStatePath(sid, process.cwd());
  mkdirSync(p.replace(/[^/\\]+$/, ""), { recursive: true });
  writeFileSync(p, JSON.stringify({ impl_files_edited: ["x.rs"], gate_green: ["engine"] }));
  execFileSync("node", [".claude/hooks/nebula-guard-turn-reset.mjs"], {
    input: JSON.stringify({ session_id: sid, cwd: process.cwd() }),
  });
  const s = JSON.parse(readFileSync(p, "utf8"));
  assert.deepEqual(s.impl_files_edited, []);
  assert.deepEqual(s.gate_green, []);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/turn-reset.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-turn-reset.mjs
import { readStdin, turnStatePath, saveState } from "./guard-lib.mjs";

const inp = await readStdin();
try {
  const p = turnStatePath(inp.session_id, inp.cwd);
  saveState(p, {
    session: inp.session_id || "unknown",
    started_at: new Date().toISOString(),
    impl_files_edited: [],
    gate_green: [],
  });
} catch { /* fail open */ }
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/turn-reset.test.mjs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-turn-reset.mjs .claude/hooks/__tests__/turn-reset.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A0 UserPromptSubmit turn-state reset hook"
```

---

### Task 3: A — Bash deny guard (`PreToolUse/Bash`)

**Files:**
- Create: `.claude/hooks/nebula-guard-bash.mjs`
- Test: `.claude/hooks/__tests__/bash-guard.test.mjs`

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/bash-guard.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

function run(command, env = {}) {
  let out = "";
  try {
    out = execFileSync("node", [".claude/hooks/nebula-guard-bash.mjs"], {
      input: JSON.stringify({ tool_name: "Bash", tool_input: { command }, cwd: process.cwd() }),
      encoding: "utf8",
      env: { ...process.env, ...env },
    });
  } catch (e) { out = e.stdout || ""; }
  return out;
}
const denied = (o) => o.includes('"permissionDecision":"deny"');

test("denies git commit --no-verify (even wrapped)", () => {
  assert.ok(denied(run('env X=1 git commit -m wip --no-verify')));
});
test("denies clippy lint suppression", () => {
  assert.ok(denied(run('cargo clippy -p nebula-engine -- -A clippy::all')));
});
test("denies cargo fmt --all", () => {
  assert.ok(denied(run('cargo fmt --all')));
});
test("allows a normal cargo nextest run", () => {
  assert.equal(run('cargo nextest run -p nebula-engine').trim(), "");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/bash-guard.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-bash.mjs
import { readStdin, parseBash, denyPre } from "./guard-lib.mjs";

const inp = await readStdin();
if (inp.tool_name !== "Bash") process.exit(0);
const cmd = (inp.tool_input && inp.tool_input.command) || "";
let p;
try { p = parseBash(cmd); } catch { process.exit(0); }
const { argv0, args, raw } = p;
const has = (re) => re.test(raw);

// 1. lefthook bypass
if (argv0 === "git" && args[0] === "commit" &&
    (args.includes("--no-verify") || args.includes("-n") || args.includes("--no-gpg-sign") ||
     args.some((a, i) => a === "-c" && /core\.hooksPath=/.test(args[i + 1] || "")))) {
  denyPre("Bypassing lefthook is the top-level cheat. Run `git commit` without --no-verify/-n/--no-gpg-sign. Fix what the hook flags.");
}
// 2. lint suppression at the command level
if (argv0 === "cargo" && args.includes("clippy") &&
    (args.some((a) => a === "-A" || a === "--allow") || /(^|\s)RUSTFLAGS=.*-A\s/.test(raw))) {
  denyPre("Silencing clippy with -A/--allow/RUSTFLAGS to reach green is cheating the oracle. Fix the lint or add a justified #[allow] in code.");
}
// 3. cargo fmt --all (Windows os-error-206 footgun + false green)
if (argv0 === "cargo" && args.includes("fmt") && args.includes("--all")) {
  denyPre("`cargo fmt --all` trips Windows os-error-206 from worktrees and reports false green. Use `bash scripts/pre-commit-fmt-check.sh` or `cargo fmt -p <crate>`.");
}
// 4. force-push to shared history (env override: NEBULA_ALLOW_FORCE=1)
if (argv0 === "git" && args[0] === "push" &&
    args.some((a) => a === "--force" || a === "-f" || a.startsWith("--force-with-lease")) &&
    process.env.NEBULA_ALLOW_FORCE !== "1") {
  denyPre("Force-push to shared history is blocked (AGENTS.md). Set NEBULA_ALLOW_FORCE=1 only if you truly mean it.");
}
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/bash-guard.test.mjs`
Expected: PASS — 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-bash.mjs .claude/hooks/__tests__/bash-guard.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A PreToolUse Bash deny guard"
```

---

### Task 4: A2 — gate-green recorder (`PostToolUse/Bash`)

**Files:**
- Create: `.claude/hooks/nebula-guard-record.mjs`
- Test: `.claude/hooks/__tests__/record.test.mjs`

> **Known limitation (document, do not hide):** `PostToolUse` exposes `tool_response` but not a guaranteed exit code. A2 records a crate green only when the command is a recognized gate command AND `tool_response` shows no failure signal (`error`, `FAILED`, `warning:`, `test result: FAILED`). This is a heuristic; the Stop gate (Task 6) is the backstop, and a false "green" still requires the agent to have actually run the gate command.

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/record.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { turnStatePath } from "../guard-lib.mjs";

function seed(sid) {
  const p = turnStatePath(sid, process.cwd());
  mkdirSync(p.replace(/[^/\\]+$/, ""), { recursive: true });
  writeFileSync(p, JSON.stringify({ impl_files_edited: [], gate_green: [] }));
  return p;
}
function run(sid, command, response) {
  execFileSync("node", [".claude/hooks/nebula-guard-record.mjs"], {
    input: JSON.stringify({ tool_name: "Bash", tool_input: { command }, tool_response: response, session_id: sid, cwd: process.cwd() }),
  });
}

test("records crate green on passing nextest", () => {
  const sid = "test-A2-ok"; const p = seed(sid);
  run(sid, "cargo nextest run -p nebula-engine", "Summary 12 tests run: 12 passed");
  assert.deepEqual(JSON.parse(readFileSync(p, "utf8")).gate_green, ["engine"]);
});

test("does NOT record on failing output", () => {
  const sid = "test-A2-fail"; const p = seed(sid);
  run(sid, "cargo clippy -p nebula-engine -- -D warnings", "warning: unused variable\nerror: aborting");
  assert.deepEqual(JSON.parse(readFileSync(p, "utf8")).gate_green, []);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/record.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-record.mjs
import { readStdin, parseBash, crateOf, turnStatePath, loadState, saveState } from "./guard-lib.mjs";

const inp = await readStdin();
if (inp.tool_name !== "Bash") process.exit(0);
try {
  const cmd = (inp.tool_input && inp.tool_input.command) || "";
  const { argv0, args } = parseBash(cmd);
  const resp = typeof inp.tool_response === "string"
    ? inp.tool_response : JSON.stringify(inp.tool_response || "");
  const failed = /(^|\W)(error\b|FAILED|warning:|test result: FAILED)/i.test(resp);

  const isClippy = argv0 === "cargo" && args.includes("clippy") && args.includes("-D");
  const isNextest = argv0 === "cargo" && args.includes("nextest") && args.includes("run");
  const isDevCheck = argv0 === "task" && args.includes("dev:check");
  if (!failed && (isClippy || isNextest || isDevCheck)) {
    const p = turnStatePath(inp.session_id, inp.cwd);
    const s = loadState(p);
    const pIdx = args.indexOf("-p");
    const crate = pIdx !== -1 ? (args[pIdx + 1] || "").replace(/^nebula-/, "") : null;
    const set = new Set(s.gate_green || []);
    if (crate) set.add(crate);
    if (isDevCheck) set.add("*workspace*");
    s.gate_green = [...set];
    saveState(p, s);
  }
} catch { /* fail open */ }
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/record.test.mjs`
Expected: PASS — 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-record.mjs .claude/hooks/__tests__/record.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): A2 PostToolUse gate-green recorder"
```

---

### Task 5: B — edit anti-cheat guard (`PreToolUse/Edit|Write|MultiEdit`)

**Files:**
- Create: `.claude/hooks/nebula-guard-edit.mjs`
- Test: `.claude/hooks/__tests__/edit-guard.test.mjs`

> **Known limitation:** B inspects incoming text (`Write.content` / `Edit.new_string` / `MultiEdit.edits[].new_string`). Inline `#[cfg(test)]` modules inside a lib file may cause a false negative for the unwrap rule (clippy at the gate is the backstop). Test-weakening detection compares `old_string` vs `new_string` assert counts (Edit/MultiEdit only).

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/edit-guard.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdirSync } from "node:fs";
import { turnStatePath } from "../guard-lib.mjs";

function run(payload) {
  let out = "";
  try {
    out = execFileSync("node", [".claude/hooks/nebula-guard-edit.mjs"], {
      input: JSON.stringify({ cwd: process.cwd(), session_id: payload.sid || "edit-t", ...payload }),
      encoding: "utf8",
    });
  } catch (e) { out = e.stdout || ""; }
  return out;
}
const denied = (o) => o.includes('"permissionDecision":"deny"');

test("denies new unwrap() in lib rust", () => {
  assert.ok(denied(run({
    tool_name: "Write",
    tool_input: { file_path: "crates/engine/src/state.rs", content: "fn f(){ let x = g().unwrap(); }" },
  })));
});

test("denies #[allow] without guard-justified", () => {
  assert.ok(denied(run({
    tool_name: "Write",
    tool_input: { file_path: "crates/engine/src/state.rs", content: "#[allow(dead_code)]\nfn f(){}" },
  })));
});

test("allows #[allow] WITH guard-justified", () => {
  assert.equal(run({
    tool_name: "Write",
    tool_input: { file_path: "crates/engine/src/state.rs",
      content: "// guard-justified: FFI shim, lint is a false positive\n#[allow(dead_code)]\nfn f(){}" },
  }).trim(), "");
});

test("denies test weakening when impl edited same turn", () => {
  const sid = "edit-weaken";
  const p = turnStatePath(sid, process.cwd());
  mkdirSync(p.replace(/[^/\\]+$/, ""), { recursive: true });
  writeFileSync(p, JSON.stringify({ impl_files_edited: ["crates/engine/src/state.rs"], gate_green: [] }));
  assert.ok(denied(run({
    sid,
    tool_name: "Edit",
    tool_input: { file_path: "crates/engine/tests/retry.rs",
      old_string: "assert_eq!(got, want);", new_string: "assert!(true);" },
  })));
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/edit-guard.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-edit.mjs
import { readStdin, isLibRust, turnStatePath, loadState, saveState, denyPre } from "./guard-lib.mjs";

const inp = await readStdin();
const tool = inp.tool_name;
if (!["Write", "Edit", "MultiEdit"].includes(tool)) process.exit(0);
const ti = inp.tool_input || {};
const file = (ti.file_path || "").replace(/\\/g, "/");
if (!file) process.exit(0);

let added = "";
if (tool === "Write") added = ti.content || "";
else if (tool === "Edit") added = ti.new_string || "";
else if (tool === "MultiEdit") added = (ti.edits || []).map((e) => e.new_string || "").join("\n");

const isTest = /\/(tests|benches)\//.test(file) || /#\[(cfg\(test\)|test)\]/.test(added);

// track impl edits this turn (non-test rust under src)
const sp = turnStatePath(inp.session_id, inp.cwd);
const st = loadState(sp);
if (isLibRust(file) && !isTest) {
  if (!st.impl_files_edited.includes(file)) st.impl_files_edited.push(file);
  saveState(sp, st);
}

function justified(text, idx) {
  const before = text.slice(0, idx);
  const prevLine = before.split("\n").slice(-2, -1)[0] || before.split("\n").pop() || "";
  return /\/\/\s*guard-justified:/.test(prevLine) || /\/\/\s*guard-justified:/.test(before.split("\n").pop() || "");
}

if (isLibRust(file) && !isTest) {
  if (/\.unwrap\(\)|\.expect\(|(^|\W)panic!\(/.test(added)) {
    denyPre("New unwrap()/expect()/panic!() in library code is forbidden (AGENTS.md). Use a typed thiserror variant.");
  }
  for (const m of added.matchAll(/#\[allow\(|(?:^|\W)(todo!|unimplemented!|unreachable!)\(/g)) {
    if (!justified(added, m.index)) {
      denyPre(`'${m[0].replace(/\(.*/, "")}' is a path-of-least-work escape. Fix it, or justify with a '// guard-justified: <reason>' line directly above.`);
    }
  }
  if (/\/\/\s*(TODO|FIXME|HACK|XXX)\b|TODO\([A-Z]+-?\d|(^|\W)Phase\s[A-Z]\b/.test(added)) {
    denyPre("TODO/FIXME/HACK/plan-id comments must not land in committed code (comments must read fine after the plan is deleted).");
  }
  if (/let\s+_\s*=\s*[\w.]*\b(transition|send|write|commit|flush|lock|spawn)\w*\s*\(/.test(added)) {
    denyPre("`let _ = <call>` silently swallows a Result/must-use. Handle the error explicitly.");
  }
}

// test-integrity: weakening a test while impl changed this turn
if ((tool === "Edit" || tool === "MultiEdit") && /\/(tests|benches)\//.test(file) && st.impl_files_edited.length) {
  const olds = (tool === "Edit" ? [ti.old_string || ""] : (ti.edits || []).map((e) => e.old_string || "")).join("\n");
  const news = (tool === "Edit" ? [ti.new_string || ""] : (ti.edits || []).map((e) => e.new_string || "")).join("\n");
  const cnt = (s) => (s.match(/\bassert\w*!/g) || []).length;
  const weaken = cnt(olds) > cnt(news) || /assert!\(\s*true\s*\)|#\[ignore\]/.test(news) ||
    /assert_eq!\(\s*([A-Za-z_]\w*)\s*,\s*\1\s*\)/.test(news);
  if (weaken) {
    denyPre("Weakening a test (removed assert / #[ignore] / assert!(true) / tautology) while impl changed this turn is blocked. Fix the logic, not the test.");
  }
}
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/edit-guard.test.mjs`
Expected: PASS — 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-edit.mjs .claude/hooks/__tests__/edit-guard.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): B PreToolUse edit anti-cheat guard"
```

---

### Task 6: C — Stop falsifiable-finish gate (`Stop`)

**Files:**
- Create: `.claude/hooks/nebula-guard-stop.mjs`
- Test: `.claude/hooks/__tests__/stop-guard.test.mjs`

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/stop-guard.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdirSync } from "node:fs";
import { turnStatePath } from "../guard-lib.mjs";

function seed(sid, state) {
  const p = turnStatePath(sid, process.cwd());
  mkdirSync(p.replace(/[^/\\]+$/, ""), { recursive: true });
  writeFileSync(p, JSON.stringify(state));
}
function run(sid, stopActive = false) {
  try {
    execFileSync("node", [".claude/hooks/nebula-guard-stop.mjs"], {
      input: JSON.stringify({ session_id: sid, cwd: process.cwd(), stop_hook_active: stopActive }),
      encoding: "utf8",
    });
    return { code: 0, stderr: "" };
  } catch (e) { return { code: e.status, stderr: (e.stderr || "").toString() }; }
}

test("blocks when impl changed but no green gate", () => {
  seed("stop-block", { impl_files_edited: ["crates/engine/src/state.rs"], gate_green: [] });
  const r = run("stop-block");
  assert.equal(r.code, 2);
  assert.match(r.stderr, /never showed clippy \+ nextest green/);
});

test("allows when touched crate is green", () => {
  seed("stop-ok", { impl_files_edited: ["crates/engine/src/state.rs"], gate_green: ["engine"] });
  assert.equal(run("stop-ok").code, 0);
});

test("never re-blocks when stop_hook_active", () => {
  seed("stop-loop", { impl_files_edited: ["crates/engine/src/state.rs"], gate_green: [] });
  assert.equal(run("stop-loop", true).code, 0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/stop-guard.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-stop.mjs
// Side-effect-free: reads turn-state only, runs no tools (deadlock-safe).
import { readStdin, crateOf, turnStatePath, loadState } from "./guard-lib.mjs";

const inp = await readStdin();
if (inp.stop_hook_active === true) process.exit(0); // loop guard
try {
  const s = loadState(turnStatePath(inp.session_id, inp.cwd));
  const touched = [...new Set((s.impl_files_edited || []).map(crateOf).filter(Boolean))];
  if (touched.length === 0) process.exit(0);
  const green = new Set(s.gate_green || []);
  if (green.has("*workspace*")) process.exit(0);
  const missing = touched.filter((c) => !green.has(c));
  if (missing.length) {
    process.stderr.write(
      `You changed crate(s) [${missing.join(", ")}] but never showed clippy + nextest green ` +
      `for them this turn. Run \`cargo clippy -p nebula-<crate> -- -D warnings\` and ` +
      `\`cargo nextest run -p nebula-<crate>\` (or \`task dev:check\`) before claiming done. ` +
      `Weakening tests to get there is blocked by the edit guard.`,
    );
    process.exit(2);
  }
} catch { process.exit(0); } // fail open
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/stop-guard.test.mjs`
Expected: PASS — 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-stop.mjs .claude/hooks/__tests__/stop-guard.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): C Stop falsifiable-finish gate"
```

---

### Task 7: D — post-edit formatter (`PostToolUse/Edit|Write|MultiEdit`)

**Files:**
- Create: `.claude/hooks/nebula-guard-fmt.mjs`
- Test: `.claude/hooks/__tests__/fmt-hook.test.mjs`

- [ ] **Step 1: Write the failing test**

```javascript
// .claude/hooks/__tests__/fmt-hook.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

test("fmt hook always exits 0 and never blocks (non-rust path)", () => {
  const out = execFileSync("node", [".claude/hooks/nebula-guard-fmt.mjs"], {
    input: JSON.stringify({ tool_name: "Write", tool_input: { file_path: "README.md" }, cwd: process.cwd() }),
    encoding: "utf8",
  });
  assert.equal(out, "");
});

test("fmt hook tolerates missing file without throwing", () => {
  const out = execFileSync("node", [".claude/hooks/nebula-guard-fmt.mjs"], {
    input: JSON.stringify({ tool_name: "Write", tool_input: { file_path: "crates/zzz/src/nope.rs" }, cwd: process.cwd() }),
    encoding: "utf8",
  });
  assert.equal(out, "");
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test .claude/hooks/__tests__/fmt-hook.test.mjs`
Expected: FAIL — module not found.

- [ ] **Step 3: Write minimal implementation**

```javascript
// .claude/hooks/nebula-guard-fmt.mjs
// Format-only, single file, never organize-imports (split-edit safe), never blocks.
import { execFileSync } from "node:child_process";
import { readStdin } from "./guard-lib.mjs";

const inp = await readStdin();
try {
  if (!["Write", "Edit", "MultiEdit"].includes(inp.tool_name)) process.exit(0);
  const f = (inp.tool_input && inp.tool_input.file_path) || "";
  const opts = { cwd: inp.cwd || process.cwd(), stdio: "ignore", timeout: 8000 };
  if (f.endsWith(".rs")) {
    try { execFileSync("rustfmt", ["--edition", "2024", f], opts); } catch { /* best effort */ }
  } else if (f.endsWith(".toml")) {
    try { execFileSync("taplo", ["fmt", f], opts); } catch { /* best effort */ }
  }
} catch { /* fail open */ }
process.exit(0);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test .claude/hooks/__tests__/fmt-hook.test.mjs`
Expected: PASS — 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add .claude/hooks/nebula-guard-fmt.mjs .claude/hooks/__tests__/fmt-hook.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): D PostToolUse single-file formatter"
```

---

### Task 8: Wire hooks into committed `.claude/settings.json`

**Files:**
- Create: `.claude/settings.json`

> Hooks arrays concatenate with `.claude/settings.local.json` (personal, BridgeSpace-free after the spec's §7 work). `permissions.allow` is broad **because** guard A is now the real Bash gate. Uses the `args: string[]` exec form per changelog 2.1.121.

- [ ] **Step 1: Create the settings file**

```json
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": [
      "Bash(cargo *)",
      "Bash(cargo nextest *)",
      "Bash(task *)",
      "Bash(git *)",
      "Bash(gh *)",
      "Bash(bash scripts/*)",
      "Bash(node --test *)",
      "Bash(rustfmt *)",
      "Bash(taplo *)"
    ]
  },
  "hooks": {
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-turn-reset.mjs"] } ] }
    ],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-bash.mjs"] } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-edit.mjs"] } ] }
    ],
    "PostToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-record.mjs"] } ] },
      { "matcher": "Edit|Write|MultiEdit", "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-fmt.mjs"] } ] }
    ],
    "Stop": [
      { "hooks": [ { "type": "command", "command": "node", "args": ["$CLAUDE_PROJECT_DIR/.claude/hooks/nebula-guard-stop.mjs"] } ] }
    ]
  }
}
```

- [ ] **Step 2: Validate JSON + schema-shape**

Run: `node -e "const j=require('./.claude/settings.json'); if(!j['$schema']||!j.hooks.PreToolUse.length||!j.hooks.Stop.length) process.exit(1); console.log('settings.json OK')"`
Expected: `settings.json OK`

- [ ] **Step 3: Commit**

```bash
git add .claude/settings.json
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): wire guard hooks in committed settings.json"
```

---

### Task 9: `task hooks:test` target + CLAUDE.md Enforced-Discipline section

**Files:**
- Modify: `Taskfile.yml` (add a `hooks:test` task under `tasks:`)
- Modify: `CLAUDE.md` (append an "Enforced Discipline" section)

- [ ] **Step 1: Add the Taskfile target**

Locate the top-level `tasks:` map in `Taskfile.yml` and add:

```yaml
  hooks:test:
    desc: Run guard-hook unit tests (node --test)
    cmds:
      - node --test .claude/hooks/__tests__
```

- [ ] **Step 2: Verify the target runs all hook tests**

Run: `task hooks:test`
Expected: all `__tests__/*.test.mjs` pass (15 tests across 6 files + lib).

- [ ] **Step 3: Append the Enforced-Discipline section to CLAUDE.md**

Append to `CLAUDE.md`:

```markdown
## Enforced Discipline (guard hooks)

These rules are mechanically enforced by `.claude/hooks/` (committed in
`.claude/settings.json`), not advisory. `task hooks:test` proves each guard
deny-bad / allow-good. Plan 2 makes this file canonical.

| Rule | Guard |
|------|-------|
| No `git commit --no-verify` / lefthook bypass | `nebula-guard-bash.mjs` |
| No clippy `-A`/`--allow`/`RUSTFLAGS` lint suppression | `nebula-guard-bash.mjs` |
| No `cargo fmt --all` (Windows 206 / false green) | `nebula-guard-bash.mjs` |
| No `unwrap()/expect()/panic!()` in lib code | `nebula-guard-edit.mjs` |
| `#[allow]/todo!/unimplemented!/unreachable!` need `// guard-justified:` | `nebula-guard-edit.mjs` |
| No TODO/FIXME/HACK/plan-id in committed code | `nebula-guard-edit.mjs` |
| No test-weakening while impl changed same turn | `nebula-guard-edit.mjs` |
| Cannot end a turn with impl changed but no green clippy+nextest | `nebula-guard-stop.mjs` |

Escape hatch for discretionary edit rules: a `// guard-justified: <reason>`
line directly above the construct. There is **no** escape for the lefthook
bypass, lint-suppression, or no-unwrap rules.
```

- [ ] **Step 4: Commit**

```bash
git add Taskfile.yml CLAUDE.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "feat(scripts): task hooks:test + CLAUDE.md Enforced-Discipline map"
```

---

### Task 10: Integration smoke (acceptance §11)

**Files:**
- Create: `.claude/hooks/__tests__/integration-smoke.test.mjs`

- [ ] **Step 1: Write the smoke test (deny-cheat / allow-clean end to end)**

```javascript
// .claude/hooks/__tests__/integration-smoke.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdirSync } from "node:fs";
import { turnStatePath } from "../guard-lib.mjs";

const callEdit = (payload) => {
  try {
    return execFileSync("node", [".claude/hooks/nebula-guard-edit.mjs"], {
      input: JSON.stringify({ cwd: process.cwd(), ...payload }), encoding: "utf8",
    });
  } catch (e) { return e.stdout || ""; }
};

test("cheat path: edit impl then neuter a test → denied", () => {
  const sid = "smoke-cheat";
  callEdit({ session_id: sid, tool_name: "Write",
    tool_input: { file_path: "crates/engine/src/state.rs", content: "pub fn add(a:i32,b:i32)->i32{a+b}" } });
  const out = callEdit({ session_id: sid, tool_name: "Edit",
    tool_input: { file_path: "crates/engine/tests/state.rs",
      old_string: "assert_eq!(add(2,2), 4);", new_string: "assert!(true);" } });
  assert.ok(out.includes('"permissionDecision":"deny"'));
});

test("clean path: well-formed impl edit → allowed", () => {
  const sid = "smoke-clean";
  const p = turnStatePath(sid, process.cwd());
  mkdirSync(p.replace(/[^/\\]+$/, ""), { recursive: true });
  writeFileSync(p, JSON.stringify({ impl_files_edited: [], gate_green: [] }));
  const out = callEdit({ session_id: sid, tool_name: "Write",
    tool_input: { file_path: "crates/engine/src/state.rs",
      content: "pub fn add(a: i32, b: i32) -> i32 { a + b }" } });
  assert.equal(out.trim(), "");
});
```

- [ ] **Step 2: Run the full suite**

Run: `task hooks:test`
Expected: PASS — all files including `integration-smoke` green.

- [ ] **Step 3: Commit**

```bash
git add .claude/hooks/__tests__/integration-smoke.test.mjs
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "test(scripts): guard-hooks integration smoke (cheat denied / clean allowed)"
```

---

## Self-Review

**1. Spec coverage (§ → task):**
- §4.A0 turn-reset → Task 2 ✓
- §4.A Bash deny (no-verify, clippy -A, fmt --all, force-push) → Task 3 ✓
- §4.A2 gate-green record → Task 4 ✓ (limitation documented)
- §4.B edit anti-cheat (unwrap, allow/todo without justified, TODO/plan-id, `let _ =` swallow, test-weakening) → Task 5 ✓
- §4.C Stop falsifiable finish + `stop_hook_active` guard + side-effect-free → Task 6 ✓
- §4.D post-edit fmt (rustfmt --edition 2024 / taplo, single file, never block) → Task 7 ✓
- §4 settings wiring (args[] form, $schema, concatenation, broad permissions) → Task 8 ✓
- §8.1 `__tests__` deny-bad/allow-good + `task hooks:test` → Tasks 1–10 ✓
- §8.2 CLAUDE.md rule→guard map → Task 9 ✓
- §11 acceptance (scripted cheat denied, clean allowed) → Task 10 ✓
- Out of scope for Plan 1 (correctly): D8 inversion, G/H curation, lefthook granularity, `nebula-pitfalls`, full `permissions.allow` cleanup → Plans 2–4.

**2. Placeholder scan:** No TBD/TODO-as-instruction; every code step contains complete runnable code; every command has expected output. (Literal `TODO` strings appear only as regex content the guard detects.)

**3. Type consistency:** `guard-lib.mjs` exports `readStdin, parseBash, crateOf, isLibRust, turnStatePath, loadState, saveState, denyPre` — all consumed with those exact names/signatures in Tasks 2–7. Turn-state shape `{session, started_at, impl_files_edited[], gate_green[]}` is written by A0 and read consistently by A2/B/C. `*workspace*` sentinel set by A2 (`task dev:check`) is honored by C.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-05-16-guard-hooks-subsystem.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

**Which approach?**
