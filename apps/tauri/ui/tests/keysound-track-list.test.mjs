import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { generateFooter, trackCaption, typingReadout } from "../src/lib/keysounds.js";
import { keysoundsRenameTrack, keysoundsSnapToGrid } from "../src/lib/midi.js";
import KeysoundTrackList from "../src/parts/keysounds/KeysoundTrackList.svelte";

const tracks = [
  { id: 4, name: "Lead", instrument: "Vital", noteCount: 128, muted: false, solo: false },
  { id: 7, name: "Bass", instrument: "", noteCount: 1, muted: true, solo: false },
];
const shown = (props = {}) => render(KeysoundTrackList, { props: { tracks, activeTrackId: 4, ...props } }).body;
const source = readFileSync(new URL("../src/parts/keysounds/KeysoundTrackList.svelte", import.meta.url), "utf8");

test("a track's caption names its instrument and its note count", () => {
  assert.equal(trackCaption(tracks[0]), "Vital · 128 notes");
  assert.equal(trackCaption(tracks[1]), "No instrument · 1 note");
  assert.equal(trackCaption(null), "No instrument · 0 notes");
});

test("the Typing pill reads its octave and velocity; Generate's footer shows chords only while some need resolving", () => {
  assert.equal(typingReadout({ enabled: true, basePitch: 60, velocity: 100 }), "C4 · v100");
  assert.deepEqual(generateFooter({ chordCount: 3, anyNotes: true, loading: false }),
    { chords: "3 chords to resolve", disabled: false });
  assert.deepEqual(generateFooter({ chordCount: 1, anyNotes: true, loading: false }),
    { chords: "1 chord to resolve", disabled: false });
  assert.deepEqual(generateFooter({ chordCount: 0, anyNotes: true, loading: false }), { chords: "", disabled: false });
  assert.deepEqual(generateFooter({ chordCount: 0, anyNotes: false, loading: false }), { chords: "", disabled: true });
  assert.deepEqual(generateFooter({ chordCount: 0, anyNotes: true, loading: true }), { chords: "", disabled: true });
  assert.deepEqual(generateFooter(null), { chords: "", disabled: true });
});

test("rename and snap-to-grid payloads match keysound_session.rs", () => {
  assert.deepEqual(keysoundsRenameTrack(4, "  Pad  "), { kind: "rename_track", trackId: 4, name: "Pad" });
  assert.equal(keysoundsRenameTrack(4, "   "), null);
  assert.equal(keysoundsRenameTrack(null, "Pad"), null);
  assert.deepEqual(keysoundsSnapToGrid(4), { kind: "snap_notes_to_grid", trackId: 4 });
});

test("the track list shows one two-line row per track, the active one marked, with Mute and Solo", () => {
  const html = shown();
  assert.equal([...html.matchAll(/class="ks-track(?: [^"]*)?" /g)].length, 2);
  assert.match(html, /class="ks-track [^"]*active"[\s\S]*?Lead[\s\S]*?Vital · 128 notes/);
  assert.match(html, /aria-current="true"[^>]*>[\s\S]*?Lead/);
  assert.match(html, /Bass[\s\S]*?No instrument · 1 note/);
  assert.match(html, /aria-label="Mute Lead"[^>]*aria-pressed="false"/);
  assert.match(html, /aria-label="Mute Bass"[^>]*aria-pressed="true"/);
  assert.match(html, /aria-label="Solo Bass"[^>]*aria-pressed="false"/);
  // The last two rows add a track and import a MIDI file.
  assert.match(html, /aria-label="Add track"[\s\S]*aria-label="Import \.mid…"/);
  assert.doesNotMatch(html, /No keysound tracks yet/);
});

test("with no tracks the list explains the next step above the two add rows", () => {
  const html = shown({ tracks: [], activeTrackId: null });
  assert.match(html, /role="status"[^>]*>[\s\S]*No keysound tracks yet\. Add a track below, then click in the roll to place notes\./);
  assert.match(html, /aria-label="Add track"/);
  assert.match(html, /aria-label="Import \.mid…"/);
});

test("a busy list takes no input", () => {
  const html = shown({ loading: true });
  for (const label of ["Mute Lead", "Solo Lead", "Add track", "Import \\.mid…"]) {
    assert.match(html, new RegExp(`aria-label="${label}"[^>]*\\sdisabled`), label);
  }
});

test("rows select on click, rename on double-click or F2, and open Rename and Delete on right-click", () => {
  for (const wiring of [
    /onclick=\{\(\) => onselect\?\.\(track\.id\)\}/,
    /ondblclick=\{\(\) => startRename\(track\)\}/,
    /event\.key === "F2"/,
    /oncontextmenu=\{event => openMenu\(event, track\)\}/,
    /onmute\?\.\(track\.id\)/,
    /onsolo\?\.\(track\.id\)/,
    /onrename\?\.\(id, name\)/,
    /ondelete\?\.\(menu\.track\.id\)/,
    /role="menu"/,
    /Delete track/,
  ]) assert.match(source, wiring);
  // A double-click's first click makes the host busy; the name field still opens.
  assert.doesNotMatch(source.match(/function startRename[^]*?\n  \}/)[0], /loading/);
  // The menu leaves the roll's scroll box, which contains fixed boxes, for the page.
  assert.match(source, /document\.body\.append\(node\)/);
  // A narrow lane column keeps each track as its number with M over S.
  assert.match(source, /container-type:\s*inline-size/);
  assert.match(source, /@container \(max-width: 130px\)/);
});
