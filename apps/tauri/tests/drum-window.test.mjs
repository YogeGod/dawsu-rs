import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const tauri = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, tauri), "utf8");

test("drum.html loads only the fonts and the Svelte bundle, with no inline code", () => {
  const html = read("dist/drum.html");
  const head = html.match(/<head>(.*?)<\/head>/s)[1];
  const order = [...head.matchAll(/(?:href|src)="([^"]+\.(?:css|js))"/g)].map(match => match[1]);
  assert.deepEqual(order, ["fonts.css", "ui/ui.css", "ui/ui.js"]);
  assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)[^>]*>/, "no inline script");
  assert.doesNotMatch(html, /<style|\sstyle=|\son\w+=/, "no inline style or handler");
  assert.equal([...html.matchAll(/data-ui-mount="([^"]+)"/g)].map(match => match[1]).join(), "drum-window");
  assert.match(html, /<body class="drum-page">/);
  assert.match(html, /<title>Drum hit detection<\/title>/);
});
