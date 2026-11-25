import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  additionPills,
  consolidateDiagnostics,
  parseVolumeDisplay,
  sampleSetPills,
  stepDiagnostic,
  volumeSliderPercent,
} from "../src/lib/inspector.js";
import InspectorSelection from "../src/parts/InspectorSelection.svelte";
import OverviewDiagnostics from "../src/parts/OverviewDiagnostics.svelte";

test("parseVolumeDisplay and volumeSliderPercent handle single and mixed volume readouts", () => {
  assert.deepEqual(parseVolumeDisplay("80%"), { value: 80, mixed: false });
  assert.deepEqual(parseVolumeDisplay("60–80%"), { value: 70, mixed: true });
  assert.deepEqual(parseVolumeDisplay(""), { value: 100, mixed: false });
  assert.equal(volumeSliderPercent(5), 0);
  assert.equal(volumeSliderPercent(100), 100);
});

test("sampleSetPills disables standard sets when only custom file lanes are selected", () => {
  const std = sampleSetPills({ sampleSet: "soft", hasCustomFile: false });
  assert.equal(std.find(p => p.id === "soft").active, true);
  assert.equal(std.every(p => !p.disabled), true);

  const customOnly = sampleSetPills({ sampleSet: null, hasCustomFile: true });
  assert.equal(customOnly.every(p => p.disabled), true);
  assert.match(customOnly[0].title, /Custom sample files/);
});

test("additionPills reports on, off, and mixed states for W/F/C", () => {
  const pills = additionPills({ additions: { whistle: "on", finish: "mixed", clap: "off" } });
  assert.equal(pills[0].active, true);
  assert.equal(pills[1].mixed, true);
  assert.equal(pills[2].active, false);
});

test("consolidateDiagnostics merges problems, orphans, and empty objects in time order", () => {
  const diag = consolidateDiagnostics(
    { problems: [{ time: 3000, message: "Too many sounds" }], orphaned: [1000] },
    { issues: [{ kind: "empty", time: 2000 }] },
  );
  assert.equal(diag.total, 3);
  assert.deepEqual(diag.points.map(p => [p.time, p.kind, p.fixable]), [
    [1000, "orphaned", true],
    [2000, "empty", true],
    [3000, "problem", false],
  ]);
  assert.equal(stepDiagnostic(diag.points, 1500, 1).time, 2000);
  assert.equal(stepDiagnostic(diag.points, 1500, -1).time, 1000);
  assert.equal(stepDiagnostic(diag.points, 4000, 1).time, 1000);
});

test("InspectorSelection renders live volume slider, audition, S/N/D, and W/F/C pills", () => {
  bridge.inspector = {
    busy: false,
    doc: true,
    selection: {
      summary: "1 hit selected · Clap (Normal)",
      start: "00:01.000",
      volume: "80%",
      object: "Circle",
      sampleSet: "normal",
      additions: { whistle: "off", finish: "off", clap: "on" },
      sliderBody: null,
    },
    scope: "all",
    hint: "",
    range: false,
    insertVolume: 100,
    protect: false,
    audition: true,
  };
  const html = render(InspectorSelection).body;
  assert.match(html, /Selection volume slider/);
  assert.match(html, /Audition selection/);
  assert.match(html, />Soft</);
  assert.match(html, />Normal</);
  assert.match(html, />Drum</);
  assert.match(html, />W</);
  assert.match(html, />F</);
  assert.match(html, />C</);
});

test("OverviewDiagnostics renders consolidated issue count and Fix button", () => {
  bridge.scene = { moments: [[1000, 80, true, []]], coverage: { matched: 0, empty: 1, orphaned: 0, issues: [{ kind: "empty", time: 1000 }] } };
  bridge.view = { busy: false, position: 0 };
  bridge.inspector = { problemStrip: { problems: [], orphaned: [] } };
  const html = render(OverviewDiagnostics).body;
  assert.match(html, /Objects overview/);
  assert.match(html, /1 issue/);
  assert.match(html, />Add normal</);
});
