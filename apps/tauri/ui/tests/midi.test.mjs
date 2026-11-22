import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  NO_PLUGINS, PORT_KEY, importArgs, importToast, instrumentChoices, keysoundsAdd, keysoundsDelete,
  keysoundsScan, keysoundsView, readPort, recordArgs, recordToast, scanSummary, writePort,
} from "../src/lib/midi.js";
import KeysoundsModal from "../src/parts/KeysoundsModal.svelte";
import MidiImportModal from "../src/parts/MidiImportModal.svelte";

const source = name => readFileSync(new URL(`../src/parts/${name}`, import.meta.url), "utf8");
const fakeStorage = (entries = {}) => {
  const data = new Map(Object.entries(entries));
  return {
    getItem: key => data.has(key) ? data.get(key) : null,
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: key => data.delete(key),
    data,
  };
};

test("the MIDI input port is kept in localStorage as a whole index, or none for the first port", () => {
  assert.equal(PORT_KEY, "dawsu.midiInputPort");
  const storage = fakeStorage();
  assert.equal(readPort(storage), null);
  writePort(storage, 2);
  assert.equal(storage.data.get(PORT_KEY), "2");
  assert.equal(readPort(storage), 2);
  writePort(storage, null);
  assert.equal(storage.data.has(PORT_KEY), false);
  for (const bad of ["-1", "1.5", "x", ""]) assert.equal(readPort(fakeStorage({ [PORT_KEY]: bad })), null, bad);
  assert.equal(readPort(null), null);
  assert.equal(readPort({ getItem() { throw new Error("denied"); } }), null);
});

test("Import sends apply_midi_import the picked tracks in order and the options as numbers and switches", () => {
  const info = { path: "E:/maps/song.mid" };
  assert.deepEqual(importArgs(info, [3, 0, 1], { useBeatmapTempo: false, offsetMs: "-12", snap: true, followVolume: 1 }), {
    path: "E:/maps/song.mid", tracks: [0, 1, 3], useBeatmapTempo: false, offsetMs: -12, snap: true, followVolume: true,
  });
  assert.deepEqual(importArgs(info, [0], { offsetMs: "abc" }), {
    path: "E:/maps/song.mid", tracks: [0], useBeatmapTempo: true, offsetMs: 0, snap: true, followVolume: false,
  });
});

test("the import toast counts the hits added and skipped and names Undo", () => {
  assert.deepEqual(importToast({ added: 12, skipped: 3 }),
    { tone: "success", title: "MIDI imported", body: "Added 12 hits (3 skipped). Press Ctrl+Z to undo." });
  assert.equal(importToast({ added: 1, skipped: 0 }).body, "Added 1 hit. Press Ctrl+Z to undo.");
  assert.equal(importToast(null).body, "Added 0 hits. Press Ctrl+Z to undo.");
});

test("MIDI Record sends the picked port and the snap's beat divisor", () => {
  assert.deepEqual(recordArgs(1, { beats: 1, divisor: 4 }), { portIndex: 1, snapDivisor: 4 });
  assert.deepEqual(recordArgs(null, { divisor: 12 }), { portIndex: null, snapDivisor: 12 });
  assert.deepEqual(recordArgs(0, { beats: 2, divisor: 1 }), { portIndex: 0, snapDivisor: null }, "2/1 has no beat divisor");
  assert.deepEqual(recordArgs(undefined, null), { portIndex: null, snapDivisor: null });
});

test("MIDI Record's toast: armed names the port, a commit counts the hits, an empty take says so", () => {
  assert.deepEqual(recordToast({ armed: true, portName: "TD-17", bufferedNotes: 0, committedNotes: 0 }), {
    tone: "info", title: "MIDI recording armed", body: "Listening on TD-17. Press Space to play and record.",
  });
  assert.equal(recordToast({ armed: true, portName: null }).body, "No MIDI input port found.");
  assert.deepEqual(recordToast({ armed: false, portName: "TD-17", committedNotes: 5 }),
    { tone: "success", title: "MIDI recorded", body: "Added 5 hits. Press Ctrl+Z to undo." });
  assert.deepEqual(recordToast({ armed: false, committedNotes: 0 }),
    { tone: "info", title: "MIDI recording stopped", body: "No notes were recorded." });
  assert.equal(recordToast(null), null);
});

