import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  additionOnlyPolicies,
  choicesFrom,
  defaultChoices,
  optionsKey,
  previewRequest,
  singleFormats,
  tickPolicies,
  writeModes,
} from "../src/lib/export.js";
import { uniqueLaneOptions } from "../src/lib/sharedBase.js";
import ExportWindow from "../src/parts/ExportWindow.svelte";
import SharedBaseModal from "../src/parts/SharedBaseModal.svelte";
import StatusBar from "../src/parts/StatusBar.svelte";
import StepPadsRow from "../src/parts/StepPadsRow.svelte";

const insane = "C:\\Songs\\set\\Artist - Song (Mapper) [Insane].osu";
const hard = "C:\\Songs\\set\\Artist - Song (Mapper) [Hard].osu";
const view = {
  source: {
    path: "C:\\Songs\\set\\Artist - Song (Mapper) [HS 1].osu",
    difficulty: "HS 1",
    version: 4,
    hits: 24,
    lanes: [{ name: "Kick", colourKey: "kick", hitCount: 24 }],
    laneColours: { kick: "#f1ad67" },
  },
  targets: [insane, hard],
  choices: null,
  busy: false,
  previewing: false,
  writing: false,
  written: null,
  writeError: null,
  error: null,
  problems: [],
  preview: null,
};

test("export.js exposes and round-trips all hitsounds-rs parity+ choices", () => {
  assert.deepEqual(writeModes, ["direct", "hitsound_only"]);
  assert.deepEqual(singleFormats, ["source", "match_mix_format"]);
  assert.deepEqual(additionOnlyPolicies, [
    "difficulty",
    "additions_set",
    "skin_normal",
    "skin_soft",
    "skin_drum",
    "silent",
  ]);
  assert.deepEqual(tickPolicies, ["ignore", "render"]);

  const custom = choicesFrom({
    choices: {
      leftOut: [],
      matchWithin: 6,
      copyVolumes: true,
      autoFixStagedMixes: true,
      writeMode: "hitsound_only",
      hitsoundOnlyDir: "E:\\Exports\\MyMap",
      singleFormat: "match_mix_format",
      additionOnlyNormal: "silent",
      tickPolicy: "render",
      allowSchemaGrowth: false,
      autoCleanupStale: false,
      takeOverSamples: true,
      applyVolumeEnvelope: false,
    },
  });
  assert.equal(custom.writeMode, "hitsound_only");
  assert.equal(custom.hitsoundOnlyDir, "E:\\Exports\\MyMap");
  assert.equal(custom.singleFormat, "match_mix_format");
  assert.equal(custom.additionOnlyNormal, "silent");
  assert.equal(custom.tickPolicy, "render");
  assert.equal(custom.allowSchemaGrowth, false);
  assert.equal(custom.autoCleanupStale, false);
  assert.equal(custom.takeOverSamples, true);
  assert.equal(custom.applyVolumeEnvelope, false);
  assert.equal(custom.autoFixStagedMixes, true);

  assert.notEqual(optionsKey(view, custom), optionsKey(view, choicesFrom(view)));
  const req = previewRequest(view, custom);
  assert.equal(req.options.writeMode, "hitsound_only");
  assert.equal(req.options.hitsoundOnlyDir, "E:\\Exports\\MyMap");
  assert.equal(req.options.singleFormat, "match_mix_format");
  assert.equal(req.options.additionOnlyNormal, "silent");
  assert.equal(req.options.tickPolicy, "render");
  assert.equal(req.options.allowSchemaGrowth, false);
  assert.equal(req.options.autoCleanupStale, false);
  assert.equal(req.options.takeOverSamples, true);
  assert.equal(req.options.applyVolumeEnvelope, false);
  assert.equal(req.options.autoFixStagedMixes, true);

  // Direct mode always sends hitsoundOnlyDir: null even if a folder was remembered.
  const directReq = previewRequest(view, { ...defaultChoices, writeMode: "direct", hitsoundOnlyDir: "E:\\Exports\\MyMap" });
  assert.equal(directReq.options.hitsoundOnlyDir, null);
});

