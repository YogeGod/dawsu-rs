import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { emptyInspector, inspectorHandlers, problemStrip } from "../src/lib/inspector.js";
import InspectorDifficulty from "../src/parts/InspectorDifficulty.svelte";

const scene = (more = {}) => ({ path: "a.osu", version: "Hard", orphanedHits: [], ...more });
const problem = (time, more = {}) => ({ target: "a.osu", timeline: "a.osu", time, message: "This object has no hits left.", ...more });

function shown(view, current) {
  bridge.inspector = { ...emptyInspector, doc: !!current };
  bridge.view = view;
  bridge.scene = current;
  return render(InspectorDifficulty).body;
}

test("the strip marks each Save problem of this difficulty and each hit with no object at its time", () => {
  const view = { duration: 200000, outputProblems: [problem(50000), problem(150000, { timeline: "b.osu" }), problem(-20)] };
  const strip = problemStrip(view, scene({ orphanedHits: [100000, 250000] }));
  assert.equal(strip.name, "Hard");
  assert.deepEqual([strip.unsaved, strip.orphaned], [2, 2], "another difficulty's problems stay out");
  assert.deepEqual(strip.marks.map(mark => [mark.kind, mark.time, mark.left]), [
    ["orphaned", 100000, "50%"], ["orphaned", 250000, "100%"], ["unsaved", 50000, "25%"], ["unsaved", -20, "0%"],
  ]);
  assert.equal(strip.marks[0].label, "01:40.000 · Hit with no object");
  assert.equal(strip.marks[2].label, "00:50.000 · This object has no hits left.");
});

test("with no map, no song length or no lists, the strip has no marks", () => {
  assert.deepEqual(problemStrip(null, null), { name: "", marks: [], unsaved: 0, orphaned: 0 });
  const strip = problemStrip({}, { path: "a.osu" });
  assert.deepEqual([strip.marks, strip.unsaved, strip.orphaned], [[], 0, 0]);
  assert.equal(problemStrip({ duration: 0 }, scene({ orphanedHits: [500] })).marks[0].left, "100%");
});

test("Difficulty shows the strip: real counts, and the counts the editor can't make greyed out and empty", () => {
  const html = shown({ duration: 200000, outputProblems: [problem(50000)] }, scene({ orphanedHits: [100000] }));
  assert.match(html, /class="pm-name" title="Hard">Hard</);
  assert.match(html, /class="pm-sum pm-off"[^>]*title="[^"]+"[^>]*>– of – sound right</);
  for (const word of ["silent", "fallback"]) {
    assert.match(html, new RegExp(`class="pm-count pm-off"[^>]*title="[^"]+"[^>]*><i[^>]*></i>– ${word}<`), word);
  }
  assert.match(html, /class="pm-count pm-orphaned"[^>]*><i[^>]*><\/i>1 orphaned</);
  assert.match(html, /class="pm-count pm-unsaved"[^>]*><i[^>]*><\/i>1 can't be saved</);
  const marks = [...html.matchAll(/<button type="button" class="pm-mark pm-(\w+)"[^>]*aria-label="([^"]+)"/g)].map(m => [m[1], m[2]]);
  assert.deepEqual(marks, [["orphaned", "01:40.000 · Hit with no object"], ["unsaved", "00:50.000 · This object has no hits left."]]);
  // Positions go through a custom property, which the browser sets through the CSSOM (the CSP blocks style="").
  assert.match(html, /style="--at: 50%;"/);
  for (const [id, text] of [["generate", "Generate…"], ["new", "New…"], ["copy", "Copy…"], ["merge-settings", "Merge settings…"]]) {
    assert.match(html, new RegExp(`<button id="${id}"[^>]*>${text}</button>`), text);
  }
});

test("with no map the strip shows no marks, and no Save problem count until a Save check finds some", () => {
  const html = shown(null, null);
  assert.doesNotMatch(html, /pm-mark/);
  assert.match(html, />0 orphaned</);
  assert.doesNotMatch(shown({ duration: 1000, outputProblems: [] }, scene()), /can't be saved/);
});

test("a moment an Export review lists once per target gets one mark per listing", () => {
  const view = { duration: 1000, outputProblems: [problem(500, { target: "b.osu" }), problem(500, { target: "c.osu" })] };
  assert.equal(problemStrip(view, scene()).marks.length, 2);
  assert.equal([...shown(view, scene()).matchAll(/class="pm-mark/g)].length, 2);
});

test("a mark goes to its time", () => {
  const sent = [];
  inspectorHandlers((...args) => sent.push(args)).jump(50000);
  assert.deepEqual(sent, [["jump", 50000]]);
  const source = readFileSync(new URL("../src/parts/ProblemMap.svelte", import.meta.url), "utf8");
  assert.match(source, /onclick=\{\(\) => on\.jump\(mark\.time\)\}/);
});
