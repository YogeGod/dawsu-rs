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
import { bridge } from "../src/lib/bridge.svelte.js";
import { laneHandlers } from "../src/lib/laneHandlers.js";
import { attachLanes } from "../src/editor/lanes.js";
import DrumDetectWindow from "../src/parts/DrumDetectWindow.svelte";
import LaneHeaders from "../src/parts/LaneHeaders.svelte";
import MidiVstTab from "../src/parts/preferences/MidiVstTab.svelte";

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
      { row: 2, name: "Snare", drumRowIndex: 2, midiNote: 40 },
    ],
    moments: [
      [250, 80, true, [[0, false], [1, false], [2, false]]],
      [500, 100, true, [[1, false]]],
    ],
  };
  assert.deepEqual(sceneDrumHitsToNotes(scene, null), [
    { startMs: 250, pitch: 36, velocity: 102 },
    { startMs: 250, pitch: 40, velocity: 102 },
    { startMs: 250, pitch: 42, velocity: 102 },
    { startMs: 500, pitch: 42, velocity: 127 },
  ]);

  // Merged drum rows map hits on any member row in entry.rows to the row's MIDI note.
  const mergedScene = {
    drumRows: [
      { row: 0, rows: [0, 3, 4], name: "Kick", drumRowIndex: 0 },
      { row: 1, rows: [1], name: "Closed Hi-Hat", drumRowIndex: 1 },
    ],
    moments: [
      [100, 90, true, [[3, false]]],
      [300, 100, true, [[4, false], [1, false]]],
    ],
  };
  assert.deepEqual(sceneDrumHitsToNotes(mergedScene, null), [
    { startMs: 100, pitch: 36, velocity: 114 },
    { startMs: 300, pitch: 36, velocity: 127 },
    { startMs: 300, pitch: 42, velocity: 127 },
  ]);
});

test("syncDrumVst3Track sends atomic sync_drum_track via keysounds_action without deleting user tracks", async () => {
  const calls = [];
  let tracks = [{ id: 99, name: DRUM_MAP_TRACK_NAME, isDrumSync: false, notes: [{ startMs: 10, endMs: 20, pitch: 60, velocity: 90 }] }];
  let nextId = 1;
  const fakeInvoke = async (cmd, args) => {
    calls.push({ cmd, payload: args.payload });
    const p = args.payload;
    if (p.kind === "sync_drum_track") {
      let existing = tracks.find(t => t.isDrumSync);
      if (!existing) {
        existing = { id: nextId++, name: p.trackName, isDrumSync: true, notes: [], instrument: p.pluginName ?? "" };
        tracks.push(existing);
      }
      existing.notes = p.notes;
      existing.instrument = p.pluginName ?? existing.instrument;
    }
    const syncTrack = tracks.find(t => t.isDrumSync);
    return {
      sidecarAvailable: true,
      activeTrackId: syncTrack?.id ?? null,
      tracks: structuredClone(tracks),
      vst3Plugins: [],
      loadedPlugins: [],
    };
  };

  const plugin = { id: "uid-ad2", name: "Addictive Drums 2", path: "C:/VST3/AD2.vst3" };
  const hits = [
    { startMs: 500, pitch: 36, velocity: 120 },
    { startMs: 1000, pitch: 38, velocity: 105 },
  ];
  const first = await syncDrumVst3Track(fakeInvoke, { plugin, hits });
  assert.equal(first.trackId, 1);
  assert.equal(first.addedNotes, 2);
  assert.equal(tracks.length, 2, "user track with the same display name must survive");
  assert.equal(tracks[0].id, 99);
  assert.equal(tracks[1].isDrumSync, true);
  assert.deepEqual(tracks[1].notes, [
    { startMs: 500, endMs: 500 + DRUM_NOTE_MS, pitch: 36, velocity: 120 },
    { startMs: 1000, endMs: 1000 + DRUM_NOTE_MS, pitch: 38, velocity: 105 },
  ]);
  assert.ok(calls.some(c => c.cmd === "keysounds_action" && c.payload.kind === "sync_drum_track" && c.payload.pluginUid === "uid-ad2"));
  assert.ok(!calls.some(c => c.payload.kind === "delete_track"), "sync must not issue delete_track");

  // Re-syncing updates the existing drum-sync track in place instead of duplicating or deleting user tracks.
  const second = await syncDrumVst3Track(fakeInvoke, { plugin, hits: [{ startMs: 750, pitch: 42, velocity: 90 }] });
  assert.equal(second.trackId, 1);
  assert.equal(tracks.length, 2);
  assert.equal(tracks[1].notes.length, 1);
  assert.equal(tracks[1].notes[0].pitch, 42);

  // Previewing a drum row sends its GM note via keysounds_action preview_note.
  const previewPitch = await previewDrumVst3Hit(fakeInvoke, "Open Hi-Hat", 110);
  assert.equal(previewPitch, 46);
  const lastCall = calls.at(-1);
  assert.deepEqual(lastCall.payload, { kind: "preview_note", trackId: 0, pitch: 46, velocity: 110, durationMs: 200 });
});

