// Pure constants and stateless helpers shared across the modular editor runtime.
// No DOM or Svelte imports, so apps/tauri/tests can run directly in Node without node_modules.

export const rulerBandHeight = 40;
// v3 §4: the Song · objects row is 84 px.
export const defaultWaveformHeight = 84;
export const bookmarkFlagHeight = 22;
export const bookmarkHalfWidth = 8;
export const hitHalfWidth = 10;
export const bookmarkJumpWait = 1000;
export const browserRowLimit = 500;
export const minZoom = 0.0005;
export const locatorFlagWidth = 14;
export const locatorFlagHeight = 13;
export const objectSize = 34;
export const objectReach = objectSize / 2;
export const objectWhite = "#e6eef2";
export const overviewColumns = 300;

export const sampleFolderNames = { normal: "Normal set", soft: "Soft set", drum: "Drum set" };

export const playbackStartChoices = [
  ["cursor", "Cursor"],
  ["left", "Left locator"],
  ["selection", "Selection"],
  ["selection_or_left", "Selection or left locator"],
];

export const panelSwitches = {
  browser: "view-browser",
  inspector: "view-inspector",
  controller: "view-controller",
};

// A folded panel's rail icons open it at their section.
export const railSections = {
  "rail-browser-browse": ["browser", "browse"],
  "rail-browser-search": ["browser", "search"],
  "rail-browser-library": ["browser", "library"],
  "rail-inspector-selection": ["inspector", "Selection"],
  "rail-inspector-lane": ["inspector", "Lane"],
  "rail-inspector-difficulty": ["inspector", "Difficulty"],
  "rail-inspector-bookmarks": ["inspector", "Bookmarks"],
};

// Canvas surfaces match the Logic Pro for iPad cool slate chrome.
export const slateTimeline = {
  arrange: "#0f1218", alternateRow: "#161a23", activeRow: "#2a3548", group: "#1f2430",
  ruler: "#0b0e14", rulerBar: "#f2f5fa", rulerEdge: "#343e52", overview: "#161a23", waveform: "#132229",
  volume: "#0f1218", divider: "#2c3445", beatLine: "#2c3445", barLine: "#46536b",
  cycle: "#f5b841", loop: "#4fa8ff14", locator: "#7b8ba3", playhead: "#84c5ff",
};

export const levelColours = [[58, 108, 240], [154, 72, 216], [236, 58, 58]];

export const volumePalette = Array.from({ length: 101 }, (_, volume) => {
  const index = volume < 50 ? 0 : 1;
  const mix = (volume - index * 50) / 50;
  const channels = levelColours[index].map((channel, i) =>
    Math.trunc(channel + (levelColours[index + 1][i] - channel) * mix));
  return "rgb(" + channels.join(", ") + ")";
});

// The colour a lane has until the mapper picks one: v3's lane lightness with no chroma, the
// bundle's laneGrey (ui/src/lib/colour.js). Hit diamonds keep the volume colours (Yoge, 2026-11-19).
export const laneGrey = "#9e9e9e";

export const laneSounds = {
  hitnormal: ["Normal", "normal"], hitwhistle: ["Whistle", "whistle"], hitfinish: ["Finish", "finish"],
  hitclap: ["Clap", "clap"], sliderslide: ["Slide", "slide"], sliderwhistle: ["Slide whistle", "whistle"],
  slidertick: ["Tick", "tick"],
};

// Divisor palette follows osu!lazer BindableBeatDivisor.cs and OsuColour.cs at
// 131fe72a15581049401a588a13684938d4d0e0c7. Copyright (c) 2025 ppy Pty Ltd.
// MIT licence, see THIRD-PARTY-NOTICES.md.
export const snapColours = {
  1: "#ffffff", 2: "#ed1121", 3: "#8866ee", 4: "#66ccff",
  5: "#b3d944", 6: "#eeaa00", 7: "#b3d944", 8: "#ffcc22",
  9: "#b3d944", 12: "#cc6600", 16: "#6644cc",
};

// v3 §5 (Yoge): editor grid lines stay neutral; only the ruler shows the snap colours.
// Sub-beats follow the snap divisor. They draw above the lane tint and below the hits.
export const gridLines = {
  sub: "rgba(255,255,255,.025)",
  beat: "rgba(255,255,255,.045)",
  bar: "rgba(255,255,255,.22)",
};

// v3 §5: a snap tick's height on the ruler by its reduced divisor (1/5, 1/7, 1/9 take the smallest).
export const snapTickHeights = { 2: 9, 3: 7, 4: 7, 6: 5, 8: 5, 12: 4, 16: 4 };

export const limit = (x, min, max) => Math.min(Math.max(x, min), Math.max(min, max));

export function volumeColour(volume) {
  return volumePalette[limit(Math.round(volume), 0, 100)];
}

export function nameOf(path) {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1];
}

export function clock(ms) {
  const rounded = Math.max(Math.round(ms), 0);
  const minutes = Math.floor(rounded / 60000);
  const seconds = (rounded % 60000 / 1000).toFixed(3);
  return String(minutes).padStart(2, "0") + ":" + seconds.padStart(6, "0");
}

