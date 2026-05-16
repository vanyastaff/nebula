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
// Wrapper flags that take a SEPARATE-token value (so the value is not mistaken
// for the wrapped command). Locked set — changing it affects every guard.
const VALUE_FLAGS = new Set(["-u", "-g", "-n", "-C", "-k", "-s", "-S", "-h", "-d", "-o", "-e", "--user", "--group", "--signal", "--chdir"]);
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
  const cut = toks.findIndex((t) => CUTTERS.has(t) || t.startsWith(">") || t.startsWith("2>"));
  if (cut !== -1) toks = toks.slice(0, cut);
  let i = 0;
  while (i < toks.length) {
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(toks[i]) && !toks[i].includes("/")) { i++; continue; }
    const base = toks[i].split("/").pop();
    if (WRAPPERS.has(base)) {
      i++;
      // consume the wrapper's argument region: flags (+ their values),
      // bare numeric/duration values (e.g. `timeout 600`, `nice 10`), and
      // env-style VAR=val — stop at the first real command token.
      while (i < toks.length) {
        const t = toks[i];
        if (t.startsWith("-")) {
          i++;
          if (VALUE_FLAGS.has(t) && i < toks.length && !toks[i].startsWith("-")) i++;
          continue;
        }
        if (/^\d+(?:\.\d+)?[smhdKMG]?$/.test(t)) { i++; continue; }
        if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(t) && !t.includes("/")) { i++; continue; }
        break;
      }
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

// Process-terminal: emits the deny JSON and never returns (calls
// process.exit(0)). Hooks are unit-tested as child processes, so the
// terminal exit is observable via stdout/exit without importing it.
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
