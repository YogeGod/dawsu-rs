// Geometry, plain-English metadata, Simple Flow extraction, and timeline overlay helpers for NodeGraphWindow.

export const NODE_WIDTH = 200;
export const NODE_HEADER_H = 26;
export const SOCKET_ROW_H = 20;
export const NODE_PAD_BOTTOM = 10;

export const BEAT_DIVISORS = [1, 2, 3, 4, 6, 8, 12, 16];

export const BAND_PRESETS = [
  { id: "sub", label: "Sub 20–80 Hz", loHz: 20, hiHz: 80 },
  { id: "kick", label: "Kick 30–120 Hz", loHz: 30, hiHz: 120 },
  { id: "punch", label: "Punch 80–220 Hz", loHz: 80, hiHz: 220 },
  { id: "body", label: "Body 150–400 Hz", loHz: 150, hiHz: 400 },
  { id: "wires", label: "Snare Snap 1.5–5k", loHz: 1500, hiHz: 5000 },
  { id: "hat", label: "Hi-Hat 7–11 kHz", loHz: 7000, hiHz: 11000 },
];

export const RECIPE_OPTIONS = [
  { id: "classic", label: "Kicks & Snares (Classic)" },
  { id: "simple_kick", label: "Simple Kick (5 steps)" },
  { id: "simple_hihat", label: "Simple Hi-Hat" },
  { id: "drum_stem", label: "Drum Stem (Fast)" },
  { id: "empty", label: "Blank Graph" },
];

