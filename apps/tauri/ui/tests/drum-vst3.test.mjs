import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  DRUM_MAP_TRACK_NAME, DRUM_NOTE_MS, DRUM_VST3_KEY, GM_DRUM_NOTES,
  detectedRowsToDrumNotes, drumRowToMidiNote, mapDrumRowsToMidi, previewDrumVst3Hit,
  readDrumVst3, sceneDrumHitsToNotes, syncDrumVst3Track, volumeToVelocity,
  vst3InstrumentOptions, writeDrumVst3,
} from "../src/lib/drumVst3.js";
import { isChecked } from "../src/lib/drum.js";
import DrumDetectWindow from "../src/parts/DrumDetectWindow.svelte";

const fakeStorage = (entries = {}) => {
  const data = new Map(Object.entries(entries));
  return {
    getItem: key => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: key => data.delete(key),
    data,
  };
};

test("GM drum-to-MIDI mapping maps every standard drum map row to its GM note", () => {
  const cases = [
    ["Kick", 36],
    ["Kick 2", 36],
    ["Bass Drum", 36],
    ["Snare", 38],
    ["Snare 2", 38],
    ["Rim", 37],
    ["Cross-stick", 37],
    ["Side Stick", 37],
    ["Rimshot", 37],
    ["Clap", 39],
    ["Hand Clap", 39],
    ["Closed Hi-Hat", 42],
    ["Hi-Hat", 42],
    ["Pedal Hi-Hat", 44],
    ["Open Hi-Hat", 46],
    ["Low Tom", 45],
    ["Floor Tom", 45],
    ["Mid Tom", 47],
    ["Tom", 47],
    ["High Tom", 50],
    ["Crash", 49],
    ["Crash 2", 49],
    ["Ride", 51],
    ["Percussion", 39],
    ["Other", 39],
    ["Whistle", 39],
    ["", 39],
  ];
  for (const [name, expected] of cases) {
    assert.equal(drumRowToMidiNote(name), expected, `expected ${name} -> GM ${expected}`);
  }
  assert.equal(GM_DRUM_NOTES.kick, 36);
  assert.equal(GM_DRUM_NOTES.snare, 38);
  assert.equal(GM_DRUM_NOTES.ride, 51);
});

test("drumRowToMidiNote honors explicit valid midiNote on a row object and falls back to name", () => {
  assert.equal(drumRowToMidiNote({ name: "Kick 2", midiNote: 35 }), 35);
  assert.equal(drumRowToMidiNote({ name: "Snare", midiNote: null }), 38);
  assert.equal(drumRowToMidiNote({ name: "Open Hi-Hat", midiNote: 200 }), 46);
  const mapped = mapDrumRowsToMidi([
    { index: 0, name: "Kick", laneKey: "BeatmapFile|drum-hitnormal.wav", assigned: true },
    { index: 1, name: "Ride", laneKey: null, assigned: false },
  ]);
  assert.deepEqual(mapped, [
    { index: 0, name: "Kick", laneKey: "BeatmapFile|drum-hitnormal.wav", assigned: true, midiNote: 36 },
    { index: 1, name: "Ride", laneKey: null, assigned: false, midiNote: 51 },
  ]);
});

test("drum VST3 toggle and selected plugin persist in localStorage under dawsu.drumVst3", () => {
  assert.equal(DRUM_VST3_KEY, "dawsu.drumVst3");
  const storage = fakeStorage();
  assert.deepEqual(readDrumVst3(storage), { enabled: false, pluginId: "", pluginName: "", pluginPath: "" });

  writeDrumVst3(storage, {
    enabled: true,
    pluginId: "uid-ad2",
    pluginName: "Addictive Drums 2",
    pluginPath: "C:/Program Files/Common Files/VST3/Addictive Drums 2.vst3",
  });
  assert.deepEqual(readDrumVst3(storage), {
    enabled: true,
    pluginId: "uid-ad2",
    pluginName: "Addictive Drums 2",
    pluginPath: "C:/Program Files/Common Files/VST3/Addictive Drums 2.vst3",
  });

  // Malformed or unavailable storage safely falls back to defaults.
  assert.deepEqual(readDrumVst3(fakeStorage({ [DRUM_VST3_KEY]: "{not-json" })),
    { enabled: false, pluginId: "", pluginName: "", pluginPath: "" });
  assert.deepEqual(readDrumVst3(null), { enabled: false, pluginId: "", pluginName: "", pluginPath: "" });
  assert.deepEqual(readDrumVst3({ getItem() { throw new Error("blocked"); } }),
    { enabled: false, pluginId: "", pluginName: "", pluginPath: "" });
});

test("vst3InstrumentOptions filters to instruments and sorts by name for both vst3_scan and summary shapes", () => {
  const report = {
    plugins: [
      { uid: "2", name: "Superior Drummer 3", vendor: "Toontrack", path: "C:/VST3/SD3.vst3", is_instrument: true },
      { uid: "3", name: "Valhalla Room", vendor: "Valhalla", path: "C:/VST3/Room.vst3", is_instrument: false },
      { uid: "1", name: "Addictive Drums 2", vendor: "XLN", path: "C:/VST3/AD2.vst3", is_instrument: true },
    ],
  };
  assert.deepEqual(vst3InstrumentOptions(report), [
    { id: "1", name: "Addictive Drums 2", vendor: "XLN", path: "C:/VST3/AD2.vst3" },
    { id: "2", name: "Superior Drummer 3", vendor: "Toontrack", path: "C:/VST3/SD3.vst3" },
  ]);
  assert.equal(vst3InstrumentOptions(null).length, 0);
});

