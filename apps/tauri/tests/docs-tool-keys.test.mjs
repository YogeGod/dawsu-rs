// The wiki first-project guide names the Draw key for each preset and docs/FIRST-PROJECT.md redirects there.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { DEFAULT_DAW_TOOL_KEYS, DEFAULT_OSU_TOOL_KEYS, toolKeyFor } from "../ui/src/lib/quasimode.js";

const dawDraw = toolKeyFor(DEFAULT_DAW_TOOL_KEYS, "Draw");
const osuDraw = toolKeyFor(DEFAULT_OSU_TOOL_KEYS, "Draw");

test("docs/wiki/First-Project.md names the default Draw key for both presets", async () => {
  const text = await readFile(new URL("../../../docs/wiki/First-Project.md", import.meta.url), "utf8");
  const line = text.split(/\r?\n/).find(row => row.includes("**Draw hits:**"));
  assert.ok(line, "the Draw hits line");
  assert.equal(line.match(/`(\w+)` in DAW-like keys/)?.[1], dawDraw, "DAW-like key");
  assert.equal(line.match(/`(\w+)` in osu! editor controls/)?.[1], osuDraw, "osu! key");
});

test("docs/FIRST-PROJECT.md redirects to the wiki First Project guide", async () => {
  const text = await readFile(new URL("../../../docs/FIRST-PROJECT.md", import.meta.url), "utf8");
  assert.match(text, /\[First Project\]\(wiki\/First-Project\.md\)/);
  assert.equal(text.includes("**Draw hits:**"), false, "stub must not duplicate walkthrough content");
});