export const NODE_META = {
  "input.song": {
    title: "Song Audio",
    stage: "source",
    description: "Reads the current beatmap's main audio track.",
    params: {},
  },
  "input.file": {
    title: "Audio / Stem File",
    stage: "source",
    description: "Reads a separate audio or drum stem file from disk.",
    params: {
      path: { label: "Audio file", control: "file", advanced: false },
      offsetMs: { label: "Stem offset (ms)", control: "number", advanced: false },
      expectLengthOfSong: { label: "Match song length", control: "bool", advanced: true },
    },
  },
  "input.timing": {
    title: "Beat Grid",
    stage: "source",
    description: "Reads red timing lines (BPM) from the active difficulty.",
    params: {},
  },
  "input.range": {
    title: "Time Range",
    stage: "source",
    description: "Passes either your L–R selection or the full song.",
    params: {
      mode: { label: "Range", control: "enum", advanced: false },
    },
  },
  "audio.mono": {
    title: "Mix to Mono",
    stage: "listen",
    description: "Converts stereo audio to mono and normalises loudness.",
    params: {
      mix: { label: "Channel mix", control: "enum", advanced: false },
      peakNormalise: { label: "Normalise peak", control: "bool", advanced: false },
      normaliseOver: { label: "Normalise over", control: "enum", advanced: true },
      silenceBelow: { label: "Silence floor", control: "number", advanced: true },
    },
  },
  "audio.filter": {
    title: "Audio Filter",
    stage: "listen",
    description: "Filters out unwanted low or high frequencies.",
    params: {
      mode: { label: "Filter type", control: "enum", advanced: false },
      loHz: { label: "Low cutoff (Hz)", control: "number", advanced: false },
      hiHz: { label: "High cutoff (Hz)", control: "number", advanced: false },
      q: { label: "Resonance (Q)", control: "number", advanced: true },
    },
  },
  "spectral.stft": {
    title: "Frequency Map",
    stage: "listen",
    description: "Splits audio into short frequency slices (STFT).",
    params: {
      hopMs: { label: "Time step (ms)", control: "number", advanced: false },
      maxHz: { label: "Max Hz", control: "number", advanced: false },
      window: { label: "FFT size", control: "integer", advanced: true },
      windowShape: { label: "Window shape", control: "enum", advanced: true },
      minHops: { label: "Min frames", control: "integer", advanced: true },
    },
  },
  "spectral.hpss": {
    title: "Isolate Drums",
    stage: "listen",
    description: "Separates sharp drum hits from vocals, guitars, and pads.",
    params: {
      keep: { label: "Keep component", control: "enum", advanced: false },
      medianHalf: { label: "Separation strength", control: "integer", advanced: true },
      mask: { label: "Mask mode", control: "enum", advanced: true },
      epsilon: { label: "Epsilon", control: "number", advanced: true },
    },
  },
  "onset.flux": {
    title: "Listen to Band",
    stage: "listen",
    description: "Measures sudden energy spikes inside one frequency band.",
    params: {
      loHz: { label: "Low Hz", control: "number", advanced: false },
      hiHz: { label: "High Hz", control: "number", advanced: false },
      compression: { label: "Log compression", control: "number", advanced: true },
      maxBins: { label: "Spectral width", control: "integer", advanced: true },
      normalise: { label: "Normalise", control: "enum", advanced: true },
      normaliseOver: { label: "Normalise over", control: "enum", advanced: true },
    },
  },
  "curve.math": {
    title: "Combine Curves",
    stage: "listen",
    description: "Blends two energy curves together (multiply, add, or mean).",
    params: {
      op: { label: "Combine mode", control: "enum", advanced: false },
      factor: { label: "Scale factor", control: "number", advanced: true },
      normalise: { label: "Normalise", control: "enum", advanced: true },
      normaliseOver: { label: "Normalise over", control: "enum", advanced: true },
    },
  },
  "events.peaks": {
    title: "Pick Hits",
    stage: "detect",
    description: "Finds sharp peaks above the sensitivity line.",
    params: {
      threshold: { label: "Sensitivity", control: "sensitivity", advanced: false },
      minGapMs: { label: "Min gap (ms)", control: "number", advanced: false },
      tag: { label: "Hit tag", control: "text", advanced: false },
      floorScale: { label: "Noise floor scale", control: "number", advanced: true },
      deltaScale: { label: "Delta threshold", control: "number", advanced: true },
      maxHalfFrames: { label: "Peak window", control: "integer", advanced: true },
      meanBefore: { label: "Lookback frames", control: "integer", advanced: true },
      meanAfter: { label: "Lookahead frames", control: "integer", advanced: true },
      biasMs: { label: "Timing nudge (ms)", control: "number", advanced: true },
    },
  },
  "events.sample_time": {
    title: "Beat Grid Events",
    stage: "detect",
    description: "Generates regular hits every 1/N beat.",
    params: {
      divisors: { label: "Divisors", control: "divisors", advanced: false },
      tag: { label: "Tag", control: "text", advanced: false },
    },
  },
  "events.measure": {
    title: "Measure Curve",
    stage: "detect",
    description: "Attaches energy values from another band to each hit.",
    params: {
      name: { label: "Measure name", control: "text", advanced: false },
      halfFrames: { label: "Window frames", control: "integer", advanced: true },
      reduce: { label: "Reducer", control: "enum", advanced: true },
    },
  },
  "events.condition": {
    title: "Filter / Bleed Guard",
    stage: "detect",
    description: "Keeps or drops hits using energy rules (stops kick bleed).",
    params: {
      action: { label: "Action", control: "enum", advanced: false },
      clauses: { label: "Rules (JSON)", control: "json", advanced: true },
    },
  },
  "events.merge": {
    title: "Merge Hits",
    stage: "detect",
    description: "Combines multiple hit streams into one timeline.",
    params: {},
  },
  "events.snap": {
    title: "Snap to Beat",
    stage: "detect",
    description: "Aligns hits to nearest osu! beat lines and measures lag.",
    params: {
      divisors: { label: "Allowed divisions", control: "divisors", advanced: false },
      toleranceMs: { label: "Snap window (ms)", control: "number", advanced: false },
      measureLag: { label: "Auto lag align", control: "bool", advanced: false },
      dropOffGrid: { label: "Drop off-grid hits", control: "bool", advanced: false },
      maxLagMs: { label: "Max auto lag (ms)", control: "number", advanced: true },
      minEventsForLag: { label: "Min hits for lag", control: "integer", advanced: true },
      lagRefineWindowMs: { label: "Lag refine window", control: "number", advanced: true },
      lagRefinePasses: { label: "Lag refine passes", control: "integer", advanced: true },
    },
  },
  "output.lane": {
    title: "Hitsound Lane",
    stage: "output",
    description: "Sends hits to a named output slot bound to your beatmap.",
    params: {
      slot: { label: "Slot name", control: "text", advanced: false },
      tagFilter: { label: "Only keep tag", control: "text", advanced: false },
    },
  },
  "output.viewer": {
    title: "Timeline Viewer",
    stage: "output",
    description: "Displays any curve or hit list directly over the timeline.",
    params: {
      label: { label: "Viewer label", control: "text", advanced: false },
    },
  },
};

export function nodeMeta(typeId) {
  return NODE_META[typeId] ?? {
    title: typeId,
    stage: typeId.startsWith("input.") ? "source" : typeId.startsWith("output.") ? "output" : typeId.startsWith("events.") ? "detect" : "listen",
    description: "",
    params: {},
  };
}

// Convert peak-picker threshold (2.5 strict .. 0.5 loose) <-> user-friendly Sensitivity % (0..100)
export function thresholdToSensitivity(threshold) {
  const t = Number.isFinite(Number(threshold)) ? Number(threshold) : 1.5;
  return Math.round(Math.max(0, Math.min(100, ((2.5 - t) / 2.0) * 100)));
}

