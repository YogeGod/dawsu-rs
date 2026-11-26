import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { meterPct, meterPosition } from "../src/lib/mixer.js";
import {
  SPECTRUM_BINS, applyTilt, binToFreq, freqToNote, magmaColour, parseSpectrumFrame,
} from "../src/lib/spectrum.js";
import { bridge } from "../src/lib/bridge.svelte.js";
import SpectrumPanel from "../src/parts/SpectrumPanel.svelte";

test("meterPosition uses nonlinear EBU/REAPER scaling above -60 dB and below 0 dBFS", () => {
  assert.equal(meterPosition(0), 0);
  assert.equal(meterPosition(1), 1);
  // -18 dB sits at 55% instead of linear ~12.6%
  assert.ok(Math.abs(meterPosition(10 ** (-18 / 20)) - 0.55) < 1e-6);
  // -6 dB sits at 85%
  assert.ok(Math.abs(meterPosition(10 ** (-6 / 20)) - 0.85) < 1e-6);
  assert.equal(meterPct(0), 0);
  assert.equal(meterPct(1), 100);
});

test("spectrum helpers map 440 Hz to A4, apply exact octaveless tilt around 1 kHz, and decode 0x02 frames", () => {
  assert.deepEqual(freqToNote(440), { name: "A4", midi: 69, cents: 0 });
  assert.equal(freqToNote(261.6256)?.name, "C4");
  assert.equal(applyTilt(-20, 1000, 4.5), -20);
  assert.ok(Math.abs(applyTilt(-20, 2000, 4.5) - (-15.5)) < 1e-6);
  assert.ok(Math.abs(applyTilt(-20, 500, 4.5) - (-24.5)) < 1e-6);
  assert.ok(binToFreq(0, SPECTRUM_BINS) >= 19.9 && binToFreq(SPECTRUM_BINS - 1, SPECTRUM_BINS) <= 20001);
  assert.match(magmaColour(0.75), /^rgb\(\d+,\d+,\d+\)$/);

  const buf = new ArrayBuffer(272);
  const dv = new DataView(buf);
  dv.setUint8(0, 0x02);
  dv.setUint8(1, 0x03);
  dv.setUint16(2, 128, true);
  dv.setFloat32(4, 440.0, true);
  dv.setFloat32(8, -2.5, true);
  dv.setInt16(12, 69, true);
  dv.setInt16(14, -3, true);
  dv.setInt16(16, -2450, true);
  const decoded = parseSpectrumFrame(buf);
  assert.equal(decoded.version, 2);
  assert.equal(decoded.active, true);
  assert.equal(decoded.hasPitch, true);
  assert.equal(decoded.peakHz, 440);
  assert.equal(decoded.midiNote, 69);
  assert.equal(decoded.cents, -3);
  assert.equal(decoded.binsDb[0], -24.5);
});

test("SpectrumPanel renders cleanly on server with active pitch and tilt controls", () => {
  bridge.setSpectrum({
    version: 2, active: true, hasPitch: true, peakHz: 440, peakDb: -2,
    midiNote: 69, cents: 0, binsDb: new Float32Array(SPECTRUM_BINS).fill(-24),
  });
  const html = render(SpectrumPanel).body;
  bridge.setSpectrum(null);
  assert.match(html, /aria-label="Spectrum analyzer"/);
  assert.match(html, /A4 · 440\.0 Hz \(\+0c\)/);
  assert.match(html, /\+4\.5/);
  assert.doesNotMatch(html, /dBFS|kHz|Default device/);
});
