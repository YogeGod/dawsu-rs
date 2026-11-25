// Pure helpers for the modal Drum Map Editor (MIDI note labels, preset groups, choke badges).

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export const CHOKE_GROUP_OPTIONS = [
  { id: null, label: "None" },
  { id: 1, label: "A" },
  { id: 2, label: "B" },
];

export function formatMidiNoteLabel(note) {
  if (!Number.isInteger(note) || note < 0 || note > 127) return "Unassigned";
  const octave = Math.floor(note / 12) - 2;
  return `${NOTE_NAMES[note % 12]}${octave} (${note})`;
}

export function formatChokeGroup(group) {
  if (group === 1) return "A";
  if (group === 2) return "B";
  return "None";
}

export function groupDrumPresets(presets = []) {
  const free = [];
  const pro = [];
  for (const preset of presets) {
    if (/ggd|garstka|invasion|superior|sd3/i.test(preset?.name ?? "")) pro.push(preset);
    else free.push(preset);
  }
  return { free, pro };
}
