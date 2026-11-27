import assert from "node:assert/strict";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { findNearestMoment, hitHalfWidth } from "../ui/src/editor/helpers.js";
import { bootEditor } from "./editor-runtime.mjs";

function linearNearestMoment(moments, ms, tolerance, match, strict = true) {
  let best = tolerance;
  let found = null;
  for (const moment of moments) {
    const distance = Math.abs(moment[0] - ms);
    if (!(strict ? distance < best : distance <= best)) continue;
    const value = match(moment);
    if (value === undefined) continue;
    best = distance;
    found = value;
  }
  return found;
}

function makeRng(seed) {
  let s = seed >>> 0;
  return () => {
    s = (Math.imul(1664525, s) + 1013904223) >>> 0;
    return s / 0x100000000;
  };
}

function createEditorHarness() {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) {
      const listeners = new Map();
      elements.set(id, {
        events: listeners,
        children: [],
        dataset: {},
        scrollTop: 0,
        clientWidth: 1000,
        style: { setProperty(name, value) { this[name] = value; } },
        addEventListener: (name, fn) => listeners.set(name, fn),
        append() {},
        prepend() {},
        setAttribute() {},
        getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 400 }),
      });
    }
    return elements.get(id);
  }
  const context = createContext({
    document: {
      documentElement: { dataset: {} },
      getElementById: element,
      createElement: () => element(`created-${elements.size}`),
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: {
      __TAURI__: {
        core: { invoke: async () => ({ busy: false, hasDocument: true, position: 0 }) },
        event: { listen() {} },
      },
    },
    performance: { now: () => 0 },
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  const ctx = bootEditor(context, { autoStart: false });
  return { context, ctx, get: element };
}

test("findNearestMoment matches linear scan on exact ties, boundaries, and zero/float timestamps", () => {
  const moments = [
    [0, 100, true, [[0, 100, false, false]]],
    [100, 100, true, [[1, 100, false, false]]],
    [110, 100, true, [[0, 90, false, false]]],
    [120, 100, true, [[0, 80, false, false]]],
    [120, 100, true, [[2, 70, false, false]]],
    [130, 100, true, [[0, 60, false, false]]],
    [1000000.3, 100, true, [[0, 50, false, false]]],
    [1000020.9, 100, true, [[0, 50, false, false]]],
  ];
  const matchRow = (row) => ([time, , , hits]) =>
    ((row === null ? hits.length > 0 : hits.some(([lane]) => lane === row)) ? time : undefined);

  // Exact tie between 110 and 120 at ms = 115: strict keeps first (110), non-strict keeps later (120).
  assert.equal(findNearestMoment(moments, 115, 10, matchRow(0), { strict: true }), 110);
  assert.equal(findNearestMoment(moments, 115, 10, matchRow(0), { strict: false }), 120);

  // Boundary tolerance: distance === tolerance is excluded under strict <, included under <=.
  assert.equal(findNearestMoment(moments, 100, 10, matchRow(0), { strict: true }), null);
  assert.equal(findNearestMoment(moments, 100, 10, matchRow(0), { strict: false }), 110);
  assert.equal(findNearestMoment(moments, 140, 10, matchRow(0), { strict: true }), null);
  assert.equal(findNearestMoment(moments, 140, 10, matchRow(0), { strict: false }), 130);

  // Time 0 is preserved (not treated as falsy null).
  assert.equal(findNearestMoment(moments, 5, 10, matchRow(0), { strict: true }), 0);

  // Non-matching moment closer to ms (120 on row 2 vs 110/130 on row 0) does not shadow row 0.
  assert.equal(findNearestMoment(moments, 119, 15, matchRow(1), { strict: true }), null);
  assert.equal(findNearestMoment(moments, 105, 10, matchRow(1), { strict: true }), 100);

  // Duplicate timestamp where only the second entry matches row 2.
  assert.equal(findNearestMoment(moments, 125, 10, matchRow(2), { strict: true }), 120);

  // Large timestamp with fractional tolerance (checks float cutoff parity).
  assert.equal(
    findNearestMoment(moments, 1000010.6, 10.300000000000001, matchRow(0), { strict: true }),
    linearNearestMoment(moments, 1000010.6, 10.300000000000001, matchRow(0), true),
  );

  // Degenerate inputs.
  assert.equal(findNearestMoment([], 100, 10, matchRow(null)), null);
  assert.equal(findNearestMoment(moments, 100, 0, matchRow(null), { strict: true }), null);
  assert.equal(findNearestMoment(moments, 100, 0, matchRow(null), { strict: false }), 100);
  assert.equal(findNearestMoment(moments, NaN, 10, matchRow(null)), null);
});

test("findNearestMoment matches linear scan across random dense moments and queries", () => {
  const rand = makeRng(0x5eedf4);
  const moments = [];
  let cursor = -50;
  for (let i = 0; i < 400; i++) {
    // Mix dense clusters, exact duplicates, and sparse gaps.
    const step = rand() < 0.25 ? 0 : rand() < 0.6 ? Math.floor(rand() * 5) : Math.floor(rand() * 80);
    cursor += step;
    const hitCount = rand() < 0.1 ? 0 : 1 + Math.floor(rand() * 3);
    const hits = [];
    for (let h = 0; h < hitCount; h++) {
      hits.push([Math.floor(rand() * 4), 80, false, false]);
    }
    moments.push([cursor, 80, rand() < 0.5, hits]);
  }

  for (let q = 0; q < 600; q++) {
    const ms = -70 + rand() * (cursor + 100);
    const tolerance = rand() < 0.15 ? 0 : 1 + rand() * 45;
    const row = rand() < 0.25 ? null : Math.floor(rand() * 4);
    const strict = rand() < 0.5;
    const match = (moment) => {
      const [time, , , hits] = moment;
      return (row === null ? hits.length > 0 : hits.some(([lane]) => lane === row)) ? time : undefined;
    };
    const expected = linearNearestMoment(moments, ms, tolerance, match, strict);
    const actual = findNearestMoment(moments, ms, tolerance, match, { strict });
    assert.equal(actual, expected, `mismatch at q=${q} ms=${ms} tol=${tolerance} row=${row} strict=${strict}`);
  }
});

test("nearestHitTime and hitCardAt in booted editor match reference linear scans", () => {
  const { context, ctx } = createEditorHarness();
  const moments = [
    [0, 100, false, [[0, 100, false, false]]],
    [500, 90, true, [[0, 90, false, false]]],
    [510, 80, true, [[0, 80, false, false]]],
    [515, 70, false, [[1, 70, false, false]]],
    [1000, 60, true, [[0, false, false, 60], [1, false, false, 50]]],
  ];
  ctx.state.view = { busy: false, hasDocument: true, duration: 5000 };
  ctx.state.laneRows = [0, 1];
  ctx.state.scene = {
    path: "test.osu",
    viewStart: 0,
    zoom: 1,
    rowHeight: 30,
    moments,
    lanes: [
      { family: "normal-hitnormal", group: "normal", colourKey: "k0" },
      { family: "soft-hitclap", group: "soft", colourKey: "k1" },
    ],
    gameplay: {
      points: [
        ["circle", 500, 0],
        ["slider", 510, 1],
        ["slider", 1000, 1],
      ],
      spans: [],
    },
    combos: { objects: { 0: [1, 0], 1: [2, 0] }, colours: ["#ffffff"] },
  };

  const linearNearestHit = (ms, row, zoom) =>
    linearNearestMoment(
      moments,
      ms,
      hitHalfWidth / zoom,
      ([time, , , hits]) => ((row === null ? hits.length > 0 : hits.some(([lane]) => lane === row)) ? time : undefined),
      true,
    );

  const probes = [-5, 0, 9.9, 10, 490, 490.1, 505, 512, 520, 995, 1010, 1011];
  for (const zoom of [0.5, 1, 2]) {
    for (const ms of probes) {
      for (const row of [null, 0, 1]) {
        assert.equal(
          ctx.nearestHitTime(ms, row, zoom),
          linearNearestHit(ms, row, zoom),
          `nearestHitTime mismatch at ms=${ms} row=${row} zoom=${zoom}`,
        );
      }
    }
  }

  // hitCardAt tie-breaking & boundary parity at zoom = 1 (tolerance = 10 ms):
  // Midpoint 505 between 500 and 510 keeps 510 (matching original `distance <= best` linear scan).
  const cardAtTie = ctx.hitCardAt({ clientX: 505, clientY: 15 });
  assert.ok(cardAtTie?.key.startsWith("0:510:"));
  // Boundary 490 (distance === 10 === tolerance) resolves to 500; 489.9 returns null.
  assert.ok(ctx.hitCardAt({ clientX: 490, clientY: 15 })?.key.startsWith("0:500:"));
  assert.equal(ctx.hitCardAt({ clientX: 489.9, clientY: 15 }), null);
  // Row 1 hover at 1000 shows slider end object name.
  const cardAtEnd = ctx.hitCardAt({ clientX: 1000, clientY: 45 });
  assert.equal(cardAtEnd?.detail, "Volume 50% · on slider 2 end");
  // Verify hitCardAt still agrees inside vm context too.
  assert.equal(runInContext("hitCardAt({ clientX: 0, clientY: 15 })?.time", context), "00:00.000");
});

test("hitObjectName builds and caches gameplay index per gameplay object identity", () => {
  const { ctx } = createEditorHarness();
  const gameplayA = {
    points: [
      ["circle", 100, 0],
      ["circle", 100, 99],
      ["slider", 200, 1],
      ["slider", 300, 1],
      ["slider", 400, 1],
    ],
    spans: [],
  };
  ctx.state.scene = {
    path: "test.osu",
    viewStart: 0,
    zoom: 1,
    rowHeight: 30,
    moments: [],
    gameplay: gameplayA,
    combos: { objects: { 0: [1, 0], 1: [2, 0] }, colours: ["#ffffff"] },
  };

  assert.equal(ctx.hitObjectName(100, false), "no object");
  assert.equal(ctx.hitObjectName(100, true), "circle 1");
  assert.equal(ctx.hitObjectName(200, true), "slider 2");
  assert.equal(ctx.hitObjectName(300, true), "slider 2 repeat");
  assert.equal(ctx.hitObjectName(400, true), "slider 2 end");
  assert.equal(ctx.hitObjectName(999, true), "spinner end");

  const firstCache = ctx.state.gameplayIndexCache;
  assert.equal(firstCache?.gameplay, gameplayA);

  // Mutating gameplayA.points in place must keep reusing the cached index (O(1) per hover).
  gameplayA.points.push(["circle", 999, 2]);
  assert.equal(ctx.hitObjectName(999, true), "spinner end");
  assert.equal(ctx.state.gameplayIndexCache, firstCache);

  // Replacing state.scene.gameplay with a new object rebuilds the cache.
  const gameplayB = {
    points: [["circle", 999, 2]],
    spans: [],
  };
  ctx.state.scene.gameplay = gameplayB;
  ctx.state.scene.combos = { objects: { 2: [7, 0] }, colours: ["#ffffff"] };
  assert.equal(ctx.hitObjectName(999, true), "circle 7");
  assert.equal(ctx.state.gameplayIndexCache?.gameplay, gameplayB);
  assert.notEqual(ctx.state.gameplayIndexCache, firstCache);
});

test("hitCardAt resolves hits on merged secondary drum-row members and displays the drum name", () => {
  const { ctx } = createEditorHarness();
  ctx.state.view = { busy: false, hasDocument: true, drumView: { enabled: true, name: "Auto" } };
  ctx.state.scene = {
    path: "merged.osu",
    viewStart: 0,
    zoom: 1,
    rowHeight: 30,
    lanes: [
      { key: "k1", name: "drum-hitnormal", group: "drum" },
      { key: "k2", name: "drum-hitnormal2", group: "drum" },
    ],
    drumRows: [{ row: 0, rows: [0, 1], name: "Kick", drumRowIndex: 0 }],
    moments: [[500, 75, true, [[1, false, false, 65]]]],
    gameplay: { points: [["circle", 500, 0]], spans: [] },
    combos: { objects: { 0: [1, 0] }, colours: ["#ffffff"] },
  };
  ctx.state.laneRows = [0];
  ctx.state.drumRowMembers = new Map([[0, [0, 1]]]);
  ctx.state.drumRowByMember = new Map([[0, 0], [1, 0]]);
  const card = ctx.hitCardAt({ clientX: 504, clientY: 15 });
  assert.equal(card?.title, "Kick");
  assert.equal(card?.time, "00:00.500");
  assert.equal(card?.detail, "Volume 65% · on circle 1");
});
