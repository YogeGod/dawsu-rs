import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import MapsetIntakeCard from "../src/parts/MapsetIntakeCard.svelte";

test("MapsetIntakeCard detects existing [Hitsounds] diff and offers 1-click open", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      mapset: {
        artist: "Camellia",
        title: "Exit This Earth's Atomosphere",
        folder: "12345 Camellia",
        difficulties: [
          { path: "/m/insane.osu", version: "Insane" },
          { path: "/m/hs.osu", version: "Hitsounds" },
        ],
      },
    },
  }).body;
  assert.match(html, /Camellia - Exit This Earth's Atomosphere/);
  assert.match(html, /Open \[Hitsounds\] →/);
  assert.match(html, /Auto-backup stays on/);
});

test("MapsetIntakeCard offers Create [Hitsounds] when none exists", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      mapset: {
        artist: "xi",
        title: "FREEDOM DiVE",
        folder: "999 xi",
        difficulties: [{ path: "/m/4d.osu", version: "FOUR DIMENSIONS" }],
      },
    },
  }).body;
  assert.match(html, /Create \[Hitsounds\] &amp; Start Editing →/);
});

test("MapsetIntakeCard guards against empty or corrupt mapset folders", () => {
  const emptyHtml = render(MapsetIntakeCard, {
    props: { mapset: { folder: "empty", difficulties: [] } },
  }).body;
  assert.match(emptyHtml, /No readable \.osu difficulties found/);

  const corruptHtml = render(MapsetIntakeCard, {
    props: { mapset: { folder: "bad", corrupt: true, error: "Could not parse mapset folder." } },
  }).body;
  assert.match(corruptHtml, /Could not parse mapset folder\./);
});
