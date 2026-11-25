import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { clampMicroHud, microHudMode } from "../src/lib/microHud.js";
import MicroHud from "../src/parts/MicroHud.svelte";

test("clampMicroHud keeps the floating toolbar inside the viewport", () => {
  const clamped = clampMicroHud(10, 10, 1280, 720);
  assert.equal(clamped.left, 10);
  assert.ok(clamped.top >= 10);
  assert.equal(microHudMode("drums"), "drums");
  assert.equal(microHudMode("keysounds"), "keysounds");
  assert.equal(microHudMode("other"), "hitsounds");
});

test("MicroHud adapts across hitsounds, drums, and keysounds modes (no Storyboard option)", () => {
  bridge.inspector = {
    selection: {
      summary: "1 hit selected",
      volume: "80%",
      sampleSet: "normal",
      additions: { whistle: "off", finish: "off", clap: "on" },
    },
  };
  bridge.microHud = { x: 400, y: 200, mode: "hitsounds" };
  const hsHtml = render(MicroHud).body;
  assert.match(hsHtml, /80%/);
  assert.match(hsHtml, /Audition/);

  bridge.microHud = { x: 400, y: 200, mode: "keysounds", noteLabel: "C5", chordPolicy: "flatten" };
  const ksHtml = render(MicroHud).body;
  assert.match(ksHtml, /Flatten/);
  assert.match(ksHtml, /Top only/);
  assert.doesNotMatch(ksHtml, /Storyboard/);
  bridge.microHud = null;
});
