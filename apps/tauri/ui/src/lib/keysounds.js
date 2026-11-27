// Keysounds mode chrome: pure text and state helpers for the track list, the Inspector section and
// the toolbar pills. The mode's state itself lives in KeysoundsModal.svelte, which hands a plain
// model to bridge.keysounds.
import { octaveLabel } from "./musicalTyping.js";

const count = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

// The second line of a track row: "Vital · 128 notes".
export function trackCaption(track) {
  return `${track?.instrument || "No instrument"} · ${count(track?.noteCount ?? 0, "note")}`;
}

// What the Typing pill adds while it is on: "C4 · v100".
export function typingReadout(typing) {
  return `${octaveLabel(typing?.basePitch)} · v${typing?.velocity ?? 100}`;
}

// The instrument host's state for the active track, shown under the Inspector's instrument row.
export function keysoundsStatus(summary, track, instruments, noPlugins, sidecarText) {
  if (!summary) return { text: "", tone: "" };
  if (track?.isSoundfont) return { text: `SoundFont / SFZ ready · ${track.instrument}`, tone: "ready" };
  if (!instruments?.length) return { text: noPlugins, tone: "warning" };
  return { text: sidecarText, tone: summary.sidecarAvailable ? "ready" : "" };
}

// The Inspector's pinned footer: the chord row shows only while chords need resolving, and
// Generate needs at least one note.
export function generateFooter(model) {
  const chordCount = model?.chordCount ?? 0;
  return {
    chords: chordCount > 0 ? `${count(chordCount, "chord")} to resolve` : "",
    disabled: !model || !!model.loading || !model.anyNotes,
  };
}

// The selected note's index once the notes at `deleted` are gone (Rust keeps the rest in order),
// or null when it was one of them. Delete then still acts on the note that was selected.
export function selectionAfterDelete(selected, deleted) {
  if (selected == null || deleted.includes(selected)) return null;
  return selected - deleted.filter(index => index < selected).length;
}

// The roll's line for a track with no notes (plan revision 5, Q6). `drawKey` is the key bound to
// the Draw tool in the current preset: 3 on DAW, 2 on osu!.
export function emptyRollHint(trackName, drawKey) {
  const draw = drawKey ? `Draw (${drawKey.toUpperCase()})` : "Draw";
  return `No notes on ${trackName || "this track"} yet. Double-click in the roll, or pick ${draw}, to place one.`;
}
