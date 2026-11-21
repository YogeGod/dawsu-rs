import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NODE_META,
  NODE_WIDTH,
  autoLayoutNodes,
  extractSimpleFlowModel,
  fitViewBox,
  inputSocketPos,
  linkPath,
  nodeHeight,
  outputSocketPos,
  parseParamInput,
  sensitivityToThreshold,
  sparklinePoints,
  thresholdToSensitivity,
  visibleViewerOverlay,
} from "../src/lib/node-graph.js";

test("node geometry places input sockets on left, outputs on right, and computes paths", () => {
  const n1 = { id: "a", x: 100, y: 200, inputs: [], outputs: [{ id: "curve" }] };
  const n2 = { id: "b", x: 400, y: 260, inputs: [{ id: "curve" }], outputs: [{ id: "events" }] };
  assert.ok(nodeHeight(n1) > 40);
  const out = outputSocketPos(n1, "curve");
  const inp = inputSocketPos(n2, "curve");
  assert.equal(out.x, 100 + NODE_WIDTH);
  assert.equal(inp.x, 400);
  const path = linkPath(out, inp);
  assert.match(path, /^M 300\.0 .* C .* 400\.0 /);
  const box = fitViewBox([n1, n2]);
  assert.equal(box.x, 40);
  assert.equal(box.y, 140);
});

test("NODE_META provides plain-English titles, sensitivity inversion, and sparklines", () => {
  assert.equal(NODE_META["events.peaks"].title, "Pick Hits");
  assert.equal(thresholdToSensitivity(1.5), 50);
  assert.equal(sensitivityToThreshold(50), 1.5);
  assert.ok(thresholdToSensitivity(1.0) > thresholdToSensitivity(2.0));
  const pts = sparklinePoints([0, 5, 10], 100, 20, 2);
  assert.equal(pts, "0.0,18.0 50.0,10.0 100.0,2.0");
});

test("extractSimpleFlowModel pairs detectors, HPSS, Beat Snap, and slot hit counts", () => {
  const view = {
    nodes: [
      { id: "song", typeId: "input.song", x: 0, y: 0, params: {} },
      { id: "hpss", typeId: "spectral.hpss", x: 100, y: 0, muted: false, params: {} },
      { id: "flux_k", typeId: "onset.flux", x: 200, y: 0, params: { loHz: 30, hiHz: 120 }, summary: { kind: "curve", sparkline: [0, 1] } },
      { id: "peaks_k", typeId: "events.peaks", x: 300, y: 0, params: { threshold: 1.5, tag: "kick", minGapMs: 60 }, summary: { kind: "events", count: 12 } },
      { id: "snap", typeId: "events.snap", x: 400, y: 0, params: { divisors: [4] } },
      { id: "out_k", typeId: "output.lane", x: 500, y: 0, params: { slot: "kick" }, summary: { kind: "events", count: 9 } },
    ],
    links: [
      { fromNode: "song", fromSocket: "audio", toNode: "hpss", toSocket: "spectrogram" },
      { fromNode: "hpss", fromSocket: "spectrogram", toNode: "flux_k", toSocket: "spectrogram" },
      { fromNode: "flux_k", fromSocket: "curve", toNode: "peaks_k", toSocket: "curve" },
      { fromNode: "peaks_k", fromSocket: "events", toNode: "snap", toSocket: "events" },
      { fromNode: "snap", fromSocket: "events", toNode: "out_k", toSocket: "events" },
    ],
    slots: [{ slot: "kick", boundLaneName: "Soft Finish" }],
    batch: { proposals: [{ id: "1", slot: "kick", checked: true }, { id: "2", slot: "kick", checked: false }] },
  };
  const model = extractSimpleFlowModel(view);
  assert.equal(model.detectors.length, 1);
  assert.equal(model.detectors[0].flux.id, "flux_k");
  assert.equal(model.detectors[0].foundCount, 12);
  assert.equal(model.slotOutputs[0].hitCount, 1);
  const laid = autoLayoutNodes(view.nodes, view.links);
  assert.ok(laid.find(p => p.id === "peaks_k").x > laid.find(p => p.id === "flux_k").x);
});

test("parseParamInput validates integers, numbers, and booleans cleanly", () => {
  assert.deepEqual(parseParamInput({ socketType: "bool" }, "", true), { ok: true, value: true });
  assert.deepEqual(parseParamInput({ socketType: "integer" }, "12", false), { ok: true, value: 12 });
  assert.equal(parseParamInput({ socketType: "integer" }, "1.5", false).ok, false);
  assert.deepEqual(parseParamInput({ socketType: "number", default: null }, "null", false), { ok: true, value: null });
  assert.equal(parseParamInput({ socketType: "number", default: 1 }, "abc", false).ok, false);
});

test("visibleViewerOverlay projects curve points and visible events onto ruler band", () => {
  const xOf = (ms) => ms * 0.1;
  const curve = visibleViewerOverlay(
    { kind: "curve", startMs: 0, hopMs: 10, sampleStep: 1, min: 0, max: 10, values: [0, 5, 10, 2] },
    0,
    100,
    xOf,
    40,
    80,
  );
  assert.equal(curve.kind, "curve");
  assert.equal(curve.points.length, 4);
  assert.deepEqual(curve.points[0], [0, 120]);
  assert.deepEqual(curve.points[2], [2, 40]);

  const events = visibleViewerOverlay(
    { kind: "events", timesMs: [-100, 50, 200], strengths: [0.5, 0.9, 1.0] },
    0,
    100,
    xOf,
    40,
    80,
  );
  assert.equal(events.kind, "events");
  assert.equal(events.markers.length, 1);
  assert.equal(events.markers[0].x, 5);
});
