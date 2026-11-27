// Checks tools/robot/osu-sounds.mjs against osu!lazer itself: the committed lazer oracle results in
// tests/fixtures/lazer-oracle/ (written by tools/lazer-oracle from lazer's own packages). Every
// sound moment the robot works out must match lazer's samples, banks, indexes, volumes and times.
//
// node --test tools/robot/osu-sounds.test.mjs
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { soundMoments } from "./osu-sounds.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const oracle = join(repo, "tests", "fixtures", "lazer-oracle");

function jsonFiles(dir) {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? jsonFiles(path) : name.endsWith(".json") ? [path] : [];
  });
}

// lazer's sample as the robot writes one.
function fromLazer(sample) {
  if (sample.kind === "file") return { name: "file", file: sample.filename, volume: sample.volume };
  return { name: sample.name, bank: sample.bank, index: sample.custom_sample_bank, volume: sample.volume, layered: !!sample.is_layered };
}
const key = s => s.name === "file" ? `file:${s.file}@${s.volume}` : `${s.bank}-${s.name}:${s.index}@${s.volume}${s.layered ? "+layered" : ""}`;

function lazerMoments(result) {
  const moments = [];
  for (const object of result.hit_objects) {
    if (object.type === "Slider") {
      for (const nested of object.nested ?? []) {
        if (nested.type === "SliderHeadCircle" || nested.type === "SliderRepeat") {
          moments.push({ time: nested.start_time, samples: nested.samples.map(fromLazer) });
        }
      }
      moments.push({ time: object.end_time, samples: (object.tail_samples ?? []).map(fromLazer) });
    } else if (object.type === "Spinner") {
      moments.push({ time: object.end_time, samples: object.samples.map(fromLazer) });
    } else {
      moments.push({ time: object.start_time, samples: object.samples.map(fromLazer) });
    }
  }
  return moments.sort((a, b) => a.time - b.time);
}

const files = jsonFiles(oracle);
test("the oracle results are there", () => assert.ok(files.length > 0, `no JSON under ${oracle}`));

for (const path of files) {
  const result = JSON.parse(readFileSync(path, "utf8"));
  if (result.header?.mode !== 0) continue;
  const source = join(repo, "tests", "fixtures", result.header.input);
  test(`sound moments match lazer: ${relative(oracle, path)}`, () => {
    const mine = soundMoments(readFileSync(source));
    const theirs = lazerMoments(result);
    assert.equal(mine.length, theirs.length, "moment count");
    mine.forEach((moment, i) => {
      assert.ok(Math.abs(moment.time - theirs[i].time) < 0.5, `moment ${i}: time ${moment.time} vs lazer ${theirs[i].time}`);
      assert.deepEqual(moment.samples.map(key).sort(), theirs[i].samples.map(key).sort(), `moment ${i} at ${theirs[i].time} ms`);
    });
  });
}
