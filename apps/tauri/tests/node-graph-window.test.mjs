import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const tauri = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, tauri), "utf8");

test("node-graph.html loads only fonts and the Svelte bundle with no inline panel in index.html", () => {
  const html = read("dist/node-graph.html");
  assert.doesNotMatch(html, /<script(?![^>]*src=)|style=|node-graph\.js/i);
  assert.equal([...html.matchAll(/href="([^"]+)"/g)].map(m => m[1]).join(), "studio-assets/dawsu.png,fonts.css,ui/ui.css");
  assert.equal([...html.matchAll(/src="([^"]+)"/g)].map(m => m[1]).join(), "ui/ui.js");
  assert.equal([...html.matchAll(/data-ui-mount="([^"]+)"/g)].map(m => m[1]).join(), "node-graph-window");

  const indexHtml = read("dist/index.html");
  assert.doesNotMatch(indexHtml, /node-graph-panel/);
});
