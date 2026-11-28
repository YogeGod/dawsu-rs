import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { exportModalOnTop, forwardSpace } from "../src/lib/space.js";

// A key event on a target whose closest() answers like the browser's: the target, or a parent,
// is one of the tags in the selector.
function keydown(init = {}, { inside = null } = {}) {
  const event = {
    key: " ", repeat: false, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, defaultPrevented: false,
    target: { closest: selector => (inside && selector.split(",").map(part => part.trim()).includes(inside) ? {} : null) },
    prevented: 0,
    preventDefault() { this.prevented += 1; },
    ...init,
  };
  return event;
}

function events() {
  const sent = [];
  return { sent, emitTo: (target, name) => { sent.push([target, name]); return Promise.resolve(); } };
}

test("Space in the Export, Drum or Node Graph window plays and stops the editor", () => {
  const bus = events();
  const event = keydown();
  assert.equal(forwardSpace(event, bus), true);
  assert.deepEqual(bus.sent, [["main", "preview-space"]]);
  assert.equal(event.prevented, 1, "the page doesn't scroll or press the focused button");
});

test("Space in a text, number or list field stays in the field", () => {
  for (const inside of ["input", "textarea", "select", "[contenteditable]"]) {
    const bus = events();
    const event = keydown({}, { inside });
    assert.equal(forwardSpace(event, bus), false, inside);
    assert.deepEqual(bus.sent, [], inside);
    assert.equal(event.prevented, 0, inside);
  }
});

test("a held, modified, already-handled or other key is not sent", () => {
  for (const init of [{ repeat: true }, { ctrlKey: true }, { altKey: true }, { shiftKey: true }, { metaKey: true },
    { defaultPrevented: true }, { key: "Enter" }, { key: "k" }]) {
    const bus = events();
    assert.equal(forwardSpace(keydown(init), bus), false, JSON.stringify(init));
    assert.deepEqual(bus.sent, [], JSON.stringify(init));
  }
});

test("each window sends Space through the shared helper on its window keydown", () => {
  for (const name of ["ExportWindow", "DrumDetectWindow", "NodeGraphWindow"]) {
    const source = readFileSync(new URL(`../src/parts/${name}.svelte`, import.meta.url), "utf8");
    if (name !== "ExportWindow") assert.match(source, /import \{ forwardSpace \} from "\.\.\/lib\/space\.js";/, name);
    assert.doesNotMatch(source, /preview-space/, name + " has no copy of its own");
    if (name === "ExportWindow") {
      // Embedded in the editor, the modal sends Space only while it is on top (see below).
      assert.match(source, /import \{ exportModalOnTop, forwardSpace \} from "\.\.\/lib\/space\.js";/);
      assert.match(source, /<svelte:window onkeydown=\{onWindowKeydown\} \/>/);
      assert.match(source, /function onWindowKeydown\(event\) \{\s*if \(!onclose \|\| exportModalOnTop\(onlyModalOpen\)\) forwardSpace\(event\);\s*\}/);
      continue;
    }
    if (name === "NodeGraphWindow") {
      // Space first, before the raw graph's own keys can take it.
      assert.match(source, /<svelte:window onkeydown=\{handleWindowKeyDown\} \/>/);
      assert.match(source, /function handleWindowKeyDown\(event\) \{\s*\/\/[^\n]*\n\s*if \(forwardSpace\(event\)\) return;/);
    } else {
      assert.match(source, /<svelte:window onkeydown=\{forwardSpace\} \/>/, name);
    }
  }
});

test("the Export modal takes Space and Escape only while it is the one modal and no dialog is over it", () => {
  const opened = new Set();
  const only = id => [...opened].every(open => open === id);
  const page = dialogOpen => ({ querySelector: selector => (selector === "dialog[open]" && dialogOpen ? {} : null) });
  opened.add("export");
  assert.equal(exportModalOnTop(only, page(false)), true);
  assert.equal(exportModalOnTop(only, page(true)), false, "a confirm or error dialog is on top");
  opened.add("confirm");
  assert.equal(exportModalOnTop(only, page(false)), false, "another modal is open too");
});

test("ExportModal's Escape waits for anything open on top of it", () => {
  const source = readFileSync(new URL("../src/parts/ExportModal.svelte", import.meta.url), "utf8");
  assert.match(source, /if \(open && event\.key === "Escape" && exportModalOnTop\(bridge\.onlyModalOpen\)\) \{/);
});