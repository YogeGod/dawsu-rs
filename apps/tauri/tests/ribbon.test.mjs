import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css] = await Promise.all(["index.html", "style.css"].map(read));

test("the ribbon compacts its grouped controls while keeping Save labeled", () => {
  for (const group of ["Map", "Lane mode", "Tool", "Grid"]) {
    assert.match(html, new RegExp('role="group" aria-label="' + group + '"'));
  }
  assert.match(css, /@media\s*\(max-width:\s*1320px\)\s*\{[^}]*\.ribbon-caption\s*\{\s*display:\s*none/);
  const save = html.match(/<button id="save"[^>]*>(.*?)<\/button>/s)?.[1];
  assert.match(save, />Save</);
  assert.doesNotMatch(save, /class="ribbon-(?:caption|tool-label)"/);
});

test("the sibling difficulty picker is available from the ribbon with the inspector hidden", () => {
  const ribbon = html.match(/<div id="editor-ribbon"[^>]*>(.*?)<main id="editor-dock"/s)?.[1];
  assert.match(ribbon, /<select id="difficulty"[^>]*data-doc/);
  assert.equal([...html.matchAll(/id="difficulty"/g)].length, 1);
});

test("the window exposes Map, Edit, View and Playback pop-up menu triggers", () => {
  const menus = html.match(/<nav id="editor-menus"[^>]*>(.*?)<\/nav>/s)?.[1];
  assert.ok(menus);
  for (const name of ["map", "edit", "view", "playback"]) {
    assert.match(menus, new RegExp('id="menu-' + name + '"[^>]*aria-haspopup="menu"'));
  }
  assert.match(html, /id="editor-menu"[^>]*role="menu"[^>]*hidden/);
});