// Reads an osu! editor time ("01:45.263", or "01:45:263" as in modding timestamps) or plain milliseconds.
export function parseClock(text) {
  const value = String(text).trim();
  const time = value.match(/^(\d+):(\d{1,2})(?:([.:])(\d{1,3}))?(?![\d.:a-z])/i);
  if (time) {
    const [, minutes, seconds, mark, part = ""] = time;
    const milliseconds = mark === "." ? Number(part.padEnd(3, "0")) : Number(part || 0);
    return Number(minutes) * 60000 + Number(seconds) * 1000 + milliseconds;
  }
  return /^\d+(\.\d+)?$/.test(value) ? Number(value) : NaN;
}

// "Artist - Title (Creator) [Difficulty].osu" gives the map name and "[Difficulty]"; other names show whole.
export function recentMapName(path) {
  const file = nameOf(path).replace(/\.(osu|osz)$/i, "");
  const parts = /^(.+?)\s*\([^()]*\)\s*(\[[^\]]*\])$/.exec(file);
  return parts ? [parts[1], parts[2]] : [file, ""];
}

export function outputProblemTime(problem) {
  return (problem.time < 0 ? "−" : "") + clock(Math.abs(problem.time));
}

export function describeOutputProblem(problem) {
  return outputProblemTime(problem) + " · " + nameOf(problem.target) + " · " + problem.message;
}

export function reducedDivisor(tick, divisor) {
  let remainder = Math.abs(tick % divisor);
  let factor = divisor;
  while (remainder) [factor, remainder] = [remainder, factor % remainder];
  return divisor / factor;
}

export function snapColour(tick, divisor) {
  let remainder = Math.abs(tick % divisor);
  let factor = divisor;
  while (remainder) [factor, remainder] = [remainder, factor % remainder];
  return snapColours[divisor / factor] ?? "#ff0000";
}

export function visibleWarningMarkers(problems, path, start, zoom, width) {
  const visible = problems.filter(problem => {
    const x = (problem.time - start) * zoom;
    return problem.timeline === path && x >= 0 && x <= width;
  }).sort((first, second) => first.time - second.time);
  const markers = [];
  for (const problem of visible) {
    const x = (problem.time - start) * zoom;
    const last = markers[markers.length - 1];
    if (last && x - last.x < 8) last.problems.push(problem);
    else markers.push({ x, problems: [problem] });
  }
  return markers;
}

// A slider track: the combo colour 38 % into #111111 (timeline-spec §9).
const trackShades = new Map();
export function trackShade(colour) {
  if (!trackShades.has(colour)) {
    const channels = [1, 3, 5].map(at => Math.round(17 + (parseInt(colour.slice(at, at + 2), 16) - 17) * 0.38));
    trackShades.set(colour, "#" + channels.map(value => value.toString(16).padStart(2, "0")).join(""));
  }
  return trackShades.get(colour);
}

// Folders first, then by name: each listing is sorted once, when it arrives or the lanes change.
export const byFolderThenName = ([left, a], [right, b]) =>
  Number(a !== null) - Number(b !== null) || left.localeCompare(right);

// A key's name as the editor's key list writes it ("Ctrl+Shift+1", "Numpad *"), following
// keyboard::chord and Key::name in Rust, or null for a key that can't be a shortcut.
export function shortcutName(chord) {
  const pad = chord.code.startsWith("Numpad") ? chord.code.slice(6) : "";
  const padKey = { Divide: "/", Multiply: "*", Subtract: "-", Add: "+", Decimal: ".", Enter: "Enter" }[pad] ??
    (/^\d$/.test(pad) ? pad : "");
  const named = (value) => {
    value = {
      " ": "Space", Escape: "Esc", Delete: "Del", BracketLeft: "[", BracketRight: "]", Slash: "/",
      Period: ".", Comma: ",", Minus: "-", Equal: "=", Backslash: "\\", Semicolon: ";", Quote: "'",
      Backquote: "`",
    }[value] ?? value.replace(/^Arrow/, "");
    if (/^[a-z]$/i.test(value)) return value.toUpperCase();
    return /^([0-9[\]/.,\-=\\;'`]|Space|Enter|Esc|Backspace|Del|Home|End|PageUp|PageDown|Insert|Tab|Left|Right|Up|Down|F([1-9]|1[0-2]))$/
      .test(value) ? value : "";
  };
  const key = padKey ? "Numpad " + padKey : named(chord.key) || named(chord.code.replace(/^(Key|Digit)/, ""));
  return key ? [chord.ctrl && "Ctrl", chord.shift && "Shift", chord.alt && "Alt", key].filter(Boolean).join("+") : null;
}

export function keyInput(event) {
  return {
    key: event.key,
    code: event.code ?? "",
    ctrl: !!(event.ctrlKey || event.metaKey),
    shift: !!event.shiftKey,
    alt: !!event.altKey,
  };
}
