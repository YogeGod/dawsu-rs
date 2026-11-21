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

test("every id the old scripts look up by name exists in index.html or a Svelte part", () => {
  const ids = new Set([read("dist/index.html"), ...svelteSources()].flatMap(declaredIds));
  for (const script of ["app.js", "menus.js", "docking.js"]) {
    const source = read("dist/" + script);
    const used = [...literalIds(source), ...(script === "app.js" ? appIds(source) : [])];
    assert.ok(used.length > 0, script + " looks up no ids; the pattern no longer matches its code");
    assert.deepEqual(used.filter(id => !ids.has(id)), [], script + " looks up ids that no markup declares");
  }
});

test("every button the command list presses exists in index.html or a Svelte part", () => {
  const ids = new Set([read("dist/index.html"), ...svelteSources()].flatMap(declaredIds));
  const pressed = [...read("ui/src/lib/commands.js").matchAll(/\["(?:onLane", ")?click", "([\w-]+)"\]/g)].map(match => match[1]);
  assert.ok(pressed.length > 10, "commands.js presses no buttons; the pattern no longer matches its rows");
  assert.deepEqual(pressed.filter(id => !ids.has(id)), [], "commands.js presses ids that no markup declares");
});
