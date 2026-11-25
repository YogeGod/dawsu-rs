// Pure helpers for the osu!standard Chord & Polyphony Resolver (no storyboard .osb option).

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export const CHORD_STRATEGIES = Object.freeze([
  {
    id: "flatten_polyphonic",
    label: "Flatten Chord to 1 Polyphonic Sample (Recommended)",
    description: "Renders all chord notes together into one reusable sample on the hit circle.",
  },
  {
    id: "top_note_only",
    label: "Top Note Only",
    description: "Keeps the highest melody note on the hit circle and mutes lower voices.",
  },
]);

export function formatChordPitches(pitches = []) {
  return [...pitches]
    .sort((a, b) => a - b)
    .map(p => `${NOTE_NAMES[p % 12]}${Math.floor(p / 12) - 1}`)
    .join(" + ");
}