export function sensitivityToThreshold(pct) {
  const s = Math.max(0, Math.min(100, Number(pct) || 0));
  return Number((2.5 - (s / 100) * 2.0).toFixed(2));
}

const SOCKET_COLOURS = {
  audio: "#62d6cf",
  spectrogram: "#9c9cfc",
  curve: "#f1ad67",
  events: "#77bdfa",
  timing: "#e28dd1",
  range: "#69d995",
  number: "#c4c4c4",
  integer: "#c4c4c4",
  bool: "#c4c4c4",
  enum: "#c4c4c4",
  any: "#ffffff",
};

export function socketColour(typeName) {
  return SOCKET_COLOURS[typeName] ?? "#a3a3a3";
}

export function stageAccentKey(stage) {
  if (stage === "source") return "source";
  if (stage === "listen") return "listen";
  if (stage === "detect") return "detect";
  if (stage === "output") return "output";
  return "listen";
}

export function nodeHeight(node) {
  const rows = Math.max(node.inputs?.length ?? 0, node.outputs?.length ?? 0, 1);
  const summaryBonus = node.summary?.kind === "curve" && node.summary.sparkline?.length ? 30 : 20;
  return NODE_HEADER_H + summaryBonus + rows * SOCKET_ROW_H + NODE_PAD_BOTTOM;
}

export function inputSocketPos(node, socketId, dx = 0, dy = 0) {
  const index = (node.inputs ?? []).findIndex((s) => s.id === socketId);
  const row = index >= 0 ? index : 0;
  const topOffset = NODE_HEADER_H + (node.summary?.kind === "curve" && node.summary.sparkline?.length ? 30 : 20);
  return {
    x: node.x + dx,
    y: node.y + dy + topOffset + row * SOCKET_ROW_H + SOCKET_ROW_H * 0.5,
  };
}

export function outputSocketPos(node, socketId, dx = 0, dy = 0) {
  const index = (node.outputs ?? []).findIndex((s) => s.id === socketId);
  const row = index >= 0 ? index : 0;
  const topOffset = NODE_HEADER_H + (node.summary?.kind === "curve" && node.summary.sparkline?.length ? 30 : 20);
  return {
    x: node.x + dx + NODE_WIDTH,
    y: node.y + dy + topOffset + row * SOCKET_ROW_H + SOCKET_ROW_H * 0.5,
  };
}

export function linkPath(from, to) {
  const span = Math.max(48, Math.abs(to.x - from.x) * 0.5);
  const c1x = from.x + span;
  const c2x = to.x - span;
  return `M ${from.x.toFixed(1)} ${from.y.toFixed(1)} C ${c1x.toFixed(1)} ${from.y.toFixed(1)}, ${c2x.toFixed(1)} ${to.y.toFixed(1)}, ${to.x.toFixed(1)} ${to.y.toFixed(1)}`;
}

export function fitViewBox(nodes) {
  if (!nodes || !nodes.length) {
    return { x: -60, y: -60, w: 960, h: 420 };
  }
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of nodes) {
    const h = nodeHeight(n);
    if (n.x < minX) minX = n.x;
    if (n.y < minY) minY = n.y;
    if (n.x + NODE_WIDTH > maxX) maxX = n.x + NODE_WIDTH;
    if (n.y + h > maxY) maxY = n.y + h;
  }
  const pad = 60;
  const w = Math.max(540, maxX - minX + pad * 2);
  const h = Math.max(280, maxY - minY + pad * 2);
  return { x: minX - pad, y: minY - pad, w, h };
}

export function formatHz(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return "";
  return n >= 1000 ? `${Number((n / 1000).toFixed(1))}k` : `${Math.round(n)}`;
}

export function nodeSubtitle(node, slots = []) {
  const p = node.params ?? {};
  switch (node.typeId) {
    case "input.file": {
      const raw = String(p.path ?? "").trim();
      if (!raw) return "No file picked";
      const parts = raw.split(/[\\/]/);
      return parts[parts.length - 1] || raw;
    }
    case "onset.flux":
      return `${formatHz(p.loHz ?? 30)}–${formatHz(p.hiHz ?? 120)} Hz`;
    case "audio.filter":
      return `${p.mode ?? "bandpass"} · ${formatHz(p.loHz ?? 80)}–${formatHz(p.hiHz ?? 5000)} Hz`;
    case "events.peaks":
      return `Sens ${thresholdToSensitivity(p.threshold ?? 1.5)}% · ${Math.round(Number(p.minGapMs ?? 60))} ms`;
    case "events.snap": {
      const divs = Array.isArray(p.divisors) ? p.divisors : [4];
      return divs.map((d) => `1/${d}`).join(", ") || "No grid";
    }
    case "spectral.hpss":
      return node.muted ? "Bypassed (Off)" : `Keep ${p.keep ?? "percussive"}`;
    case "output.lane": {
      const slotName = String(p.slot ?? "default");
      const bound = slots.find((s) => s.slot === slotName);
      return bound?.boundLaneName ? `${slotName} → ${bound.boundLaneName}` : `${slotName} (unbound)`;
    }
    default:
      return nodeMeta(node.typeId).title;
  }
}

