import assert from "node:assert/strict";
import { test } from "node:test";
import {
  applyVelocityLollipopDrag,
  gainToMidiVelocity,
  midiVelocityToOsuVolume,
  osuVolumeToMidiVelocity,
  velocityTier,
  yToVelocity,
} from "../src/lib/velocityLane.js";

test("gainToMidiVelocity matches Rust drum_vst::gain_to_midi_velocity golden vectors", () => {
  assert.equal(gainToMidiVelocity(0.0), 1);
  assert.equal(gainToMidiVelocity(0.5), 64);
  assert.equal(gainToMidiVelocity(0.8), 102);
  assert.equal(gainToMidiVelocity(1.0), 127);
  assert.equal(osuVolumeToMidiVelocity(100), 127);
  assert.equal(midiVelocityToOsuVolume(127), 100);
  assert.equal(midiVelocityToOsuVolume(1), 5);
});

test("velocityTier and yToVelocity classify Ghost / Mid / Hard", () => {
  assert.equal(velocityTier(50), "ghost");
  assert.equal(velocityTier(72), "mid");
  assert.equal(velocityTier(105), "hard");
  assert.equal(yToVelocity(0, 100), 127);
  assert.equal(yToVelocity(100, 100), 1);
});

test("applyVelocityLollipopDrag uses per_hit scope and preserves multi-select differences", () => {
  const single = applyVelocityLollipopDrag([{ id: "hat", volume: 80 }], 0, 64);
  assert.equal(single[0].scope, "per_hit");
  assert.equal(single[0].volume, 50);

  const multi = applyVelocityLollipopDrag(
    [{ id: "kick", volume: 90 }, { id: "hat", volume: 60 }],
    0,
    102, // ~80% (-10 delta)
  );
  assert.equal(multi[0].volume, 80);
  assert.equal(multi[1].volume, 50);
});
