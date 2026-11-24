// v3.1.4-style Color Scheme & Custom Colors engine (ported from Dialogs.Colours.cs / ThemeColors.cs / UiTheme.cs).
// Pure HSB conversion + 6 built-in presets (Obsidian default + the 5 v3.1.4 Cubase-style schemes) + 9 custom rows.

export const STORAGE_KEY = "dawsu.colorScheme";

export function hexToRgb(hex) {
  const clean = String(hex || "#000000").replace("#", "");
  const full = clean.length === 3 ? clean.split("").map(c => c + c).join("") : clean.slice(0, 6);
  const n = Number.parseInt(full, 16) || 0;
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

export function rgbToHex(r, g, b, alpha = null) {
  const byte = v => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
  const base = `#${byte(r)}${byte(g)}${byte(b)}`;
  return alpha == null ? base : `${base}${byte(alpha)}`;
}

export function hexToHsb(hex) {
  const { r: r8, g: g8, b: b8 } = hexToRgb(hex);
  const r = r8 / 255, g = g8 / 255, b = b8 / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let h = 0;
  if (d !== 0) {
    if (max === r) h = 60 * (((g - b) / d) % 6);
    else if (max === g) h = 60 * ((b - r) / d + 2);
    else h = 60 * ((r - g) / d + 4);
  }
  return { h: h < 0 ? h + 360 : h, s: max === 0 ? 0 : d / max, b: max };
}

export function hsbToHex({ h, s, b }, alpha = null) {
  const hn = (((h % 360) + 360) % 360) / 60;
  const sn = Math.max(0, Math.min(1, s));
  const vn = Math.max(0, Math.min(1, b));
  const c = vn * sn;
  const x = c * (1 - Math.abs((hn % 2) - 1));
  const m = vn - c;
  let r = 0, g = 0, bl = 0;
  const sector = Math.floor(hn);
  if (sector === 0) [r, g, bl] = [c, x, 0];
  else if (sector === 1) [r, g, bl] = [x, c, 0];
  else if (sector === 2) [r, g, bl] = [0, c, x];
  else if (sector === 3) [r, g, bl] = [0, x, c];
  else if (sector === 4) [r, g, bl] = [x, 0, c];
  else [r, g, bl] = [c, 0, x];
  return rgbToHex((r + m) * 255, (g + m) * 255, (bl + m) * 255, alpha);
}

function mixHex(a, b, t) {
  const ca = hexToRgb(a), cb = hexToRgb(b);
  return rgbToHex(ca.r + (cb.r - ca.r) * t, ca.g + (cb.g - ca.g) * t, ca.b + (cb.b - ca.b) * t);
}

function tintDarkPalette(base, hue, sat, lift) {
  const out = { ...base };
  for (const key of CHROME_KEYS) {
    if (!out[key]) continue;
    const c = hexToHsb(out[key]);
    if (c.s < 0.14) out[key] = hsbToHex({ h: hue, s: sat, b: Math.min(1, c.b + lift) });
  }
  return out;
}

const CHROME_KEYS = [
  "canvas", "panel", "chrome", "control", "hover", "selected", "border", "grid",
  "ruler", "major", "overview", "alternateRow", "activeRow", "group", "volume", "barLine",
];

const OBSIDIAN = {
  canvas: "#0e0e10", panel: "#151517", chrome: "#1c1c1f", control: "#242428",
  hover: "#303036", selected: "#3a3a41", border: "#2b2b30", grid: "#1a1a1d",
  ruler: "#08080a", major: "#34343a", overview: "#151517", alternateRow: "#151517",
  activeRow: "#26262a", group: "#1c1c1f", volume: "#0e0e10", barLine: "#44444c",
  text: "#f5f5f7", secondary: "#c7c7cc", muted: "#98989f", accent: "#5ac8fa",
  focus: "#8ee0ff", cycle: "#ffd60a", locator: "#8e8e96", reverse: "#ff453a",
  loop: "#5ac8fa14",
};

const GREY = {
  canvas: "#383838", panel: "#383838", chrome: "#494949", control: "#4d4d4d",
  hover: "#5a5a5a", selected: "#656565", border: "#171a1c", grid: "#424242",
  ruler: "#2e2e2e", major: "#5e5e5e", overview: "#2b2b2b", alternateRow: "#424242",
  activeRow: "#515151", group: "#424242", volume: "#2e2e2e", barLine: "#5e5e5e",
  text: "#edf0f1", secondary: "#c1c7cc", muted: "#bdbdbd", accent: "#287ac3",
  focus: "#5ca8ee", cycle: "#2f6fd0", locator: "#8e8e93", reverse: "#ff453a",
  loop: "#2f6fd024",
};

const DARK = {
  canvas: "#111113", panel: "#1b1b1e", chrome: "#27272b", control: "#27272b",
  hover: "#333338", selected: "#3e3e44", border: "#333338", grid: "#242427",
  ruler: "#27272b", major: "#414147", overview: "#1b1b1e", alternateRow: "#18181b",
  activeRow: "#2a2a30", group: "#27272b", volume: "#111113", barLine: "#414147",
  text: "#f4f4f6", secondary: "#b8b8c0", muted: "#9999a2", accent: "#0787f8",
  focus: "#5cb2ff", cycle: "#2f6fd0", locator: "#8e8e93", reverse: "#ff453a",
  loop: "#0787f824",
};

const SLATE = {
  canvas: "#13161b", panel: "#272c35", chrome: "#2e343f", control: "#3c4553",
  hover: "#46505e", selected: "#505b6a", border: "#363d48", grid: "#1f232a",
  ruler: "#232831", major: "#434c5a", overview: "#1f232a", alternateRow: "#1c2027",
  activeRow: "#2c313b", group: "#2a3039", volume: "#13161b", barLine: "#6f7b8c",
  text: "#f4f4f6", secondary: "#a9b4c2", muted: "#93a0b1", accent: "#3d9bff",
  focus: "#7cbcff", cycle: "#2fbf4a", locator: "#8e8e93", reverse: "#ff453a",
  loop: "#2fbf4a24",
};

export const SCHEMES = [
  { id: "obsidian", name: "Obsidian (Default)", swatch: "#1c1c1f", palette: OBSIDIAN },
  { id: "grey", name: "Grey", swatch: "#515151", palette: GREY },
  { id: "dark", name: "Dark", swatch: "#27272b", palette: DARK },
  { id: "blue-grey", name: "Blue-grey dark", swatch: "#273137", palette: tintDarkPalette(DARK, 202, 0.29, 0.05) },
  { id: "teal", name: "Teal dark", swatch: "#284c5a", palette: tintDarkPalette(DARK, 197, 0.56, 0.18) },
  { id: "slate", name: "Slate", swatch: "#2c313b", palette: SLATE },
];

export const COLOR_ROWS = [
  { id: "scheme", group: "Custom Colors", label: "User Color Scheme", mainKey: "panel", hasHue: true },
  { id: "accent", group: "Custom Colors", label: "Accent Color", mainKey: "accent", hasHue: true },
  { id: "gridBg", group: "Grid Colors", label: "Background", mainKey: "canvas", hasHue: false },
  { id: "gridLines", group: "Grid Colors", label: "Grid Lines", mainKey: "barLine", hasHue: false, requires: "gridBg" },
  { id: "gridCycle", group: "Grid Colors", label: "Cycle Region", mainKey: "cycle", hasHue: false, requires: "gridBg" },
  { id: "rulerBg", group: "Ruler Colors", label: "Background", mainKey: "ruler", hasHue: false },
  { id: "rulerInactive", group: "Ruler Colors", label: "Inactive Cycle", mainKey: "locator", hasHue: true },
  { id: "rulerActive", group: "Ruler Colors", label: "Active Cycle", mainKey: "cycle", hasHue: true },
  { id: "rulerReverse", group: "Ruler Colors", label: "Reverse Cycle", mainKey: "reverse", hasHue: true },
];

export function defaultColorConfig() {
  return { scheme: "obsidian", rows: {} };
}

export function readColorConfig(storage = null) {
  try {
    const s = storage ?? globalThis.localStorage;
    const raw = s?.getItem(STORAGE_KEY);
    if (!raw) return defaultColorConfig();
    const parsed = JSON.parse(raw);
    const scheme = SCHEMES.some(sc => sc.id === parsed?.scheme) ? parsed.scheme : "obsidian";
    const rows = parsed?.rows && typeof parsed.rows === "object" ? parsed.rows : {};
    return { scheme, rows };
  } catch {
    return defaultColorConfig();
  }
}

export function writeColorConfig(config, storage = null) {
  try {
    const s = storage ?? globalThis.localStorage;
    s?.setItem(STORAGE_KEY, JSON.stringify(config));
  } catch {}
}

export function basePalette(schemeId) {
  return (SCHEMES.find(s => s.id === schemeId) ?? SCHEMES[0]).palette;
}

export function rowStartHsb(row, schemeId) {
  const pal = basePalette(schemeId);
  return hexToHsb(pal[row.mainKey] || "#383838");
}

export function resolvePalette(config) {
  const base = { ...basePalette(config?.scheme) };
  const rows = config?.rows ?? {};
  const schemeHue = rows.scheme ? rows.scheme.h : hexToHsb(base.panel).h;
  const tinted = c => ({ h: schemeHue, s: c.s, b: c.b });

  if (rows.scheme) {
    const sc = rows.scheme;
    const step = sc.b - hexToHsb(base.panel).b;
    for (const key of CHROME_KEYS) {
      const orig = hexToHsb(base[key]);
      base[key] = hsbToHex({ h: sc.h, s: sc.s, b: Math.max(0, Math.min(1, orig.b + step)) });
    }
    if (sc.b > 0.62) {
      base.text = "#141416";
      base.secondary = "#3c3c42";
      base.muted = "#4a4a50";
    }
  }

  if (rows.accent) {
    base.accent = hsbToHex(rows.accent);
    base.focus = hsbToHex({ h: rows.accent.h, s: Math.max(0, rows.accent.s * 0.7), b: Math.min(1, rows.accent.b + 0.15) });
  }

  if (rows.gridBg) {
    const bg = tinted(rows.gridBg);
    base.canvas = hsbToHex(bg);
    base.alternateRow = hsbToHex({ ...bg, b: Math.min(1, bg.b + 0.03) });
    base.activeRow = hsbToHex({ ...bg, b: Math.min(1, bg.b + 0.09) });
    base.volume = base.canvas;
    if (rows.gridLines) {
      const lineHex = hsbToHex(tinted(rows.gridLines));
      base.barLine = lineHex;
      base.border = mixHex(lineHex, base.canvas, 0.4);
      base.grid = mixHex(lineHex, base.canvas, 0.7);
    }
    if (rows.gridCycle) {
      base.loop = hsbToHex(tinted(rows.gridCycle), 0x40);
    }
  }

  if (rows.rulerBg) base.ruler = hsbToHex(tinted(rows.rulerBg));
  if (rows.rulerInactive) base.locator = hsbToHex(rows.rulerInactive);
  if (rows.rulerActive) base.cycle = hsbToHex(rows.rulerActive);
  if (rows.rulerReverse) base.reverse = hsbToHex(rows.rulerReverse);

  return base;
}

export function applyColorConfig(config, doc = null) {
  const palette = resolvePalette(config);
  const root = (doc ?? globalThis.document)?.documentElement;
  if (root?.style) {
    const set = (name, val) => root.style.setProperty(name, val);
    set("--editor-canvas", palette.canvas);
    set("--editor-panel", palette.panel);
    set("--editor-chrome", palette.chrome);
    set("--editor-control", palette.control);
    set("--editor-hover", palette.hover);
    set("--editor-selected", palette.selected);
    set("--editor-border", palette.border);
    set("--editor-grid", palette.grid);
    set("--editor-ruler", palette.ruler);
    set("--ruler-bg", palette.ruler);
    set("--editor-major", palette.major);
    set("--editor-text", palette.text);
    set("--editor-secondary", palette.secondary);
    set("--editor-muted", palette.muted);
    set("--editor-readout", palette.text);
    set("--editor-accent", palette.accent);
    set("--editor-focus", palette.focus);
    set("--editor-warning", palette.cycle);
    set("--editor-error", palette.reverse);
    set("--transport-seg", palette.chrome);
  }
  const tl = globalThis.slateTimeline;
  if (tl) {
    tl.arrange = palette.canvas;
    tl.alternateRow = palette.alternateRow;
    tl.activeRow = palette.activeRow;
    tl.group = palette.group;
    tl.ruler = palette.ruler;
    tl.rulerBar = palette.text;
    tl.rulerEdge = palette.border;
    tl.overview = palette.overview;
    tl.volume = palette.volume;
    tl.divider = palette.border;
    tl.beatLine = palette.border;
    tl.barLine = palette.barLine;
    tl.cycle = palette.cycle;
    tl.loop = palette.loop;
    tl.locator = palette.locator;
    tl.playhead = palette.text;
  }
  globalThis.paintSoon?.();
  return palette;
}
