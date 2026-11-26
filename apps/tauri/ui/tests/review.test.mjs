import { test } from "node:test";
import assert from "node:assert/strict";
import { reviewMixSections, reviewSampleWarnings } from "../src/lib/review.js";

test("the review lists new and reused .wav files apart and keeps each mix's audition index", () => {
  const review = {
    details: ["Copy from Hard", "Per-object .osu volumes only: 3 object(s) updated, 0 new timing lines needed.",
      "Sample mixes: 1 new .wav created, 1 existing .wav reused."],
    mixes: ["soft-hitclap2.wav", "normal-hitwhistle3.wav"],
    createdMixes: ["normal-hitwhistle3.wav"],
    reusedMixes: ["soft-hitclap2.wav"],
    volumeSummary: ["Per-object .osu volumes only: 3 object(s) updated, 0 new timing lines needed.",
      "Sample mixes: 1 new .wav created, 1 existing .wav reused."],
  };
  const { sections, volumes, details } = reviewMixSections(review);
  assert.deepEqual(sections, [
    { title: "New .wav files dawsu! creates", rows: [{ path: "normal-hitwhistle3.wav", index: 1 }] },
    { title: "Existing .wav files reused (left as they are)", rows: [{ path: "soft-hitclap2.wav", index: 0 }] },
  ]);
  assert.deepEqual(volumes, review.volumeSummary);
  assert.deepEqual(details, ["Copy from Hard"], "volume lines show once, under Volumes");
});

test("a review without the split lists every mix under one heading, and nothing when there are none", () => {
  assert.deepEqual(reviewMixSections({ details: ["a"], mixes: ["x.wav"] }), {
    sections: [{ title: "Sample mixes", rows: [{ path: "x.wav", index: 0 }] }], volumes: [], details: ["a"],
  });
  assert.deepEqual(reviewMixSections({ details: [], mixes: [], createdMixes: [], reusedMixes: [] }),
    { sections: [], volumes: [], details: [] });
});

test("reviewSampleWarnings returns true-peak and RC sample warnings without changing reviewMixSections shape", () => {
  const sampleWarnings = [
    "hot-cymbal.wav reaches +0.5 dBTP (above -0.3 dBTP) — inter-sample clipping risk in lossy decode or playback.",
  ];
  assert.deepEqual(reviewSampleWarnings({ sampleWarnings }), sampleWarnings);
  assert.deepEqual(reviewSampleWarnings({}), []);
});