export function nodeBadgeText(node) {
  if (node.muted) return "OFF";
  const s = node.summary;
  if (!s) return "";
  if (s.kind === "events") return `${s.count} hits`;
  if (s.kind === "audio") return `${(s.durationMs / 1000).toFixed(1)}s`;
  if (s.kind === "timing") return `${s.lines} bpm`;
  if (s.kind === "spectrogram") return `${s.frames}f`;
  return "";
}

export function sparklinePoints(values, width = 180, height = 24, padY = 3) {
  if (!values || !values.length) return "";
  let min = Infinity;
  let max = -Infinity;
  for (const v of values) {
    if (Number.isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return "";
  const span = max > min ? max - min : 1;
  const usableH = Math.max(4, height - padY * 2);
  const denom = Math.max(1, values.length - 1);
  const pts = [];
  for (let i = 0; i < values.length; i++) {
    const v = Number.isFinite(values[i]) ? values[i] : min;
    const x = (i / denom) * width;
    const norm = max > min ? (v - min) / span : 0.5;
    const y = padY + (1 - norm) * usableH;
    pts.push(`${x.toFixed(1)},${y.toFixed(1)}`);
  }
  return pts.join(" ");
}

export function autoLayoutNodes(nodes, links) {
  const depth = new Map();
  for (const n of nodes) depth.set(n.id, 0);
  for (let pass = 0; pass < nodes.length; pass++) {
    let changed = false;
    for (const l of links) {
      const next = (depth.get(l.fromNode) ?? 0) + 1;
      if (next > (depth.get(l.toNode) ?? 0)) {
        depth.set(l.toNode, next);
        changed = true;
      }
    }
    if (!changed) break;
  }
  const cols = new Map();
  for (const n of nodes) {
    const d = depth.get(n.id) ?? 0;
    if (!cols.has(d)) cols.set(d, []);
    cols.get(d).push(n);
  }
  const positions = [];
  const sortedCols = [...cols.keys()].sort((a, b) => a - b);
  for (const c of sortedCols) {
    const group = cols.get(c);
    group.sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
    let curY = 24;
    for (const n of group) {
      positions.push({ id: n.id, x: 24 + c * 250, y: curY });
      curY += nodeHeight(n) + 28;
    }
  }
  return positions;
}

export function extractSimpleFlowModel(view) {
  if (!view) return null;
  const nodes = view.nodes ?? [];
  const links = view.links ?? [];
  const byId = new Map(nodes.map((n) => [n.id, n]));

  const songNode = nodes.find((n) => n.typeId === "input.song") ?? null;
  const fileNode = nodes.find((n) => n.typeId === "input.file") ?? null;
  const hpssNode = nodes.find((n) => n.typeId === "spectral.hpss") ?? null;
  const snapNode = nodes.find((n) => n.typeId === "events.snap") ?? null;

  const findUpstream = (startId, targetTypes) => {
    const visited = new Set();
    const queue = [startId];
    while (queue.length) {
      const cur = queue.shift();
      for (const l of links) {
        if (l.toNode !== cur || visited.has(l.fromNode)) continue;
        visited.add(l.fromNode);
        const node = byId.get(l.fromNode);
        if (!node) continue;
        if (targetTypes.includes(node.typeId)) return node;
        queue.push(node.id);
      }
    }
    return null;
  };

  const findDownstream = (startId, targetType) => {
    const visited = new Set();
    const queue = [startId];
    while (queue.length) {
      const cur = queue.shift();
      for (const l of links) {
        if (l.fromNode !== cur || visited.has(l.toNode)) continue;
        visited.add(l.toNode);
        const node = byId.get(l.toNode);
        if (!node) continue;
        if (node.typeId === targetType) return node;
        queue.push(node.id);
      }
    }
    return null;
  };

  const detectors = nodes
    .filter((n) => n.typeId === "events.peaks")
    .map((peaks) => {
      const flux = findUpstream(peaks.id, ["onset.flux"]);
      const curveSource = flux ?? findUpstream(peaks.id, ["curve.math"]);
      const bleed = findDownstream(peaks.id, "events.condition");
      const tag = String(peaks.params?.tag ?? peaks.id);
      const isSnare = /snare|clap/i.test(tag) || /snare/i.test(peaks.id);
      const isHat = /hat|cymbal|hihat/i.test(tag) || /hat/i.test(peaks.id);
      const accent = isSnare ? "clap" : isHat ? "hat" : "kick";
      return {
        peaks,
        flux,
        curveSource,
        bleed,
        title: isSnare ? "Snare / Clap Detector" : isHat ? "Hi-Hat Detector" : "Kick Detector",
        accent,
        sensitivity: thresholdToSensitivity(peaks.params?.threshold ?? 1.5),
        minGapMs: Number(peaks.params?.minGapMs ?? 60),
        foundCount: peaks.summary?.kind === "events" ? peaks.summary.count : null,
      };
    });

  const proposals = view.batch?.proposals ?? [];
  const slotCounts = new Map();
  for (const p of proposals) {
    if (p.checked) {
      slotCounts.set(p.slot, (slotCounts.get(p.slot) ?? 0) + 1);
    }
  }
  const slotOutputs = (view.slots ?? []).map((s) => {
    const outNode = nodes.find((n) => n.typeId === "output.lane" && String(n.params?.slot ?? "") === s.slot);
    const rawCount = outNode?.summary?.kind === "events" ? outNode.summary.count : null;
    const batchCount = slotCounts.has(s.slot) ? slotCounts.get(s.slot) : rawCount;
    return { ...s, hitCount: batchCount };
  });

  return {
    sourceNode: fileNode ?? songNode,
    isStem: !!fileNode && !songNode,
    fileNode,
    hpssNode,
    detectors,
    snapNode,
    slotOutputs,
  };
}

export function formatParamValue(spec, value) {
  if (spec.socketType === "number" || spec.socketType === "integer") {
    return value ?? spec.default ?? 0;
  }
  if (typeof value === "string") return value;
  return JSON.stringify(value ?? spec.default ?? "");
}

export function parseParamInput(spec, rawValue, checked) {
  if (spec.socketType === "bool") return { ok: true, value: !!checked };
  if (spec.socketType === "enum") {
    return { ok: true, value: String(rawValue) };
  }
  if (spec.socketType === "integer") {
    const n = Number(rawValue);
    if (!Number.isFinite(n) || !Number.isInteger(n)) return { ok: false, error: "Expected whole number" };
    return { ok: true, value: n };
  }
  if (spec.socketType === "number") {
    const text = String(rawValue).trim();
    if (text === "null" && spec.default === null) return { ok: true, value: null };
    const n = Number(text);
    if (!Number.isFinite(n)) return { ok: false, error: "Expected finite number" };
    return { ok: true, value: n };
  }
  try {
    return { ok: true, value: JSON.parse(String(rawValue)) };
  } catch {
    return { ok: true, value: String(rawValue) };
  }
}

export function visibleViewerOverlay(viewer, startMs, endMs, xOf, topY, height) {
  if (!viewer || height <= 2) return null;
  if (viewer.kind === "curve") {
    const values = viewer.values ?? [];
    if (!values.length || !(viewer.hopMs > 0)) return null;
    const stepMs = (viewer.sampleStep || 1) * viewer.hopMs;
    const min = viewer.min ?? 0;
    const max = viewer.max ?? 1;
    const span = max > min ? max - min : 1;
    const first = Math.max(0, Math.floor((startMs - viewer.startMs) / stepMs) - 1);
    const last = Math.min(values.length - 1, Math.ceil((endMs - viewer.startMs) / stepMs) + 1);
    const points = [];
    for (let i = first; i <= last; i++) {
      const t = viewer.startMs + i * stepMs;
      const v = values[i];
      if (!Number.isFinite(v)) continue;
      const norm = max > min ? (v - min) / span : 0.5;
      const y = topY + (1 - Math.max(0, Math.min(1, norm))) * height;
      points.push([xOf(t), y]);
    }
    return { kind: "curve", points };
  }
  if (viewer.kind === "events") {
    const times = viewer.timesMs ?? [];
    const strengths = viewer.strengths ?? [];
    const markers = [];
    for (let i = 0; i < times.length; i++) {
      const t = times[i];
      if (t < startMs - 20 || t > endMs + 20) continue;
      markers.push({ x: xOf(t), strength: strengths[i] ?? 1 });
    }
    return { kind: "events", markers };
  }
  return null;
}
