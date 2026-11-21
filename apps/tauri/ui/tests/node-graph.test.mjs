import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NODE_WIDTH,
  fitViewBox,
  inputSocketPos,
  linkPath,
  nodeHeight,
  outputSocketPos,
  parseParamInput,
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
  assert.match(path, /^M 280\.0 .* C .* 400\.0 /);
  const box = fitViewBox([n1, n2]);
  assert.equal(box.x, 40);
  assert.equal(box.y, 140);
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
