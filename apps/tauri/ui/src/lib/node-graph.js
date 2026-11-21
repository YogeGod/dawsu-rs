// Pure geometry, colour, and timeline viewer helpers for the Svelte Node Graph panel.

export const NODE_WIDTH = 180;
export const NODE_HEADER_H = 26;
export const SOCKET_ROW_H = 20;
export const NODE_PAD_BOTTOM = 8;

const SOCKET_COLOURS = {
  audio: "#77bdfa",
  spectrogram: "#c594f5",
  curve: "#f0aa70",
  events: "#62d6cf",
  timing: "#f0708a",
  range: "#9cd670",
  number: "#b7c4d0",
  integer: "#b7c4d0",
  bool: "#b7c4d0",
  enum: "#b7c4d0",
  any: "#ffffff",
};

export function socketColour(typeName) {
  return SOCKET_COLOURS[typeName] ?? "#9aa3ad";
}

export function nodeHeight(node) {
  const rows = Math.max(node.inputs?.length ?? 0, node.outputs?.length ?? 0, 1);
  return NODE_HEADER_H + rows * SOCKET_ROW_H + NODE_PAD_BOTTOM;
}

export function inputSocketPos(node, socketId, dx = 0, dy = 0) {
  const index = (node.inputs ?? []).findIndex((s) => s.id === socketId);
  const row = index >= 0 ? index : 0;
  return {
    x: node.x + dx,
    y: node.y + dy + NODE_HEADER_H + row * SOCKET_ROW_H + SOCKET_ROW_H * 0.5,
  };
}

export function outputSocketPos(node, socketId, dx = 0, dy = 0) {
  const index = (node.outputs ?? []).findIndex((s) => s.id === socketId);
  const row = index >= 0 ? index : 0;
  return {
    x: node.x + dx + NODE_WIDTH,
    y: node.y + dy + NODE_HEADER_H + row * SOCKET_ROW_H + SOCKET_ROW_H * 0.5,
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
    return { x: -80, y: -80, w: 960, h: 420 };
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
  const w = Math.max(480, maxX - minX + pad * 2);
  const h = Math.max(240, maxY - minY + pad * 2);
  return { x: minX - pad, y: minY - pad, w, h };
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
    if (spec.enumValues && spec.enumValues.length) {
      return { ok: true, value: String(rawValue) };
    }
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
