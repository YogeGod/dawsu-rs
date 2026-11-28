import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const css = readFileSync(new URL("../src/parts/export/ExportReviewPane.svelte", import.meta.url), "utf8").split("<style>")[1];

test("Export's sample file list wraps a long file name instead of cutting it off", () => {
  // "dawsu-mix-77ca292da2e6545c9adb9540ffd3137bca129272..." was cut, and that name is what lands in the mapset.
  const rule = css.match(/\.write-details li\s*\{([^}]*)\}/)?.[1] ?? "";
  assert.doesNotMatch(rule, /nowrap|ellipsis/);
  assert.match(rule, /overflow-wrap:\s*anywhere;/);
});
