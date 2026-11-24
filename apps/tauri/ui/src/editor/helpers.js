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
export const MAX_DRAW_STROKE_STEPS = 512;

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

// Canvas surfaces match Apple Logic Pro for iPad's pure black & neutral obsidian.
export const slateTimeline = {
  arrange: "#0e0e10", alternateRow: "#151517", activeRow: "#26262a", group: "#1c1c1f",
  ruler: "#08080a", rulerBar: "#f5f5f7", rulerEdge: "#2b2b30", overview: "#151517", waveform: "#121718",
  volume: "#0e0e10", divider: "#2b2b30", beatLine: "#2b2b30", barLine: "#44444c",
  cycle: "#ffd60a", loop: "#5ac8fa14", locator: "#8e8e96", playhead: "#f5f5f7",
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
// Neutral grid (osu! snap colours stay on the ruler), a step brighter than tokens.css's
// --grid-sub/-beat/-bar so sub-beats still read on the darker arrange rows.
export const gridLines = {
  sub: "rgba(255,255,255,.04)",
  beat: "rgba(255,255,255,.07)",
  bar: "rgba(255,255,255,.26)",
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

/**
 * Finds the best matching moment within `tolerance` of `ms` on a time-sorted `moments` array.
 * Binary-searches to `ms - tolerance` and stops past `ms + tolerance`.
 */
export function findNearestMoment(moments, ms, tolerance, match, { strict = true } = {}) {
  if (!moments?.length || !(strict ? tolerance > 0 : tolerance >= 0)) return null;
  // Compare `ms - time > tolerance` (monotone in sorted `time`) so the search cutoff matches `Math.abs(time - ms)`.
  let lo = 0;
  let hi = moments.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (ms - moments[mid][0] > tolerance) lo = mid + 1;
    else hi = mid;
  }
  let best = tolerance;
  let found = null;
  for (let i = lo; i < moments.length; i++) {
    const moment = moments[i];
    const time = moment[0];
    if (time - ms > tolerance) break;
    const distance = Math.abs(time - ms);
    if (!(strict ? distance < best : distance <= best)) continue;
    const value = match(moment);
    if (value === undefined) continue;
    best = distance;
    found = value;
  }
  return found;
}

function activeSectionFor(sections, ms) {
  if (!sections?.length) return null;
  let chosen = sections[0];
  for (let i = 1; i < sections.length; i++) {
    if (sections[i][0] <= ms + 1e-6) chosen = sections[i];
    else break;
  }
  return chosen;
}

function closestObjectTime(moments, ms) {
  if (!moments?.length) return null;
  let best = Infinity;
  let found = null;
  for (let i = 0; i < moments.length; i++) {
    const moment = moments[i];
    if (!moment[2]) continue;
    const dist = Math.abs(moment[0] - ms);
    if (dist < best) {
      best = dist;
      found = moment[0];
    }
  }
  return found;
}

export function snapTimeInScene(scene, ms) {
  const clamped = Math.max(0, ms);
  if (!scene?.snap) return clamped;
  if (scene.snapMode === "objects") {
    return closestObjectTime(scene.moments, clamped) ?? clamped;
  }
  const section = activeSectionFor(scene.sections, clamped);
  if (!section || !(section[1] > 0)) return clamped;
  const beats = Math.max(1, scene.beats || 1);
  const divisor = Math.max(1, scene.divisor || 4);
  const step = (section[1] * beats) / divisor;
  if (!(step > 0)) return clamped;
  const origin = section[0];
  let snapped = origin + Math.round((clamped - origin) / step) * step;
  if (snapped < 0) snapped += step;
  return snapped;
}

function nextGridStepInScene(scene, curMs, dir) {
  const sections = scene.sections;
  if (!sections?.length) return curMs;
  const beats = Math.max(1, scene.beats || 1);
  const divisor = Math.max(1, scene.divisor || 4);
  let idx = 0;
  for (let i = 1; i < sections.length; i++) {
    if (sections[i][0] <= curMs + 1e-6) idx = i;
    else break;
  }
  const [origin, beat] = sections[idx];
  if (!(beat > 0)) return curMs;
  const step = (beat * beats) / divisor;
  if (!(step > 0)) return curMs;
  const rawIndex = (curMs - origin) / step;
  const nextIndex = dir > 0 ? Math.floor(rawIndex + 1e-6) + 1 : Math.ceil(rawIndex - 1e-6) - 1;
  let next = origin + nextIndex * step;
  if (dir > 0 && idx + 1 < sections.length && next >= sections[idx + 1][0] - 1e-6) {
    next = sections[idx + 1][0];
  } else if (dir < 0 && idx > 0 && next < origin - 1e-6) {
    const prev = sections[idx - 1];
    if (prev[1] > 0) {
      const prevStep = (prev[1] * beats) / divisor;
      const prevIdx = Math.ceil((curMs - prev[0]) / prevStep - 1e-6) - 1;
      next = prev[0] + prevIdx * prevStep;
    }
  }
  return Math.max(0, next);
}

/**
 * Computes all snapped step times between `startMs` and `endMs` (in drag order,
 * capped at `maxSteps`) and filters out steps where `row` already has a hit
 * within 5 ms. Used for live Draw-stroke ghost previews on the canvas.
 */
export function computeDrawStrokeSteps(scene, startMs, endMs, row = null, maxSteps = MAX_DRAW_STROKE_STEPS) {
  if (!scene || !Number.isFinite(startMs) || !Number.isFinite(endMs) || maxSteps <= 0) {
    return { steps: [], newSteps: [] };
  }
  const start = snapTimeInScene(scene, startMs);
  const end = snapTimeInScene(scene, endMs);
  const steps = [];
  if (!scene.snap || Math.abs(end - start) < 1e-6) {
    steps.push(end);
  } else if (scene.snapMode === "objects") {
    const forward = end > start;
    const lo = forward ? start : end;
    const hi = forward ? end : start;
    const raw = [];
    for (const moment of scene.moments ?? []) {
      if (!moment[2]) continue;
      if (moment[0] >= lo - 1e-6 && moment[0] <= hi + 1e-6) raw.push(moment[0]);
    }
    if (!forward) raw.reverse();
    for (const t of raw) {
      if (steps.length >= maxSteps) break;
      if (!steps.length || Math.abs(t - steps[steps.length - 1]) >= 1.0) steps.push(t);
    }
    if (!steps.length) steps.push(end);
  } else {
    const forward = end > start;
    const dir = forward ? 1 : -1;
    steps.push(start);
    let cur = start;
    while (steps.length < maxSteps) {
      const next = nextGridStepInScene(scene, cur, dir);
      if (Math.abs(next - cur) < 1e-6) break;
      if ((forward && next > end + 1e-6) || (!forward && next < end - 1e-6)) break;
      steps.push(next);
      cur = next;
      if (Math.abs(cur - end) < 1e-6) break;
    }
    if (steps.length < maxSteps && Math.abs(steps[steps.length - 1] - end) >= 1e-6) {
      steps.push(end);
    }
  }
  const newSteps = [];
  for (const t of steps) {
    if (newSteps.some(prev => Math.abs(prev - t) < 5.0)) continue;
    const occupied = typeof row === "number" && findNearestMoment(
      scene.moments,
      t,
      5.0,
      ([, , , hits]) => (hits.some(([laneRow]) => laneRow === row) ? true : undefined),
      { strict: false },
    );
    if (!occupied) newSteps.push(t);
  }
  return { steps, newSteps };
}
