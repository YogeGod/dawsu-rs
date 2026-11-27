// Checks the 30-second hitsounding plan on the committed ranked fixtures: sensible and safe.
//
// node --test tools/robot/hitsound-plan.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { beatPosition, planHitsounds } from "./hitsound-plan.mjs";
import { controlPoints, readOsu, soundMoments } from "./osu-sounds.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const maps = ["catalyst-impetus-std.osu", "mukduk-lazer-v128-bom.osu"].map(name => join(repo, "tests/fixtures/save-pack/owner", name));

for (const path of maps) {
  const bytes = readFileSync(path);
  const plan = planHitsounds(bytes, new Set());
  const lines = controlPoints(readOsu(bytes));
  test(`a human-like plan for ${path.split(/[\\/]/).pop()}`, () => {
    assert.ok(plan.moments.length >= 60, `${plan.moments.length} moments`);
    assert.ok(plan.window.end - plan.window.start === 30000);
    // Every gameplay sound moment in the window is planned (none skipped on these maps).
    const inWindow = soundMoments(bytes).filter(m => m.time >= plan.window.start && m.time < plan.window.end);
    assert.equal(plan.moments.length, new Set(inWindow.map(m => Math.round(beatPosition(lines, m.time).snapped))).size);
    for (const moment of plan.moments) {
      assert.ok(moment.volume >= 5 && moment.volume <= 100, `${moment.time}: ${moment.volume}%`);
      assert.equal(moment.layers[0].sound, "hitnormal", `${moment.time}: no hitnormal first`);
      const sounds = moment.layers.map(l => l.sound);
      assert.ok(!(sounds.includes("hitclap") && sounds.includes("hitfinish")), `${moment.time}: clap and finish together`);
      assert.ok(sounds.length <= 2, `${moment.time}: ${sounds.join("+")}`);
      // One custom index per moment, and only Soft outside kiai.
      assert.equal(new Set(moment.layers.map(l => l.index)).size, 1);
      if (!moment.kiai) assert.ok(moment.layers.every(l => l.bank === "soft"), `${moment.time}: ${moment.layers.map(l => l.lane)}`);
      if (sounds.includes("hitclap")) {
        const where = beatPosition(lines, moment.exact);
        assert.ok(where.onBeat && where.barBeat % 2 === 1, `${moment.time}: clap off the backbeat`);
      }
    }
    // A real hitsound pass has claps and finishes, not just hitnormals.
    assert.ok(plan.moments.some(m => m.layers.some(l => l.sound === "hitclap")));
    assert.ok(plan.moments.some(m => m.layers.some(l => l.sound === "hitfinish")));
  });
}

test("the map's own samples are used when every file of a section is there", () => {
  const bytes = readFileSync(maps[0]);
  const files = new Set(["soft-hitnormal.wav", "soft-hitclap.wav", "soft-hitfinish.wav", "soft-hitwhistle.wav"]);
  const plan = planHitsounds(bytes, files);
  assert.equal(plan.index.calm, 1);
  assert.equal(plan.index.kiai, 0);
});
