import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  NO_PLUGINS, PORT_KEY, PX_PER_MS, QUANTIZE_MS, ROLL_HIGH, ROLL_LOW, ROW_PX, SCALE_DEFS, SCALE_ROOTS,
  findNoteIndex, generateToast, importArgs, importToast, instrumentChoices, isBlackKey, isPitchInScale,
  keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsDeleteNote, keysoundsGenerate, keysoundsHumanise,
  keysoundsLoadVst3, keysoundsMoveNote, keysoundsPreview, keysoundsQuantize, keysoundsResizeNote,
  keysoundsScan, keysoundsSetVelocity, keysoundsStrum, keysoundsSyncToObjects, keysoundsToggleMute,
  keysoundsToggleSolo, keysoundsTranspose, keysoundsTrimOverlaps, keysoundsVelocityRamp, keysoundsView,
  msAtX, noteDragPayload, pitchAtY, pitchName, pitchRow, readPort, recordArgs, recordToast, rollPitches,
  rollSpanMs, scalePitchMask, scanSummary, sidecarStatus, velocityAtY, visibleRollPitches, writePort,
} from "../src/lib/midi.js";
import { sampleVelocityRamp } from "../src/lib/pianoRoll.js";
import {
  createMusicalTypingState, handleMusicalTypingKeyDown, handleMusicalTypingKeyUp, octaveLabel, pitchForKeyCode,
} from "../src/lib/musicalTyping.js";
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
  assert.deepEqual(recordArgs(1, { snap: false, beats: 1, divisor: 4 }), { portIndex: 1, snapDivisor: null }, "snap off disables MIDI record quantization");
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
  assert.match(source("MidiImportModal.svelte"), /bridge\.registerModal\("openMidiImport", openMidiImport\)/);
  assert.match(source("KeysoundsModal.svelte"), /bridge\.registerModal\("openKeysounds", openKeysounds\)/);
  // Both hold the timeline's keys while open, as Preferences does.
  assert.match(source("MidiImportModal.svelte"), /bridge\.setModalOpen\("midi-import", open\)/);
  assert.match(source("KeysoundsModal.svelte"), /bridge\.setModalOpen\("keysounds", open\)/);
});

