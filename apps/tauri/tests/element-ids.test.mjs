import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";

// The vm fakes in the other tests create any id they are asked for, so only this test notices
// when markup moves into a Svelte part and app.js still reaches for an id that is gone.
const tauri = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, tauri), "utf8");

function svelteSources(dir = new URL("ui/src/", tauri)) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? svelteSources(new URL(entry.name + "/", dir))
    : entry.name.endsWith(".svelte") ? [readFileSync(new URL(entry.name, dir), "utf8")] : []);
}

const literalIds = source => [...source.matchAll(/(?:\$|byId)\(\s*(["'`])([^"'`$]+)\1\s*\)/g)].map(match => match[2]);
const declaredIds = source => [...source.matchAll(/\bid="([^"{}]+)"/g)].map(match => match[1]);

test("every id the old scripts look up by name exists in index.html or a Svelte part", () => {
  const ids = new Set([read("dist/index.html"), ...svelteSources()].flatMap(declaredIds));
  for (const script of ["app.js", "menus.js", "docking.js"]) {
    const used = literalIds(read("dist/" + script));
    assert.ok(used.length > 0, script + " looks up no ids; the pattern no longer matches its code");
    assert.deepEqual(used.filter(id => !ids.has(id)), [], script + " looks up ids that no markup declares");
  }
});