test("DrumDetectWindow, Preferences and Drum Map Setup expose the Use VST3 sounds for drum map toggle", () => {
  const { body } = render(DrumDetectWindow);
  assert.match(body, /Use VST3 sounds for drum map/);
  assert.match(body, /aria-label="VST3 drum instrument"/);

  // Keysounds mode's own checkbox moved to Preferences › MIDI Input & VST3, on the same saved setting.
  const read = file => readFileSync(new URL(file, import.meta.url), "utf8");
  assert.doesNotMatch(read("../src/parts/KeysoundsModal.svelte") + read("../src/parts/keysounds/InspectorKeysounds.svelte"),
    /Use VST3 sounds for drum map/);
  const preferences = read("../src/parts/preferences/MidiVstTab.svelte");
  assert.match(preferences, /writeDrumVst3\(storage\(\), \{ \.\.\.readDrumVst3\(storage\(\)\), enabled: checked \}\)/);

  const lanes = readFileSync(new URL("../src/editor/lanes.js", import.meta.url), "utf8");
  assert.match(lanes, /Use VST3 sounds for drum map/);
  assert.match(lanes, /kind:\s*"list_installed"/);
  assert.match(lanes, /kind:\s*"browse"/);
  assert.match(lanes, /syncDrumVst3Track/);
});

