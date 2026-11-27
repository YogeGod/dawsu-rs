import assert from "node:assert/strict";
import { test } from "node:test";
import { PREVIEW_SKIN_KEY, readPreviewSkin, timelineSkinSource } from "../src/lib/previewSkin.js";

test("the picked skin is read from one saved key, Argon when nothing or no storage", () => {
  assert.equal(PREVIEW_SKIN_KEY, "dawsu.previewSkin");
  assert.equal(readPreviewSkin({ getItem: key => (key === PREVIEW_SKIN_KEY ? "dir:C:/Skins/X" : null) }), "dir:C:/Skins/X");
  assert.equal(readPreviewSkin({ getItem: () => null }), "argon");
  assert.equal(readPreviewSkin({ getItem: () => "" }), "argon");
  assert.equal(readPreviewSkin({ getItem: () => { throw new Error("blocked"); } }), "argon");
  assert.equal(readPreviewSkin(null), "argon");
});

test("the objects row draws lazer Classic unless a skin folder is picked", () => {
  for (const id of ["argon", "", null, undefined, "bogus", "classic"]) assert.equal(timelineSkinSource(id), "classic", String(id));
  assert.equal(timelineSkinSource("dir:C:/Skins/X"), "dir:C:/Skins/X");
});
