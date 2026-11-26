import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  fxSlotSummary, latencyLabel, normalizeMixerFxState, stripFxKey, wetDryPercent,
} from "../src/lib/mixerFx.js";
import PluginWrapperBar from "../src/parts/PluginWrapperBar.svelte";

test("stripFxKey resolves fixed buses and canonical lane keys cleanly", () => {
  assert.equal(stripFxKey("song"), "song");
  assert.equal(stripFxKey("hitsounds"), "hitsounds");
  assert.equal(stripFxKey("master"), "master");
  assert.equal(stripFxKey("lane", { fxKey: "lane:Skin|soft-hitclap", colourKey: "other" }), "lane:Skin|soft-hitclap");
  assert.equal(stripFxKey("lane", { colourKey: "File|kick.wav" }), "lane:File|kick.wav");
  assert.equal(stripFxKey("unknown"), null);
});

test("wetDryPercent, latencyLabel, and fxSlotSummary format and summarize state", () => {
  assert.equal(wetDryPercent(0.354), 35);
  assert.equal(wetDryPercent(1.5), 100);
  assert.equal(wetDryPercent(-0.2), 0);
  assert.equal(latencyLabel(0, 48000), "0 smp");
  assert.equal(latencyLabel(512, 48000), "512 smp (10.7 ms)");
  assert.deepEqual(
    fxSlotSummary([
      { bypassed: false, latencySamples: 256 },
      { bypassed: true, latencySamples: 512 },
    ]),
    { total: 2, active: 1, bypassed: 1, totalLatency: 768 },
  );
  const norm = normalizeMixerFxState(null);
  assert.equal(norm.totalPdcSamples, 0);
  assert.equal(norm.sampleRate, 44100);
});

test("PluginWrapperBar renders A/B, Copy, Soft Bypass, Wet/Dry, Pin, Undo/Redo with no fake GR meter or forbidden attributes", () => {
  const fxState = {
    revision: 3,
    totalPdcSamples: 512,
    sampleRate: 48000,
    canUndo: true,
    canRedo: false,
    sidecarAvailable: true,
    availableEffects: [
      { name: "Pro-Q 4", vendor: "FabFilter", uid: "UID1", path: "C:/VST3/ProQ4.vst3", category: "Fx" },
    ],
    strips: {
      master: [
        {
          slotId: "fx-1",
          pluginPath: "C:/VST3/ProQ4.vst3",
          pluginUid: "UID1",
          name: "Pro-Q 4",
          bypassed: false,
          wetDry: 0.75,
          activeBank: "b",
          hasStateA: true,
          hasStateB: true,
          pinned: true,
          editorOpen: true,
          alive: true,
          available: true,
          latencySamples: 512,
          lateBlocks: 0,
        },
      ],
    },
  };

  const { body } = render(PluginWrapperBar, {
    props: {
      stripKey: "master",
      stripLabel: "Master",
      fxState,
      disabled: false,
    },
  });

  assert.match(body, /Master FX/);
  assert.match(body, /PDC 512 smp \(10\.7 ms\)/);
  assert.match(body, /Pro-Q 4/);
  assert.match(body, /512 smp/);
  assert.match(body, /aria-label="Toggle A\/B bank for Pro-Q 4"[^>]*aria-pressed="true"[^>]*>\s*B/);
  assert.match(body, /B→A/);
  assert.match(body, /Wet dry mix for Pro-Q 4/);
  assert.match(body, /75%/);
  assert.match(body, /aria-label="Pin window for Pro-Q 4"[^>]*aria-pressed="true"/);
  assert.match(body, /Undo FX change/);
  assert.match(body, /Redo FX change/);
  assert.doesNotMatch(body, /\sid=|\sstyle=|gain.?reduction|\bGR\b/i);

  const src = readFileSync(new URL("../src/parts/PluginWrapperBar.svelte", import.meta.url), "utf8");
  assert.match(src, /invoke\("mixer_fx_action"/);
});