test("LaneHeaders renders a clickable VST tag when vstRouted/vstBypassed is set and toggles per-lane routing", async () => {
  const baseLane = (more = {}) => ({
    type: "lane", key: "lane:0", row: 0, number: 1, name: "Normal", label: "normal-hitnormal",
    display: "normal-hitnormal", title: "normal-hitnormal", tag: "Normal · skin", icon: "normal",
    colour: null, folder: "normal", active: false, selected: false, muted: false, solo: false,
    quiet: false, layer: false, family: null, addLayer: null, remove: null, ...more,
  });

  // Unrouted lane has no VST tag; routed lane shows lit VST tag; bypassed lane shows dimmed VST tag.
  bridge.lanes = {
    names: true,
    busy: false,
    rows: [
      baseLane({ key: "lane:0", row: 0, vstRouted: true }),
      baseLane({ key: "lane:1", row: 1, name: "Clap", vstBypassed: true }),
      baseLane({ key: "lane:2", row: 2, name: "Kick" }),
    ],
  };
  const html = render(LaneHeaders).body;
  const rows = html.split(/(?=<div class="lane\b)/).filter(r => r.startsWith('<div class="lane'));
  assert.equal(rows.length, 3);
  assert.match(rows[0], /<button type="button" class="vst-tag\b[^"]*"[^>]*aria-pressed="true"[^>]*>VST<\/button>/);
  assert.doesNotMatch(rows[0], /class="vst-tag[^"]*\bbypassed\b/);
  assert.match(rows[1], /<button type="button" class="vst-tag[^"]*\bbypassed\b[^"]*"[^>]*aria-pressed="false"[^>]*>VST<\/button>/);
  assert.doesNotMatch(rows[2], /class="vst-tag/);

  // Compact mode (names hidden) omits the VST tag so M/S stay uncluttered.
  bridge.lanes = { names: false, busy: false, rows: [baseLane({ vstRouted: true })] };
  assert.doesNotMatch(render(LaneHeaders).body, /class="vst-tag/);

  // Handler forwards toggleVst(row) cleanly.
  const sent = [];
  const handlers = laneHandlers((...args) => sent.push(args), async () => {});
  handlers.toggleVst(0);
  assert.deepEqual(sent, [["toggleVst", 0]]);

  // End-to-end laneHeaderModel + laneHeaderAction("toggleVst", row) calls toggle_lane and refreshes scene.
  const calls = [];
  const laneNode = { style: { setProperty() {} }, dataset: {} };
  const state = {
    orderedLaneKeys: [], renamedLanes: new Map(), sampleFolderOrder: ["normal"],
    collapsedFolders: new Set(), laneColours: new Map(), collapsedFamilies: new Set(),
    selectionLaneKeys: new Set(), laneFilterText: "", usedLaneKeys: null,
    showLaneInfo: false, layoutWrites: 0, drag: null,
    view: { busy: false, activeLane: 0, panels: { laneNames: true }, drumView: null },
    scene: {
      path: "map.osu", rowHeight: 30, laneOrder: [0],
      lanes: [{
        key: "n", sourceKey: "n", name: "normal-hitnormal", family: "normal-hitnormal",
        group: "normal", index: 0, colourKey: "Skin|normal-hitnormal", hitCount: 2,
        vstRouted: true, vstBypassed: false,
      }],
    },
  };
  let laneHeadersModel = null;
  const ctx = {
    env: {
      window: {
        __TAURI__: {
          core: {
            invoke: async (cmd, args) => {
              calls.push({ cmd, args });
              if (cmd === "drum_vst_action" && args?.payload?.kind === "toggle_lane") {
                const l = state.scene.lanes[args.payload.row];
                l.vstRouted = !l.vstRouted;
                l.vstBypassed = !l.vstRouted;
              }
              return {};
            },
          },
        },
      },
      console,
    },
    state,
    dom: {
      $: id => (id === "lanes" ? laneNode : null),
      makeButton: () => ({}), input: () => ({}), labeled: () => ({}), paragraph: () => ({}),
    },
    run: task => task(),
    update: async (...args) => {
      calls.push({ update: args });
      ctx.renderLanes();
    },
    ui: () => ({ lanes: model => { laneHeadersModel = model; }, mixer: () => {} }),
    paintSoon: () => {},
    renderActiveLane: () => {},
  };
  attachLanes(ctx);
  ctx.renderLanes();
  assert.equal(laneHeadersModel.rows[1].vstRouted, true);
  assert.equal(laneHeadersModel.rows[1].vstBypassed, undefined);

  await ctx.laneHeaderAction("toggleVst", 0);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(calls.some(c => c.cmd === "drum_vst_action" && c.args.payload.kind === "toggle_lane" && c.args.payload.row === 0));
  assert.ok(calls.some(c => Array.isArray(c.update) && c.update[0] === null && c.update[1] === true));
  assert.equal(laneHeadersModel.rows[1].vstRouted, undefined);
  assert.equal(laneHeadersModel.rows[1].vstBypassed, true);

  bridge.lanes = laneHeadersModel;
  const afterHtml = render(LaneHeaders).body;
  assert.match(afterHtml, /<button type="button" class="vst-tag[^"]*\bbypassed\b[^"]*"[^>]*aria-pressed="false"[^>]*>VST<\/button>/);
});

test("Preferences shows the drum map VST3 setting an earlier version saved, and keeps its instrument", () => {
  const before = globalThis.localStorage;
  const checkbox = html => html.match(/<input[^>]*type="checkbox"[^>]*>(?=\s*<span[^>]*>Use VST3 sounds for drum map)/)?.[0];
  try {
    globalThis.localStorage = fakeStorage();
    assert.doesNotMatch(checkbox(render(MidiVstTab).body), /\schecked/);
    globalThis.localStorage = fakeStorage({
      [DRUM_VST3_KEY]: JSON.stringify({ enabled: true, pluginId: "uid-ad2", pluginName: "Addictive Drums 2", pluginPath: "C:/ad2.vst3" }),
    });
    const html = render(MidiVstTab).body;
    assert.match(checkbox(html), /\schecked/);
    assert.match(html, /Drum map rows play through Addictive Drums 2\./);
    // Switching it off writes only the flag: the instrument picked elsewhere stays.
    const storage = globalThis.localStorage;
    writeDrumVst3(storage, { ...readDrumVst3(storage), enabled: false });
    assert.deepEqual(readDrumVst3(storage),
      { enabled: false, pluginId: "uid-ad2", pluginName: "Addictive Drums 2", pluginPath: "C:/ad2.vst3" });
  } finally {
    globalThis.localStorage = before;
  }
});
