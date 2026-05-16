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
