import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import MapsetIntakeCard from "../src/parts/MapsetIntakeCard.svelte";

test("MapsetIntakeCard pins existing [Hitsounds] diff at top and folds creation options", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      mapset: {
        artist: "Camellia",
        title: "Exit This Earth's Atomosphere",
        creator: "Kylix",
        folder: "12345 Camellia",
        difficulties: [
          { path: "/m/insane.osu", version: "Insane", mode: 0, isHs: false },
          { path: "/m/hs.osu", version: "Hitsounds", mode: 0, isHs: true },
        ],
      },
    },
  }).body;
  assert.match(html, /Camellia - Exit This Earth's Atomosphere/);
  assert.match(html, /Mapped by Kylix/);
  assert.match(html, /Continue \[Hitsounds\] →/);
  assert.match(html, /Create another hitsound diff…/);
  assert.match(html, /Auto-backup stays on/);
});

test("MapsetIntakeCard offers Path 1, Path 2, and Path 3 when no [Hitsounds] diff exists", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      mapset: {
        artist: "xi",
        title: "FREEDOM DiVE",
        folder: "999 xi",
        difficulties: [
          { path: "/m/4d.osu", version: "FOUR DIMENSIONS", mode: 0, isHs: false },
          { path: "/m/another.osu", version: "Another", mode: 0, isHs: false },
        ],
      },
    },
  }).body;
  assert.match(html, /Custom samples only/);
  assert.match(html, /12 osu! default lanes/);
  assert.match(html, /Or edit a difficulty directly/);
  assert.match(html, /FOUR DIMENSIONS/);
  assert.match(html, /Timing reference/);
});

test("MapsetIntakeCard disables non-standard difficulties and guards timing reference", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      mapset: {
        artist: "Test",
        title: "Mixed Modes",
        folder: "mixed",
        difficulties: [
          { path: "/m/taiko.osu", version: "Oni", mode: 1, isHs: false },
          { path: "/m/mania.osu", version: "4K MX", mode: 3, isHs: false },
        ],
      },
    },
  }).body;
  assert.match(html, /taiko/);
  assert.match(html, /mania/);
  assert.match(html, /No osu!standard difficulty found to copy timing from/);
});

test("MapsetIntakeCard renders inline mode with mapset background artwork when provided", () => {
  const html = render(MapsetIntakeCard, {
    props: {
      inline: true,
      mapset: {
        artist: "Camellia",
        title: "Ghost",
        folder: "ghost",
        backgroundUrl: "data:image/png;base64,AA==",
        difficulties: [{ path: "/m/extra.osu", version: "Extra", mode: 0, isHs: false }],
      },
    },
  }).body;
  assert.match(html, /intake-card-inline/);
  assert.match(html, /intake-hero-bg/);
  assert.match(html, /data:image\/png;base64,AA==/);
  assert.doesNotMatch(html, /intake-backdrop/);
  assert.doesNotMatch(html, /\sstyle=/);
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