test("ExportWindow renders Auto-fix switch, Hitsound-only folder summary, and Advanced toggle", () => {
  const { body: directBody } = render(ExportWindow, { props: { initialView: view } });
  assert.match(directBody, /aria-label="Auto-fix ranking criteria sample issues"/);
  assert.match(directBody, /aria-label="Export destination"/);
  assert.match(directBody, /Into the mapset/);
  assert.match(directBody, /Advanced…/);

  const hsOnly = {
    ...view,
    choices: {
      ...defaultChoices,
      writeMode: "hitsound_only",
      hitsoundOnlyDir: "E:\\Exports\\Demo",
    },
  };
  const { body: isolatedBody } = render(ExportWindow, { props: { initialView: hsOnly } });
  assert.match(isolatedBody, /Hitsound-only folder/);
  assert.match(isolatedBody, /E:\\Exports\\Demo/);
  assert.match(isolatedBody, /Your mapset folder stays untouched/);
});

test("StatusBar renders Keep Pitch toggle, Beat Step Pads toggle, and polite a11y live region", () => {
  const scene = {
    path: "C:/Songs/Set/Song [Hard].osu",
    lanes: [{ key: "a" }],
    laneOrder: [0],
    moments: [[1000, 100, true, [[0, false]]]],
    selection: { count: 2 },
    zoom: 1,
    rowHeight: 30,
  };
  bridge.set(
    {
      busy: false,
      playing: false,
      speed: 0.5,
      keepPitch: true,
      stepPadsMode: true,
      status: "Ready",
      warnings: [],
      outputDevice: "Speakers",
      outputProblems: [],
    },
    scene,
  );
  const { body } = render(StatusBar);
  assert.match(body, /id="keep-pitch-toggle"[^>]*aria-pressed="true"/);
  assert.match(body, /id="step-pads-toggle"[^>]*aria-pressed="true"/);
  assert.match(body, /id="a11y-live"[^>]*aria-live="polite"[^>]*aria-atomic="true"/);
  assert.match(body, /Ready · 2 hits selected/);

  bridge.set({ ...bridge.view, keepPitch: false, stepPadsMode: false }, scene);
  const { body: offBody } = render(StatusBar);
  assert.match(offBody, /id="keep-pitch-toggle"[^>]*aria-pressed="false"/);
  assert.match(offBody, /id="step-pads-toggle"[^>]*aria-pressed="false"/);
});

test("StepPadsRow renders 8 beat-snapped pads and highlights active hits on its lane", () => {
  bridge.set(
    { position: 0, busy: false, stepPadsMode: true },
    {
      sections: [[0, 400]], // 150 BPM -> 1/4 step = 100 ms
      divisor: 4,
      moments: [
        [0, 100, true, [[0, false]]],
        [200, 100, true, [[0, false]]],
        [300, 100, true, [[1, false]]],
      ],
    },
  );
  const { body } = render(StepPadsRow, { props: { laneRow: 0, laneName: "Kick" } });
  const padMatches = body.match(/class="step-pad\b/g) ?? [];
  assert.equal(padMatches.length, 8);
  const activeMatches = body.match(/aria-pressed="true"/g) ?? [];
  assert.equal(activeMatches.length, 2, "steps 1 (0 ms) and 3 (200 ms) are active on lane 0");
  assert.match(body, /aria-label="Kick step 1 at 0 ms"/);
  assert.match(body, /aria-label="Kick step 3 at 200 ms"/);

  const src = readFileSync(new URL("../src/parts/StepPadsRow.svelte", import.meta.url), "utf8");
  assert.match(src, /kind:\s*"toggle_step_hit"/);
  assert.doesNotMatch(src, /kind:\s*"pointer"/);
});

test("SharedBaseModal deduplicates laneOptions and keys SVG volume points safely", () => {
  assert.deepEqual(
    uniqueLaneOptions([
      { name: "soft-hitwhistle" },
      { name: "drum-hitnormal" },
      { name: "soft-hitwhistle" },
      { name: "" },
      {},
    ]),
    ["soft-hitwhistle", "drum-hitnormal"],
  );
  bridge.set(
    { busy: false },
    {
      path: "C:/Songs/Set/Song [Hard].osu",
      lanes: [
        { name: "soft-hitwhistle" },
        { name: "soft-hitwhistle" },
        { name: "drum-hitnormal" },
      ],
      moments: [],
    },
  );
  const { body } = render(SharedBaseModal);
  assert.equal(typeof body, "string");
  const sbSrc = readFileSync(new URL("../src/parts/SharedBaseModal.svelte", import.meta.url), "utf8");
  assert.match(sbSrc, /uniqueLaneOptions\(/);
  assert.match(sbSrc, /`\$\{pt\.timeMs\}-\$\{idx\}`/);
});