test("detectedRowsToDrumNotes and sceneDrumHitsToNotes convert hits into deduplicated sorted GM MIDI notes", () => {
  assert.equal(volumeToVelocity(100), 127);
  assert.equal(volumeToVelocity(0.5), 64);
  assert.equal(volumeToVelocity(0), 1);

  const detected = detectedRowsToDrumNotes([
    { id: "s1", drum: "snare", time: 1000, strength: 0.8, lands: "circle" },
    { id: "k1", drum: "kick", time: 500, strength: 1.0, lands: "circle" },
    { id: "k2", drum: "kick", time: 500.2, strength: 0.5, lands: "circle" },
    { id: "f1", drum: "kick", time: 1500, strength: 0.4, lands: "flagged" },
  ], {}, isChecked);
  assert.deepEqual(detected, [
    { startMs: 500, pitch: 36, velocity: 127 },
    { startMs: 1000, pitch: 38, velocity: 102 },
  ]);

  const scene = {
    drumRows: [
      { row: 0, name: "Kick / Kick 2", drumRowIndex: 0 },
      { row: 1, name: "Closed Hi-Hat", drumRowIndex: 1 },
    ],
    moments: [
      [250, 80, true, [[0, false], [1, false]]],
      [500, 100, true, [[1, false]]],
    ],
  };
  assert.deepEqual(sceneDrumHitsToNotes(scene, null), [
    { startMs: 250, pitch: 36, velocity: 102 },
    { startMs: 250, pitch: 42, velocity: 102 },
    { startMs: 500, pitch: 42, velocity: 127 },
  ]);
});

test("syncDrumVst3Track creates/updates the Drum Map (VST3) keysound track and routes hits via keysounds_action", async () => {
  const calls = [];
  let tracks = [];
  let nextId = 1;
  const fakeInvoke = async (cmd, args) => {
    calls.push({ cmd, payload: args.payload });
    const p = args.payload;
    if (p.kind === "delete_track") {
      tracks = tracks.filter(t => t.id !== p.trackId);
    } else if (p.kind === "add_track") {
      tracks.push({ id: nextId++, name: p.name, notes: [], instrument: p.vst3Plugin ?? "" });
    } else if (p.kind === "add_note") {
      const t = tracks.find(track => track.id === p.trackId);
      t?.notes.push({ startMs: p.startMs, endMs: p.endMs, pitch: p.pitch, velocity: p.velocity });
    }
    return { sidecarAvailable: true, tracks: structuredClone(tracks), vst3Plugins: [], loadedPlugins: [] };
  };

  const plugin = { id: "uid-ad2", name: "Addictive Drums 2", path: "C:/VST3/AD2.vst3" };
  const hits = [
    { startMs: 500, pitch: 36, velocity: 120 },
    { startMs: 1000, pitch: 38, velocity: 105 },
  ];
  const first = await syncDrumVst3Track(fakeInvoke, { plugin, hits });
  assert.equal(first.trackId, 1);
  assert.equal(first.addedNotes, 2);
  assert.equal(tracks.length, 1);
  assert.equal(tracks[0].name, DRUM_MAP_TRACK_NAME);
  assert.deepEqual(tracks[0].notes, [
    { startMs: 500, endMs: 500 + DRUM_NOTE_MS, pitch: 36, velocity: 120 },
    { startMs: 1000, endMs: 1000 + DRUM_NOTE_MS, pitch: 38, velocity: 105 },
  ]);
  assert.ok(calls.some(c => c.cmd === "keysounds_action" && c.payload.kind === "load_vst3" && c.payload.pluginUid === "uid-ad2"));

  // Re-syncing replaces the existing Drum Map (VST3) track cleanly instead of duplicating notes.
  const second = await syncDrumVst3Track(fakeInvoke, { plugin, hits: [{ startMs: 750, pitch: 42, velocity: 90 }] });
  assert.equal(second.trackId, 2);
  assert.equal(tracks.length, 1);
  assert.equal(tracks[0].notes.length, 1);
  assert.equal(tracks[0].notes[0].pitch, 42);

  // Previewing a drum row sends its GM note via keysounds_action preview_note.
  const previewPitch = await previewDrumVst3Hit(fakeInvoke, "Open Hi-Hat", 110);
  assert.equal(previewPitch, 46);
  const lastCall = calls.at(-1);
  assert.deepEqual(lastCall.payload, { kind: "preview_note", trackId: 0, pitch: 46, velocity: 110, durationMs: 200 });
});

test("DrumDetectWindow, KeysoundsModal, and Drum Map Setup expose the Use VST3 sounds for drum map toggle", () => {
  const { body } = render(DrumDetectWindow);
  assert.match(body, /Use VST3 sounds for drum map/);
  assert.match(body, /aria-label="VST3 drum instrument"/);

  const keysounds = readFileSync(new URL("../src/parts/KeysoundsModal.svelte", import.meta.url), "utf8");
  assert.match(keysounds, /Use VST3 sounds for drum map/);
  assert.match(keysounds, /syncDrumVst3Track/);

  const lanes = readFileSync(new URL("../src/editor/lanes.js", import.meta.url), "utf8");
  assert.match(lanes, /Use VST3 sounds for drum map/);
  assert.match(lanes, /invoke\("vst3_scan"\)/);
  assert.match(lanes, /syncDrumVst3Track/);
});