test("Keysounds shows the no-plug-ins message, Preferences lists MIDI ports and scans VST3", () => {
  const keysounds = source("KeysoundsModal.svelte");
  assert.match(keysounds, /invoke\("keysounds_action", \{ payload \}\)/);
  assert.match(keysounds, /\{NO_PLUGINS\}/);
  const preferences = source("PreferencesModal.svelte") + "\n" + source("preferences/MidiVstTab.svelte");
  assert.match(preferences, /\{ id: "midi", label: "MIDI Input & VST3" \}/);
  assert.match(preferences, /invoke\("midi_action", \{ action: \{ kind: "input_ports" \} \}\)/);
  assert.match(preferences, /invoke\("vst3_scan"\)/);
  assert.match(preferences, /invoke\("vst3_scan",\s*\{\s*retryBlocked:\s*true\s*\}\)/);
  assert.match(preferences, /Retry blocked plug-ins/);
  assert.match(preferences, /Scan VST3 plug-ins/);
  assert.match(source("TransportBar.svelte"), /invoke\("midi_action", \{[\s\S]*?kind: "record_toggle"[\s\S]*?recordArgs\(/);
});

test("piano roll note payloads match keysound_session.rs and send whole numbers", () => {
  assert.deepEqual(keysoundsAddNote(3, 1000.4, 60.6),
    { kind: "add_note", trackId: 3, startMs: 1000, endMs: 1250, pitch: 61, velocity: 100 });
  assert.deepEqual(keysoundsAddNote(3, -5, 200, 0, 100),
    { kind: "add_note", trackId: 3, startMs: 0, endMs: 100, pitch: 127, velocity: 1 });
  assert.deepEqual(keysoundsMoveNote(3, 2, 1500, 64), { kind: "move_note", trackId: 3, noteIndex: 2, startMs: 1500, pitch: 64 });
  assert.deepEqual(keysoundsResizeNote(3, 2, 1875.6), { kind: "resize_note", trackId: 3, noteIndex: 2, endMs: 1876 });
  assert.deepEqual(keysoundsDeleteNote(3, 0), { kind: "delete_note", trackId: 3, noteIndex: 0 });
  assert.deepEqual(keysoundsSetVelocity(3, 1, 140), { kind: "set_velocity", trackId: 3, noteIndex: 1, velocity: 127 });
  assert.deepEqual(keysoundsQuantize(3), { kind: "quantize_notes", trackId: 3, stepMs: 125 });
  assert.deepEqual(keysoundsTranspose(3, -12), { kind: "transpose_notes", trackId: 3, semitones: -12 });
  assert.deepEqual(keysoundsPreview(3, 60), { kind: "preview_note", trackId: 3, pitch: 60, velocity: 100, durationMs: 200 });
  assert.deepEqual(keysoundsGenerate(), { kind: "generate_keysounds" });
  assert.deepEqual(keysoundsToggleMute(3), { kind: "toggle_mute", trackId: 3 });
  assert.deepEqual(keysoundsToggleSolo(3), { kind: "toggle_solo", trackId: 3 });
  assert.deepEqual(keysoundsSyncToObjects(3), { kind: "sync_to_objects", trackId: 3, maxShiftMs: 50 });
  assert.deepEqual(keysoundsSyncToObjects(3, 75.4), { kind: "sync_to_objects", trackId: 3, maxShiftMs: 75 });
  assert.deepEqual(keysoundsTrimOverlaps(3), { kind: "trim_overlaps", trackId: 3 });
});

test("musicalTyping maps QWERTY keys to pitches, clamps octave/velocity, and deduplicates held keys", () => {
  let state = { ...createMusicalTypingState(), enabled: true };
  const held = new Set();
  assert.equal(octaveLabel(state.basePitch), "C4");
  assert.equal(pitchForKeyCode("KeyA", 60), 60);
  assert.equal(pitchForKeyCode("KeyW", 60), 61);
  assert.equal(pitchForKeyCode("KeyK", 60), 72);
  assert.equal(pitchForKeyCode("Quote", 60), 77);
  assert.equal(pitchForKeyCode("Digit1", 60), null);

  const first = handleMusicalTypingKeyDown(state, held, { code: "KeyA", key: "a", repeat: false });
  assert.equal(first.action, "note");
  assert.equal(first.pitch, 60);
  assert.equal(first.velocity, 100);

  // Held key repeat is ignored until keyup.
  const repeat = handleMusicalTypingKeyDown(state, held, { code: "KeyA", key: "a", repeat: true });
  assert.equal(repeat.action, "repeat");
  handleMusicalTypingKeyUp(held, "KeyA");
  const again = handleMusicalTypingKeyDown(state, held, { code: "KeyA", key: "a", repeat: false });
  assert.equal(again.action, "note");

  // Z/X shift octave and clamp to C1..C7; C/V shift velocity and clamp to 1..127.
  for (let i = 0; i < 6; i++) state = handleMusicalTypingKeyDown(state, held, { code: "KeyZ", key: "z" }).state;
  assert.equal(state.basePitch, 24);
  for (let i = 0; i < 10; i++) state = handleMusicalTypingKeyDown(state, held, { code: "KeyX", key: "x" }).state;
  assert.equal(state.basePitch, 96);
  for (let i = 0; i < 20; i++) state = handleMusicalTypingKeyDown(state, held, { code: "KeyC", key: "c" }).state;
  assert.equal(state.velocity, 1);
  for (let i = 0; i < 20; i++) state = handleMusicalTypingKeyDown(state, held, { code: "KeyV", key: "v" }).state;
  assert.equal(state.velocity, 127);

  // Escape turns musical typing off and clears held keys.
  const off = handleMusicalTypingKeyDown(state, held, { code: "Escape", key: "Escape" });
  assert.equal(off.action, "off");
  assert.equal(off.state.enabled, false);
  assert.equal(held.size, 0);
});

test("Load VST3 sends the scanned plug-in's file and id, or nothing without a track or file", () => {
  const plugin = { id: "UID1", name: "Zebra", vendor: "u-he", path: "C:/VST3/Zebra.vst3", isInstrument: true };
  assert.deepEqual(keysoundsLoadVst3(4, plugin),
    { kind: "load_vst3", trackId: 4, pluginPath: "C:/VST3/Zebra.vst3", pluginUid: "UID1", pluginName: "Zebra" });
  assert.equal(keysoundsLoadVst3(null, plugin), null);
  assert.equal(keysoundsLoadVst3(4, undefined), null);
  assert.equal(sidecarStatus({ sidecarAvailable: false, loadedPlugins: [] }), "VST3 host not found: notes preview with the built-in tone.");
  assert.equal(sidecarStatus({ sidecarAvailable: true, loadedPlugins: [{ id: "a" }] }), "VST3 host ready · 1 instrument loaded.");
  assert.equal(sidecarStatus({ sidecarAvailable: true, loadedPlugins: [] }), "VST3 host ready · no instrument loaded.");
});

test("the roll runs C2 to C6, one row per semitone, with C4 as MIDI 60", () => {
  assert.equal(ROLL_LOW, 36);
  assert.equal(ROLL_HIGH, 84);
  const pitches = rollPitches();
  assert.equal(pitches.length, 49);
  assert.equal(pitchName(pitches[0]), "C6");
  assert.equal(pitchName(pitches.at(-1)), "C2");
  assert.equal(pitchName(60), "C4");
  assert.equal(pitchName(61), "C#4");
  assert.equal(isBlackKey(61), true);
  assert.equal(isBlackKey(64), false);
  assert.equal(pitchRow(84), 0);
  assert.equal(pitchAtY(pitchRow(60) + ROW_PX / 2), 60);
  assert.equal(pitchAtY(-50), 84);
  assert.equal(pitchAtY(10_000), 36);
});

test("a click on an empty cell places a note on the 125 ms grid at that pitch", () => {
  const x = 1060 * PX_PER_MS;
  const y = pitchRow(67) + 3;
  assert.equal(msAtX(x), 1000);
  assert.equal(msAtX(-10), 0);
  assert.deepEqual(keysoundsAddNote(9, msAtX(x), pitchAtY(y)),
    { kind: "add_note", trackId: 9, startMs: 1000, endMs: 1250, pitch: 67, velocity: 100 });
  assert.equal(rollSpanMs([]), 8000);
  assert.equal(rollSpanMs([{ endMs: 9000 }]), 11000);
});

test("dragging a note body moves it, dragging its right edge resizes it, and a still click sends nothing", () => {
  const note = { index: 2, startMs: 1000, endMs: 1250, pitch: 60, velocity: 90 };
  assert.deepEqual(noteDragPayload(5, note, "move", 500 * PX_PER_MS, -2 * ROW_PX),
    { kind: "move_note", trackId: 5, noteIndex: 2, startMs: 1500, pitch: 62 });
  assert.deepEqual(noteDragPayload(5, note, "resize", 250 * PX_PER_MS, 40),
    { kind: "resize_note", trackId: 5, noteIndex: 2, endMs: 1500 });
  assert.deepEqual(noteDragPayload(5, note, "resize", -1000 * PX_PER_MS, 0),
    { kind: "resize_note", trackId: 5, noteIndex: 2, endMs: 1125 }, "a note keeps one grid step");
  assert.equal(noteDragPayload(5, note, "move", 2, 3), null);
  assert.equal(noteDragPayload(5, note, "resize", 1, 0), null);
  // Zero-distance click on an off-grid note must NOT quantize or mutate its timing.
  const offGrid = { index: 3, startMs: 1037, endMs: 1287, pitch: 60, velocity: 90 };
  assert.equal(noteDragPayload(5, offGrid, "move", 0, 0), null);
  assert.equal(noteDragPayload(5, offGrid, "resize", 0, 0), null);
  // Rust re-sorts the track after a move; the selection follows the note by start and pitch.
  const sorted = [{ index: 0, startMs: 1200, pitch: 60 }, { index: 1, startMs: 1500, pitch: 62 }];
  assert.equal(findNoteIndex(sorted, 1500, 62), 1);
  assert.equal(findNoteIndex(sorted, 1500, 61), null);
});

test("the velocity lane maps its top to 127 and its foot to 1", () => {
  assert.equal(velocityAtY(0, 64), 127);
  assert.equal(velocityAtY(64, 64), 1);
  assert.equal(velocityAtY(32, 64), 64);
  assert.deepEqual(keysoundsSetVelocity(1, 0, velocityAtY(16, 64)), { kind: "set_velocity", trackId: 1, noteIndex: 0, velocity: 95 });
});

test("Generate [Keysounds] toasts the written file and the synced count", () => {
  assert.deepEqual(generateToast({ fileName: "Song (Keysounds).osu", syncedCount: 12, unsyncedCount: 0, manifestCount: 5 }),
    { tone: "success", title: "[Keysounds] generated", body: "Wrote Song (Keysounds).osu with 12 synced notes." });
  assert.equal(generateToast({ fileName: "a.osu", syncedCount: 1, unsyncedCount: 2 }).body,
    "Wrote a.osu with 1 synced note (2 not on an object).");
  assert.equal(generateToast(undefined), null);
});

test("the Keysounds modal wires every piano roll gesture to its keysounds_action", () => {
  const keysounds = [
    "KeysoundsModal.svelte",
    "keysounds/KeysoundTrackBar.svelte",
    "keysounds/KeysoundPianoRoll.svelte",
    "keysounds/KeysoundVelocityLane.svelte",
  ].map(source).join("\n");
  for (const wiring of [
    /send\(keysoundsAddNote\(trackId, startMs, pitch\)\)[\s\S]*?preview\(pitch\)/, // empty cell: add then play
    /noteDragPayload\(trackId, note, mode, dx, dy, QUANTIZE_MS, pitches\)/, // pointerup on a dragged body or edge
    /onNoteDown\(e, note, "move"\)/,
    /onNoteDown\(e, note, "resize"\)/,
    /oncontextmenu=\{\(e\) => \{ e\.preventDefault\(\); e\.stopPropagation\(\); deleteNote\(note\.index\)/,
    /event\.key === "Delete" \|\| event\.key === "Backspace"/,
    /send\(keysoundsSetVelocity\(trackId, index, velocity\)\)/,
    /onpointerdown=\{\(\) => preview\(pitch\)\}/, // the keyboard plays
    /keysoundsQuantize\(id, QUANTIZE_MS\)/,
    /keysoundsTranspose\(id, -12\)/, /keysoundsTranspose\(id, -1\)/, /keysoundsTranspose\(id, 1\)/, /keysoundsTranspose\(id, 12\)/,
    /send\(keysoundsLoadVst3\(trackId, plugin\)\)/,
    /payload: keysoundsGenerate\(\)[\s\S]*?showToast\(toast\)/,
    /Generate \[Keysounds\]/,
    /keysoundsToggleMute\(id\)/,
    /keysoundsToggleSolo\(id\)/,
    /keysoundsTrimOverlaps\(id\)/,
    /keysoundsSyncToObjects\(trackId\)/,
    /class="ks-workspace"/,
    /class="ks-ghost-note"/,
    /class="ks-object-line"/,
  ]) assert.match(keysounds, wiring);
  // Inline workspace must not wrap the piano roll in a floating modal backdrop.
  assert.doesNotMatch(source("KeysoundsModal.svelte"), /class="ks-backdrop"/);
  // A press that lands on a note must not fall through to the grid and add another.
  assert.match(keysounds, /event\.target !== event\.currentTarget/);
  // Previews skip send() so the bar does not flicker; preview_note's summary is not applied.
  assert.match(keysounds, /invoke\("keysounds_action", \{ payload: keysoundsPreview\(/);
});

test("scale helpers, fold-to-scale pitch mapping, strum/humanise/velocity ramp payloads and layout fixes match contracts", () => {
  assert.equal(SCALE_ROOTS.length, 12);
  assert.ok(SCALE_DEFS.some(s => s.id === "major"));
  assert.equal(scalePitchMask(0, "chromatic"), 0xfff);
  assert.equal(isPitchInScale(60, 0, "major"), true);
  assert.equal(isPitchInScale(61, 0, "major"), false);

  const chromatic = visibleRollPitches(0, "chromatic", true);
  assert.equal(chromatic.length, 49);
  const cMajor = visibleRollPitches(0, "major", true);
  assert.equal(cMajor.length, 29, "C2..=C6 contains 29 C-major pitches");
  assert.equal(cMajor.includes(61), false);

  // Folded coordinates map strictly across scale rows (never landing on out-of-scale pitches).
  const rowC4 = pitchRow(60, cMajor);
  assert.ok(rowC4 >= 0);
  assert.equal(pitchAtY(rowC4 - ROW_PX + 2, cMajor), 62, "one row up from C4 lands on D4 in C major");
  const e4 = { index: 0, startMs: 1000, endMs: 1250, pitch: 64, velocity: 90 };
  assert.deepEqual(
    noteDragPayload(3, e4, "move", 0, -ROW_PX, QUANTIZE_MS, cMajor),
    { kind: "move_note", trackId: 3, noteIndex: 0, startMs: 1000, pitch: 65 },
  );

  // Strum, humanise, velocity ramp payloads & live curve preview mirror Rust.
  assert.deepEqual(keysoundsStrum(4), { kind: "strum_notes", trackId: 4, spreadMs: 20, direction: "up", chordToleranceMs: 15 });
  assert.deepEqual(keysoundsHumanise(4, 12, 8, 99), { kind: "humanise_notes", trackId: 4, timingMs: 12, velocityRange: 8, seed: 99 });
  assert.deepEqual(keysoundsVelocityRamp(4, 100, 900, 30, 120, 1), {
    kind: "velocity_ramp", trackId: 4, startMs: 100, endMs: 900, startVelocity: 30, endVelocity: 120, curve: 1,
  });
  assert.equal(sampleVelocityRamp(500, 0, 1000, 20, 120, 0), 70);
  assert.equal(sampleVelocityRamp(500, 0, 1000, 20, 120, 1), 45);
  assert.equal(sampleVelocityRamp(500, 1000, 0, 120, 20, 0), 70);
  assert.equal(sampleVelocityRamp(1500, 0, 1000, 20, 120, 0), null);

  // Inline 1b21edb structural defect fixes:
  const timelineCss = readFileSync(new URL("../src/timeline.css", import.meta.url), "utf8");
  assert.match(timelineCss, /#editor-arrange\.keysounds-active[\s\S]*?grid-template-rows:\s*auto minmax\(0,\s*1fr\);/);
  assert.match(timelineCss, /#editor-arrange\.keysounds-active\s*>\s*#base-diff-strip\s*\{\s*grid-row:\s*1;/);
  assert.match(timelineCss, /#editor-arrange\.keysounds-active\s*>\s*#keysounds-workspace\s*\{\s*grid-row:\s*2;/);
  assert.match(timelineCss, /\.arrange\.keysounds-active\s*>\s*\.ruler/);

  const modalSrc = source("KeysoundsModal.svelte");
  assert.doesNotMatch(modalSrc, /bridge\.playhead/, "KeysoundsModal must use bridge.currentPositionMs(), not reactive bridge.playhead");
  assert.match(modalSrc, /Boolean\(m\[2\]\)/);
  assert.match(modalSrc, /tapRecordingArmed/);
  assert.match(modalSrc, /bridge\.currentPositionMs\(\)/);
  assert.match(modalSrc, /bridge\.callAction\("command",\s*"play"\)/);
  assert.match(modalSrc, /send\(\{\s*kind:\s*"undo"\s*\}\)/);
  assert.match(modalSrc, /send\(\{\s*kind:\s*"redo"\s*\}\)/);
  assert.match(source("keysounds/KeysoundPianoRoll.svelte"), /\.ks-beat\.sustained/);
  assert.match(source("keysounds/KeysoundVelocityLane.svelte"), /onvelocityramp/);
});
