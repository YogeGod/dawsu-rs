#!/usr/bin/env node
/**
 * Generates a synthetic, licence-free 2-difficulty demo osu! mapset for learning dawsu!.
 *
 * Includes:
 * - A 120 BPM synthetic backing track (`demo-song.wav`, 8 seconds)
 * - Clean synthesized hitsound samples (`drum-hitnormal.wav`, `drum-hitclap.wav`, `soft-hitwhistle.wav`)
 * - Two osu!standard difficulties:
 *   - `[Hard].osu` with hits on beats 1, 2, 3, 4 (including an extra clap on beat 3.5)
 *   - `[Normal].osu` with simpler rhythm (intentionally missing the beat 3.5 circle so mappers
 *     can test Coverage Review, Shared Base projection, and Export right away)
 *
 * Usage:
 *   node tools/make-demo-mapset.mjs [output-dir]
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";

const outDir = resolve(process.argv[2] || "demo-mapset");
const SAMPLE_RATE = 44100;

function writePcm16Wav(filePath, samples) {
  const numFrames = samples.length;
  const dataBytes = numFrames * 2;
  const buf = Buffer.alloc(44 + dataBytes);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataBytes, 4);
  buf.write("WAVEfmt ", 8);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(1, 22); // mono
  buf.writeUInt32LE(SAMPLE_RATE, 24);
  buf.writeUInt32LE(SAMPLE_RATE * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(dataBytes, 40);
  for (let i = 0; i < numFrames; i++) {
    const clamped = Math.max(-1, Math.min(1, samples[i]));
    buf.writeInt16LE(Math.round(clamped * 32767), 44 + i * 2);
  }
  writeFileSync(filePath, buf);
}

function synthKick(durationSec = 0.14) {
  const n = Math.floor(SAMPLE_RATE * durationSec);
  const out = new Float32Array(n);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const t = i / SAMPLE_RATE;
    const freq = 55 + 120 * Math.exp(-t * 36);
    phase += (2 * Math.PI * freq) / SAMPLE_RATE;
    const env = Math.exp(-t * 22);
    out[i] = Math.sin(phase) * env * 0.85;
  }
  return out;
}

function synthClap(durationSec = 0.12) {
  const n = Math.floor(SAMPLE_RATE * durationSec);
  const out = new Float32Array(n);
  let seed = 1337;
  const rand = () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return (seed / 0x7fffffff) * 2 - 1;
  };
  for (let i = 0; i < n; i++) {
    const t = i / SAMPLE_RATE;
    const burst = t < 0.018 ? 0.9 : Math.exp(-(t - 0.018) * 32);
    out[i] = rand() * burst * 0.65;
  }
  return out;
}

function synthWhistle(durationSec = 0.16) {
  const n = Math.floor(SAMPLE_RATE * durationSec);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / SAMPLE_RATE;
    const env = Math.exp(-t * 18);
    out[i] = (Math.sin(2 * Math.PI * 1320 * t) * 0.45 + Math.sin(2 * Math.PI * 1980 * t) * 0.2) * env;
  }
  return out;
}

function synthSong(durationSec = 8.0) {
  const n = Math.floor(SAMPLE_RATE * durationSec);
  const out = new Float32Array(n);
  const chords = [220, 261.63, 329.63, 392.0];
  for (let i = 0; i < n; i++) {
    const t = i / SAMPLE_RATE;
    const beat = Math.floor(t / 0.5);
    const beatFrac = (t % 0.5) / 0.5;
    const pulse = Math.exp(-beatFrac * 6) * 0.18;
    const bass = Math.sin(2 * Math.PI * (beat % 4 < 2 ? 110 : 130.81) * t) * pulse;
    let pad = 0;
    for (const f of chords) {
      pad += Math.sin(2 * Math.PI * f * t) * 0.045;
    }
    out[i] = bass + pad;
  }
  return out;
}

function buildOsu(version, objects) {
  return [
    "osu file format v14",
    "",
    "[General]",
    "AudioFilename: demo-song.wav",
    "AudioLeadIn: 0",
    "PreviewTime: 1000",
    "Countdown: 0",
    "SampleSet: Soft",
    "StackLeniency: 0.7",
    "Mode: 0",
    "",
    "[Metadata]",
    "Title: dawsu! Demo Track",
    "Artist: dawsu! Studio",
    "Creator: Demo",
    `Version: ${version}`,
    "",
    "[Difficulty]",
    "HPDrainRate: 5",
    "CircleSize: 4",
    "OverallDifficulty: 6",
    "ApproachRate: 8",
    "SliderMultiplier: 1.4",
    "SliderTickRate: 1",
    "",
    "[TimingPoints]",
    "500,500,4,2,1,85,1,0",
    "",
    "[HitObjects]",
    ...objects,
    "",
  ].join("\r\n");
}

mkdirSync(outDir, { recursive: true });
writePcm16Wav(join(outDir, "demo-song.wav"), synthSong());
writePcm16Wav(join(outDir, "drum-hitnormal.wav"), synthKick());
writePcm16Wav(join(outDir, "drum-hitclap.wav"), synthClap());
writePcm16Wav(join(outDir, "soft-hitwhistle.wav"), synthWhistle());

const hardObjects = [
  "128,192,1000,1,0,3:0:1:90:",
  "224,192,1500,1,8,3:3:1:85:",
  "320,192,2000,1,0,3:0:1:90:",
  "384,192,2250,1,8,3:3:1:80:",
  "320,256,2500,1,2,2:2:1:85:",
  "192,256,3000,2,0,L|352:256,1,140,0|2,3:0|2:2,0:0:0:0:",
  "128,192,4000,1,0,3:0:1:95:",
  "256,192,4500,1,8,3:3:1:90:",
  "384,192,5000,1,2,2:2:1:85:",
];

// Simpler rhythm: no circle at 2250 ms (demonstrates orphaned hit) and an extra circle at 5500 ms.
const normalObjects = [
  "128,192,1000,1,0,0:0:0:0:",
  "256,192,1500,1,0,0:0:0:0:",
  "384,192,2000,1,0,0:0:0:0:",
  "256,256,2500,1,0,0:0:0:0:",
  "192,256,3000,2,0,L|352:256,1,140,0|0,0:0|0:0,0:0:0:0:",
  "128,192,4000,1,0,0:0:0:0:",
  "256,192,4500,1,0,0:0:0:0:",
  "384,192,5000,1,0,0:0:0:0:",
  "256,128,5500,1,0,0:0:0:0:",
];

writeFileSync(join(outDir, "dawsu! Studio - dawsu! Demo Track (Demo) [Hard].osu"), buildOsu("Hard", hardObjects));
writeFileSync(join(outDir, "dawsu! Studio - dawsu! Demo Track (Demo) [Normal].osu"), buildOsu("Normal", normalObjects));

console.log(`Created synthetic demo mapset in ${outDir}`);
