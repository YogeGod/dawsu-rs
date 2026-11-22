import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { emptyInspector, problemMarkHandlers, problemStrip, signedClock } from "../src/lib/inspector.js";
import InspectorDifficulty from "../src/parts/InspectorDifficulty.svelte";

// What app.js's inspectorModel() hands over as problemStrip.
const input = (more = {}) => ({
  path: "a.osu", name: "Hard", duration: 200000, problems: [], orphaned: [], objects: [], viewStart: 0, viewWidth: 0, ...more,
});
const problem = (time, more = {}) => ({ target: "a.osu", timeline: "a.osu", time, message: "This object has no hits left.", ...more });

function shown(strip) {
  bridge.inspector = { ...emptyInspector, doc: !!strip, problemStrip: strip };
  bridge.view = strip ? { busy: false } : null;
  bridge.scene = strip ? { path: "a.osu" } : null;
  return render(InspectorDifficulty).body;
}
const marks = html => [...html.matchAll(/<button type="button" class="pm-mark pm-(\w+)"[^>]*>/g)].map(match => ({
  kind: match[1], tag: match[0], label: match[0].match(/aria-label="([^"]+)"/)?.[1],
  tabindex: match[0].match(/tabindex="(-?\d)"/)?.[1],
}));

test("times read as Next problem writes them, signed", () => {
  assert.equal(signedClock(100000), "01:40.000");
  assert.equal(signedClock(-20), "−00:00.020");
});

test("only this difficulty's own Save problems count, once per time, and Generate's targets stay out", () => {
  const strip = problemStrip(input({ problems: [
    problem(50000), problem(50000, { message: "The slider body volume conflicts with its head." }),
    problem(150000, { target: "b.osu" }), problem(-20),
    problem(90000, { target: "Generated.osu", timeline: "a.osu" }),
  ] }));
  assert.equal(strip.unsaved, 2);
  assert.deepEqual(strip.marks.map(mark => [mark.kind, mark.time]), [["unsaved", -20], ["unsaved", 50000]]);
  assert.equal(strip.marks[1].label, "00:50.000 · This object has no hits left. The slider body volume conflicts with its head.");
  assert.equal(strip.marks[0].label, "−00:00.020 · This object has no hits left.");
  const atTime = problemStrip(input({ problems: [problem(4000, { message: "At 4000 ms a slider sound (Slide) is on a hit." })] }));
  assert.equal(atTime.marks[0].label, "At 4000 ms a slider sound (Slide) is on a hit.", "no second time");
});

test("marks are sorted by time and those in one pixel column merge, per kind", () => {
  // 200 s across 100 px: 2 s per column.
  const orphaned = Array.from({ length: 300 }, (_, index) => 60000 + index * 10);
  const strip = problemStrip(input({ orphaned: [190000, ...orphaned], problems: [problem(60500)] }), 100);
  assert.equal(strip.orphaned, 301);
  assert.deepEqual(strip.marks.map(mark => [mark.kind, mark.time, mark.left]),
    [["orphaned", 60000, "30%"], ["unsaved", 60500, "30.25%"], ["orphaned", 62000, "31%"], ["orphaned", 190000, "95%"]]);
  assert.equal(strip.marks[0].label, "01:00.000 · 200 hits with no object");
  assert.equal(strip.marks[2].label, "01:02.000 · 100 hits with no object");
  assert.equal(strip.marks.at(-1).label, "03:10.000 · Hit with no object");
  assert.equal(new Set(strip.marks.map(mark => mark.key)).size, strip.marks.length, "keys are unique");
});

test("object ticks, one per pixel column, and the view frame come from the editor's own data", () => {
  const strip = problemStrip(input({ objects: [0, 500, 1000, 100000, 300000], viewStart: 50000, viewWidth: 20000 }), 100);
  assert.deepEqual(strip.ticks, ["0%", "50%", "100%"], "0, 500 and 1000 ms share column 0; past the end sits at the end");
  assert.deepEqual(strip.frame, { left: "25%", width: "10%" });
  assert.equal(problemStrip(input({ viewWidth: NaN }), 100).frame, null);
});

test("with no map the strip is empty", () => {
  assert.deepEqual(problemStrip(null), { name: "", marks: [], ticks: [], frame: null, unsaved: 0, orphaned: 0 });
});

test("a mark goes to its time; Left, Right, Home and End move between marks", () => {
  const sent = [];
  const on = problemMarkHandlers((...args) => sent.push(args));
  on.click({ time: 50000 });
  assert.deepEqual(sent, [["jump", 50000]]);
  const key = (name, index, count = 4) => {
    const event = { key: name, preventDefault() { this.prevented = true; } };
    return [on.key(event, index, count), !!event.prevented];
  };
  assert.deepEqual(key("ArrowRight", 0), [1, true]);
  assert.deepEqual(key("ArrowRight", 3), [3, true], "the last mark stays");
  assert.deepEqual(key("ArrowLeft", 0), [0, true]);
  assert.deepEqual(key("Home", 2), [0, true]);
  assert.deepEqual(key("End", 0), [3, true]);
  assert.deepEqual(key("Enter", 1), [null, false], "Enter clicks the mark as usual");
});

test("Difficulty shows the strip: one Tab stop, real counts, and the counts it can't make greyed out and empty", () => {
  const html = shown(input({ problems: [problem(50000)], orphaned: [100000], objects: [1000], viewStart: 0, viewWidth: 20000 }));
  assert.match(html, /class="pm-name" title="Hard">Hard</);
  assert.match(html, /class="pm-sum pm-off" title="Not counted yet\." aria-label="Sound right: not counted yet"[^>]*>– of – sound right</);
  for (const word of ["silent", "fallback"]) {
    const name = word[0].toUpperCase() + word.slice(1);
    assert.match(html, new RegExp(`class="pm-count pm-off" title="Not counted yet\\." aria-label="${name}: not counted yet"[^>]*><i[^>]*></i>– ${word}<`), word);
  }
  assert.match(html, /class="pm-count pm-orphaned" title="Moments with hits and no object"[^>]*><i[^>]*><\/i>1 hit with no object</);
  assert.match(html, /class="pm-count pm-unsaved"[^>]*><i[^>]*><\/i>1 can't be saved</);
  assert.match(html, /<div class="pm-track" role="group" aria-label="Problems across the song"/);
  assert.deepEqual(marks(html).map(mark => [mark.kind, mark.label, mark.tabindex]),
    [["unsaved", "00:50.000 · This object has no hits left.", "0"], ["orphaned", "01:40.000 · Hit with no object", "-1"]]);
  assert.match(html, /class="pm-tick"/);
  assert.match(html, /class="pm-frame"/);
  // Positions go through custom properties, which the browser sets through the CSSOM (the CSP blocks style="").
  assert.match(html, /style="--at: 25%;"/);
  for (const [id, text] of [["export-to-mapset", "Export to mapset…"], ["new", "New…"], ["merge-settings", "Merge settings…"]]) {
    assert.match(html, new RegExp(`<button id="${id}"[^>]*>${text}</button>`), text);
  }
});

test("with no map the strip shows no marks, and no Save problem count until a Save check finds some", () => {
  const html = shown(null);
  assert.doesNotMatch(html, /pm-mark/);
  assert.match(html, />0 hits with no object</);
  assert.doesNotMatch(shown(input()), /can't be saved/);
});