test("the VST3 scan says plainly when no plug-ins are installed", () => {
  assert.equal(scanSummary({ plugins: [], skipped: [] }), NO_PLUGINS);
  assert.match(NO_PLUGINS, /^No VST3 plug-ins found\./);
  assert.equal(scanSummary(null), NO_PLUGINS);
  assert.equal(scanSummary({ plugins: [], skipped: [{ path: "a.vst3", reason: "bad" }] }), NO_PLUGINS + " 1 file could not be read.");
  const plugins = [
    { uid: "a", name: "Drums", vendor: "X", is_instrument: true, is_effect: false },
    { uid: "b", name: "Reverb", vendor: "Y", is_instrument: false, is_effect: true },
  ];
  assert.equal(scanSummary({ plugins, skipped: [] }), "Found 2 plug-ins (1 instrument).");
  assert.equal(scanSummary({ plugins, skipped: [{}, {}] }), "Found 2 plug-ins (1 instrument), 2 skipped.");
});

test("a keysound track may use only scanned instruments, sorted by name", () => {
  const plugins = [
    { id: "2", name: "Zebra", vendor: "u-he", isInstrument: true },
    { id: "3", name: "Reverb", vendor: "Y", isInstrument: false },
    { id: "1", name: "Addictive Drums", vendor: "XLN", isInstrument: true },
  ];
  assert.deepEqual(instrumentChoices(plugins), [
    { id: "1", name: "Addictive Drums", vendor: "XLN" },
    { id: "2", name: "Zebra", vendor: "u-he" },
  ]);
  assert.deepEqual(instrumentChoices(undefined), []);
});

test("keysounds_action payloads match main.rs's kinds", () => {
  assert.deepEqual(keysoundsView(), { kind: "view" });
  assert.deepEqual(keysoundsScan(), { kind: "scan_vst3" });
  assert.deepEqual(keysoundsAdd("  Piano  ", "Zebra"), { kind: "add_track", name: "Piano", vst3Plugin: "Zebra" });
  assert.deepEqual(keysoundsAdd("", null), { kind: "add_track", name: "Keysound Track" });
  assert.deepEqual(keysoundsDelete(7), { kind: "delete_track", trackId: 7 });
});

test("both modals start closed and register their opener for menus.js", () => {
  bridge.set({ busy: false }, { path: "Hard.osu" });
  assert.equal(render(MidiImportModal).body.replace(/<!--[^>]*-->/g, "").trim(), "");
  assert.equal(render(KeysoundsModal).body.replace(/<!--[^>]*-->/g, "").trim(), "");
  assert.match(source("MidiImportModal.svelte"), /globalThis\.openMidiImport = openMidiImport/);
  assert.match(source("KeysoundsModal.svelte"), /globalThis\.openKeysounds = openKeysounds/);
  // Both hold the timeline's keys while open, as Preferences does.
  for (const name of ["MidiImportModal.svelte", "KeysoundsModal.svelte"]) {
    assert.match(source(name), /globalThis\.dawsuModalOpen = open/, name);
  }
});

test("Keysounds shows the no-plug-ins message, Preferences lists MIDI ports and scans VST3", () => {
  const keysounds = source("KeysoundsModal.svelte");
  assert.match(keysounds, /invoke\("keysounds_action", \{ payload \}\)/);
  assert.match(keysounds, /\{NO_PLUGINS\}/);
  const preferences = source("PreferencesModal.svelte");
  assert.match(preferences, /\{ id: "midi", label: "MIDI Input & VST3" \}/);
  assert.match(preferences, /invoke\("midi_input_ports"\)/);
  assert.match(preferences, /invoke\("vst3_scan"\)/);
  assert.match(preferences, /Scan VST3 plug-ins/);
  assert.match(source("TransportBar.svelte"), /invoke\("midi_record_toggle", recordArgs\(/);
});
