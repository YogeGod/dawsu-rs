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
// app.js also reaches ids through its setValue, toggle and number helpers and `for (const id of [...])` loops.
const appIds = source => [
  ...[...source.matchAll(/(?<![.\w])(?:setValue|toggle|number)\(\s*"([^"]+)"/g)].map(match => match[1]),
  ...[...source.matchAll(/for \(const id of \[([^\]]*)\]\)/g)]
    .flatMap(match => [...match[1].matchAll(/"([^"]+)"/g)].map(id => id[1])),
];

test("the id guard also reads app.js's helpers and id loops", () => {
  const sample = 'setValue("a", 1); toggle("b", "kind"); number("c", 0); button.classList.toggle("active", on); ' +
    'for (const id of ["d", "e"]) {} for (const type of ["pointerup"]) {}';
  assert.deepEqual(appIds(sample), ["a", "b", "c", "d", "e"]);
});

import { readEditorSource } from "./editor-runtime.mjs";

test("every id the editor scripts look up by name exists in index.html or a Svelte part", () => {
  const ids = new Set([read("dist/index.html"), ...svelteSources()].flatMap(declaredIds));
  const scripts = [
    ["ui/src/editor/", readEditorSource(), true],
    ["menus.js", read("dist/menus.js"), false],
    ["docking.js", read("dist/docking.js"), false],
  ];
  for (const [label, source, checkAppIds] of scripts) {
    const used = [...literalIds(source), ...(checkAppIds ? appIds(source) : [])];
    assert.ok(used.length > 0, label + " looks up no ids; the pattern no longer matches its code");
    assert.deepEqual(used.filter(id => !ids.has(id)), [], label + " looks up ids that no markup declares");
  }
});

test("every button the command list presses exists in index.html or a Svelte part", () => {
  const ids = new Set([read("dist/index.html"), ...svelteSources()].flatMap(declaredIds));
  const pressed = [...read("ui/src/lib/commands.js").matchAll(/\["(?:onLane", ")?click", "([\w-]+)"\]/g)].map(match => match[1]);
  assert.ok(pressed.length > 10, "commands.js presses no buttons; the pattern no longer matches its rows");
  assert.deepEqual(pressed.filter(id => !ids.has(id)), [], "commands.js presses ids that no markup declares");
});
