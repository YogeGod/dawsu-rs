import { test } from "node:test";
import assert from "node:assert/strict";
import { normalizeMessage, flatten, diffIssues, evaluateCase, checksThatRan } from "./oracle.mjs";

const response = (cats, timings = []) => ({
  checks: { 1: { name: "Low volume hit sounding." }, 2: { name: "Too short hit sounds." }, 3: { name: "Unused timing lines." } },
  general: { category: "General", checkResults: cats.General ?? [] },
  difficulties: Object.entries(cats)
    .filter(([k]) => k !== "General")
    .map(([category, checkResults]) => ({ category, checkResults })),
  checkTimings: { checks: timings.map((checkName) => ({ checkName })) },
});
const muted = (t) => ({ id: 1, level: "Warning", message: `${t} (1) - 10% volume circle, this may be hard to hear over the song.` });

test("timestamps stay, other numbers become #", () => {
  assert.equal(normalizeMessage("00:01:500 (2,3) - 10% volume, 1.5x"), "00:01:500 (#,#) - #% volume, #x");
});

test("two same-template issues at different times count as two", () => {
  const before = flatten(response({ Hard: [muted("00:01:000")] }));
  const after = flatten(response({ Hard: [muted("00:01:000"), muted("00:02:000")] }));
  const { failing } = diffIssues(before, after);
  assert.equal(failing.length, 1);
  assert.match(failing[0].message, /00:02:000/);
});

test("same issue repeated at one time counts per occurrence", () => {
  const before = flatten(response({ Hard: [muted("00:01:000")] }));
  const after = flatten(response({ Hard: [muted("00:01:000"), muted("00:01:000")] }));
  assert.equal(diffIssues(before, after).failing.length, 1);
});

test("a changed measured value is not new", () => {
  const msg = (ms) => ({ id: 2, level: "Problem", message: `"a.wav" is shorter than 25 ms (${ms} ms).` });
  const before = flatten(response({ General: [msg(20)] }));
  const after = flatten(response({ General: [msg(21)] }));
  assert.equal(diffIssues(before, after).failing.length, 0);
});

test("a difficulty only in after has all its issues counted", () => {
  const before = flatten(response({ Hard: [muted("00:01:000")] }));
  const after = flatten(response({ Hard: [muted("00:01:000")], Insane: [muted("00:01:000")] }));
  const { failing } = diffIssues(before, after);
  assert.equal(failing.length, 1);
  assert.equal(failing[0].category, "Insane");
});

test("Error level fails, Minor is only reported", () => {
  const after = flatten(response({
    General: [{ id: 2, level: "Error", message: "Unable to check." }],
    Hard: [{ id: 3, level: "Minor", message: "00:06:000 - Inherited line affects nothing." }],
  }));
  const { failing, minor } = diffIssues([], after);
  assert.deepEqual(failing.map((r) => r.level), ["Error"]);
  assert.deepEqual(minor.map((r) => r.level), ["Minor"]);
});

test("allowlist cancels one occurrence per entry", () => {
  const after = flatten(response({ Hard: [muted("00:01:000"), muted("00:02:000")] }));
  const { failing } = diffIssues([], after, [after[0].key]);
  assert.equal(failing.length, 1);
  assert.equal(failing[0].key, after[1].key);
});

test("checks that ran come from timings, deduplicated", () => {
  assert.deepEqual(checksThatRan(response({}, ["B", "A", "B"])), ["A", "B"]);
});

const ok = (json) => async () => ({ status: 200, body: JSON.stringify(json) });

test("identical before and after pass", async () => {
  const json = response({ Hard: [muted("00:01:000")] });
  const r = await evaluateCase({ name: "x", beforeDir: "b", afterDir: "a", post: ok(json) });
  assert.equal(r.result, "PASS");
});

test("before broken skips, after broken fails, timeout fails", async () => {
  const good = JSON.stringify(response({}));
  const by = (map) => async (dir) => map[dir]();
  const s500 = async () => ({ status: 500, body: "boom" });
  const timeout = async () => { const e = new Error("t"); e.name = "TimeoutError"; throw e; };
  const fine = async () => ({ status: 200, body: good });

  assert.equal((await evaluateCase({ name: "s", beforeDir: "b", afterDir: "a", post: by({ b: s500, a: fine }) })).result, "SKIP");
  const f = await evaluateCase({ name: "f", beforeDir: "b", afterDir: "a", post: by({ b: fine, a: s500 }) });
  assert.equal(f.result, "FAIL");
  assert.match(f.reason, /HTTP 500/);
  const t = await evaluateCase({ name: "t", beforeDir: "b", afterDir: "a", post: by({ b: fine, a: timeout }) });
  assert.equal(t.result, "FAIL");
  assert.match(t.reason, /timed out/);
});

test("new warning fails the case and names the check", async () => {
  const before = response({});
  const after = response({ Hard: [muted("00:01:000")] });
  const r = await evaluateCase({ name: "m", beforeDir: "b", afterDir: "a", post: async (d) => ({ status: 200, body: JSON.stringify(d === "b" ? before : after) }) });
  assert.equal(r.result, "FAIL");
  assert.equal(r.failing[0].check, "Low volume hit sounding.");
});
