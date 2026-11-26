import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { items, keyOnly, menuBar, menus } from "../src/lib/commands.js";
import { TOOLS } from "../src/lib/tools.js";
import EditorMenu from "../src/parts/EditorMenu.svelte";

const row = fields => ({ index: 0, label: "Undo", icon: null, role: "menuitem", className: "menu-item menu-item-plain",
  checked: undefined, disabled: false, chip: false, keys: "", switch: false, ...fields });

function shown(rows) {
  bridge.handOverMenu(rows && { open: 1, rows }, () => {});
  return render(EditorMenu).body.replace(/<!--[^]*?-->/g, "");
}

test("nothing is drawn while no menu is open", () => {
  assert.equal(shown(null).trim(), "");
});

test("a row draws its icon, label, hidden key until the keys arrive, and a switch for a toggle", () => {
  const html = shown([
    { type: "item", item: row({ index: 0, label: "Undo · Draw hit", icon: "Undo2", className: "menu-item", chip: true, keys: "Ctrl+Z" }) },
    { type: "separator" },
    { type: "item", item: row({ index: 1, label: "Cut", icon: "Eraser", className: "menu-item", chip: true, disabled: true }) },
    { type: "item", item: row({ index: 2, label: "Protect hitnormals from deletion", role: "menuitemcheckbox", checked: false, switch: true }) },
  ]);
  const buttons = [...html.matchAll(/<button([^>]*)>([^]*?)<\/button>/g)];
  assert.equal(buttons.length, 3);
  const [undo, cut, protect] = buttons;
  assert.match(undo[1], /class="menu-item"/);
  assert.match(undo[1], /role="menuitem"/);
  assert.match(undo[1], /tabindex="-1"/);
  assert.doesNotMatch(undo[1], /aria-checked|disabled/);
  assert.equal(undo[2], '<img class="button-icon" src="studio-assets/icons/Undo2.svg" alt=""/>Undo · Draw hit<kbd>Ctrl+Z</kbd>');
  assert.match(cut[1], /disabled/);
  assert.match(cut[2], /<kbd hidden="">/, "a key hint stays hidden until editor_shortcuts answers");
  assert.match(protect[1], /class="menu-item menu-item-plain"/);
  assert.match(protect[1], /role="menuitemcheckbox"/);
  assert.match(protect[1], /aria-checked="false"/);
  assert.match(protect[2], /^Protect hitnormals from deletion<span class="menu-switch" aria-hidden="true"><\/span>$/);
  assert.match(html, /<div class="menu-separator" role="separator"><\/div>/);
  assert.doesNotMatch(html, /\sstyle=|\sid=/);
});

test("segmented choices sit in a labelled group, each a radio row", () => {
  const html = shown([{ type: "choices", label: "Start playing from", choices: [
    row({ index: 0, label: "Cursor", role: "menuitemradio", className: "menu-choice", checked: true }),
    row({ index: 1, label: "Left locator", role: "menuitemradio", className: "menu-choice", checked: false }),
  ] }]);
  assert.match(html, /^\s*<div class="menu-choice-group" role="group" aria-label="Start playing from"><div class="menu-choice-label">Start playing from<\/div>\s*<div class="menu-choices">/);
  const choices = [...html.matchAll(/<button([^>]*)>([^]*?)<\/button>/g)];
  assert.deepEqual(choices.map(match => match[2]), ["Cursor", "Left locator"]);
  assert.match(choices[0][1], /class="menu-choice"[^>]*role="menuitemradio"[^>]*aria-checked="true"/);
  assert.match(choices[1][1], /aria-checked="false"/);
});

test("the command list places every row once per menu and names each catalogue command once", () => {
  const ids = items.map(item => item.id);
  assert.equal(new Set(ids).size, ids.length, "row ids are unique");
  for (const [name, placed] of Object.entries(menus)) {
    for (const id of placed.filter(id => id !== "-")) assert.ok(ids.includes(id), name + " places " + id);
    assert.equal(new Set(placed.filter(id => id !== "-")).size, placed.filter(id => id !== "-").length, name);
  }
  const placed = new Set(Object.values(menus).flat());
  assert.deepEqual(ids.filter(id => !placed.has(id)), [], "every row sits in a menu");
  assert.deepEqual(menuBar, ["file", "edit", "playback", "map", "view", "help"]);
  for (const name of menuBar) assert.ok(menus[name], name);
  const named = items.map(item => item.command).filter(Boolean);
  for (const id of keyOnly) assert.ok(!named.includes(id), id + " is on a menu row and in keyOnly");
  assert.deepEqual(menus.tool.map(id => items.find(item => item.id === id).label), TOOLS,
    "the tool picker lists the tools in the toolbar's order");
});

test("a pressed row calls menus.js's menuAction with its index", async () => {
  const part = await readFile(new URL("../src/parts/EditorMenu.svelte", import.meta.url), "utf8");
  const menus = await readFile(new URL("../src/editor/menus.js", import.meta.url), "utf8");
  assert.match(part, /const press = index => bridge\.callAction\("menuAction", index\);/);
  assert.match(part, /onclick=\{\(\) => press\(row\.index\)\}/);
  assert.match(menus, /^\s*function menuAction\(index\) \{$/m, "menus.js defines menuAction");
  assert.match(menus, /registerAction\("menuAction", menuAction\)/, "menus.js registers menuAction on bridge");
});
