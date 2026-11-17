"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const grid = $("grid");
const controller = $("controller");
const ruler = $("ruler");
const overview = $("overview");
const dialog = $("dialog");
let view = { busy: false };
let scene = null;
let drag = null;
let markedRange = null;
let browserMap = "";
let browserFolder = "";
let browserSample = null;
let browserSource = "mapset";
let browserLibrary = null;
let browserLibraryFolders = new Map();
const browserExpanded = new Set();
let browserScene = null;
let browserState = "";
let usedLaneKeys = null;
let showLaneInfo = false;
let laneRows = [];
let lanePositions = new Map();
const collapsedFamilies = new Set();
let familyParents = new Map();
let previewParents = new Map();
const sampleFolderNames = { normal: "Normal set", soft: "Soft set", drum: "Drum set" };
let sampleFolderOrder = Object.keys(sampleFolderNames);
let layoutWrites = 0;
const collapsedFolders = new Set();
const sampleFolderDragType = "application/x-dawsu-sample-set";
let modalKind = "";
let modalClose = null;
let outputProblemIndex = -1;
let outputProblemList = "";
let rulerWarningMarkers = [];
const levelColours = [[58, 108, 240], [154, 72, 216], [236, 58, 58]];
const volumePalette = Array.from({ length: 101 }, (_, volume) => {
  const index = volume < 50 ? 0 : 1;
  const mix = (volume - index * 50) / 50;
  const channels = levelColours[index].map((channel, i) =>
    Math.trunc(channel + (levelColours[index + 1][i] - channel) * mix));
  return "rgb(" + channels.join(", ") + ")";
});

function volumeColour(volume) {
  return volumePalette[limit(Math.round(volume), 0, 100)];
}
let pending = Promise.resolve();
let requests = 0;
let refreshing = false;
let lastRefresh = 0;
let frame = 0;

const nameOf = (path) => path.split(/[\\/]/).at(-1);
const limit = (x, min, max) => Math.min(Math.max(x, min), Math.max(min, max));
const command = (id) => act({ kind: "command", id });
const toggle = (id, kind) => $(id).addEventListener("change", () => act({ kind, on: $(id).checked }));
const number = (id, fallback) => $(id).valueAsNumber || ($(id).value === "0" ? 0 : fallback);

function makeButton(label, click) {
  const button = document.createElement("button");
  button.textContent = label;
  button.addEventListener("click", click);
  return button;
}

function run(task) {
  requests++;
  const result = pending.then(task).catch((error) => {
    $("status").textContent = String(error);
    console.error(error);
  }).finally(() => { requests--; });
  pending = result;
  return result;
}

async function update(action, full = true) {
  let next = await invoke("editor_view", { scene: full, action: action ?? null });
  if (!full && (view.busy !== next.busy || view.hasDocument !== next.hasDocument ||
      view.laneCount !== next.laneCount || view.showHome !== next.showHome)) {
    next = await invoke("editor_view", { scene: true, action: null });
  }
  show(next);
  if (next.shortcutHelp) await showHelp();
  if (next.shortcutOpen && !next.busy) show(await invoke("pick_map"));
}

function act(action, full = true) {
  return run(() => update(action, full));
}

function hydrateLaneLayout(layout) {
  sampleFolderOrder = layout?.folder_order?.slice() ?? Object.keys(sampleFolderNames);
  collapsedFolders.clear();
  for (const group of layout?.collapsed_folders ?? []) collapsedFolders.add(group);
  collapsedFamilies.clear();
  for (const family of layout?.collapsed_families ?? []) collapsedFamilies.add(family);
}

function saveLaneLayout() {
  const path = scene.path;
  const value = {
    folder_order: [...sampleFolderOrder],
    collapsed_folders: [...collapsedFolders],
    collapsed_families: [...collapsedFamilies],
  };
  layoutWrites++;
  run(async () => {
    try {
      if (path === scene?.path) await update({ kind: "lane_layout", value }, false);
    } finally {
      layoutWrites--;
    }
  });
}

function show(next) {
  const wasPlaying = view.playing;
  const wasBusy = view.busy;
  view = next;
  if (!view.hasDocument || view.showHome) scene = null;
  if (view.scene) {
    const loaded = view.scene.path !== scene?.path || (wasBusy && view.status === "Difficulty loaded");
    if (loaded) {
      usedLaneKeys = null;
      markedRange = null;
      $("used-only").checked = false;
      $("lane-filter").value = "";
    }
    if (loaded || !layoutWrites) hydrateLaneLayout(view.scene.laneLayout);
    scene = view.scene;
  }
  $("home").hidden = !!scene;
  $("workspace").hidden = !scene;
  $("go-home").hidden = !scene;
  $("home-resume").hidden = !view.showHome || !view.hasDocument;
  $("home-resume").disabled = !!view.busy;
  const path = scene?.path ?? "";
  const title = $("title");
  title.textContent = path ? (view.dirty ? "● " : "") + nameOf(path) : "";
  title.title = path;
  $("status").textContent = view.status || (view.busy ? "Working…" : "");
  $("status").title = $("status").textContent;
  $("musical-position").textContent = view.musicalPosition ?? "";
  $("musical-position").hidden = !view.musicalPosition;
  $("warnings").hidden = !(view.warnings?.length);
  const problems = view.outputProblems ?? [];
  const listKey = JSON.stringify(problems);
  if (outputProblemList !== listKey) {
    outputProblemList = listKey;
    outputProblemIndex = -1;
    rulerWarningMarkers = [];
    ruler.title = "";
    if (modalKind === "output-problem") closeModal();
  }
  $("save-problem-badge").hidden = problems.length === 0;
  $("save-problem-badge").textContent = problems.length ? String(problems.length) : "";
  $("save-problem-badge").title = problems.length + " output problems";
  $("next-output-problem").hidden = problems.length === 0;
  $("next-output-problem").disabled = !!view.busy;
  $("cancel-job").hidden = !view.busy;
  $("difficulty").hidden = !scene;
  for (const button of document.querySelectorAll("[data-doc]")) button.disabled = !scene || !!view.busy;
  $("ramp-shape").value = view.rampShape ?? "Linear";
  $("open").disabled = $("home-open").disabled = $("folders").disabled = !!view.busy;

  const lane = scene?.lanes?.[view.activeLane];
  $("active-lane-sample").textContent = lane?.sample ?? lane?.name ?? "";
  $("replace-sample").disabled = !!view.busy || !lane?.canReplace;
  renderPanelLayout();
  if (!scene && view.recent) renderRecent(view.recent);
  if (view.scene) renderScene();
  else renderSelection();
  $("edit-range").disabled = !!view.busy || !scene?.range;
  renderBookmarks();
  if (scene) {
    setValue("speed", String(view.speed).replace(/^0\./, "."));
    $("cycle").checked = view.cycle;
    $("follow").checked = !!view.follow;
    followPlayhead();
    $("audition").checked = view.audition;
    if (wasPlaying !== view.playing) {
      $("play-icon").src = view.playing ? "studio-assets/icons/Pause.svg" : "studio-assets/icons/Play.svg";
      $("play-label").textContent = view.playing ? "Pause" : "Play";
    }
    $("position").textContent = clock(view.position);
    for (const button of $("tools").children) button.classList.toggle("active", button.textContent === view.tool);
  }
  renderDialogs();
  window.refreshEditorMenu?.();
  paintSoon();
}

function setValue(id, value) {
  if (document.activeElement !== $(id)) $(id).value = value;
}

function renderRecent(paths) {
  const panel = $("recent");
  if (panel.dataset.paths === JSON.stringify(paths)) return;
  panel.dataset.paths = JSON.stringify(paths);
  panel.replaceChildren(...paths.map((path) => {
    const button = makeButton(nameOf(path), () => act({ kind: "open", path }));
    button.title = path;
    return button;
  }));
}

function renderPanelLayout() {
  if (!view.panels) return;
  const { inspector, controller, laneNames } = view.panels;
  $("editor-dock").dataset.inspector = String(inspector);
  $("editor-arrange").dataset.controller = String(controller);
  $("editor-arrange").dataset.laneNames = String(laneNames);
  $("editor-inspector").hidden = !inspector;
  $("volume-controller-panel").hidden = !controller;
  const browser = scene && view.panels.browser ? (view.classicBrowser ? "classic" : "tree") : "off";
  $("editor-dock").dataset.browser = browser;
  $("tree-browser-host").hidden = browser !== "tree";
  $("classic-browser-host").hidden = browser !== "classic";
  if (browser !== "off") {
    const host = $(browser + "-browser-host");
    const panel = $("sample-browser");
    if (panel.parentNode !== host) host.append(panel);
    renderSampleBrowser();
  }
}

function reorderSampleFolder(from, to, after) {
  if (view.busy || drag || from === to || !sampleFolderOrder.includes(from) || !sampleFolderOrder.includes(to)) return;
  sampleFolderOrder = sampleFolderOrder.filter(group => group !== from);
  sampleFolderOrder.splice(sampleFolderOrder.indexOf(to) + (after ? 1 : 0), 0, from);
  renderLanes();
  paintSoon();
  saveLaneLayout();
}

function sampleFolderHeader(group) {
  const title = sampleFolderNames[group];
  const header = document.createElement("div");
  header.className = "lane sample-folder";
  header.style.height = scene.rowHeight + "px";
  header.draggable = true;

  const collapsed = collapsedFolders.has(group);
  const disclosure = makeButton((collapsed ? "▸ " : "▾ ") + title, () => {
    if (view.busy || drag) return;
    if (!collapsedFolders.delete(group)) collapsedFolders.add(group);
    renderLanes();
    paintSoon();
    saveLaneLayout();
  });
  disclosure.className = "name";
  disclosure.title = "Alt+Up or Alt+Down to move this folder";
  disclosure.setAttribute("aria-expanded", !collapsed);
  disclosure.addEventListener("keydown", event => {
    if (!event.altKey || !["ArrowUp", "ArrowDown"].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const headers = laneRows.filter(row => typeof row === "string");
    const after = event.key === "ArrowDown";
    const next = headers[headers.indexOf(group) + (after ? 1 : -1)];
    if (!next) return;
    reorderSampleFolder(group, next, after);
    $("lanes").children[laneRows.indexOf(group)]?.children[0]?.focus?.();
  });

  const members = scene.lanes.filter(lane => lane.group === group);
  const mix = (label, operation, flag) => {
    const button = makeButton(label, () => {
      if (!view.busy && !drag) act({ kind: "group_mix", set: group, operation });
    });
    button.setAttribute("aria-label", (operation === "mute" ? "Mute " : "Solo ") + title);
    button.setAttribute("aria-pressed", members.every(lane => lane[flag]));
    return button;
  };
  header.append(disclosure, mix("M", "mute", "muted"), mix("S", "solo", "solo"));
  header.addEventListener("dragstart", event => {
    event.dataTransfer.setData(sampleFolderDragType, group);
    event.dataTransfer.effectAllowed = "move";
  });
  header.addEventListener("dragover", event => {
    if (event.dataTransfer.types.includes(sampleFolderDragType)) event.preventDefault();
  });
  header.addEventListener("drop", event => {
    const from = event.dataTransfer.getData(sampleFolderDragType);
    if (!from) return;
    event.preventDefault();
    const rect = header.getBoundingClientRect();
    reorderSampleFolder(from, group, event.clientY > rect.y + rect.height / 2);
  });
  return header;
}

function renderLanes() {
  const filter = ($("lane-filter").value ?? "").trim().toLowerCase();
  const eligible = [];
  const families = new Map();
  for (const row of scene.laneOrder ?? scene.lanes.map((_, row) => row)) {
    const lane = scene.lanes[row];
    if (usedLaneKeys !== null && !usedLaneKeys.has(lane.key)) continue;
    if (filter && ![lane.name, lane.sample].some(name => name?.toLowerCase().includes(filter))) continue;
    eligible.push(row);
    if (lane.family == null) continue;
    if (!families.has(lane.family)) families.set(lane.family, []);
    families.get(lane.family).push(row);
  }

  familyParents = new Map();
  previewParents = new Map();
  for (const [family, rows] of families) {
    if (rows.length < 2) continue;
    const parent = rows.find(row => scene.lanes[row].index === 1) ??
      rows.find(row => scene.lanes[row].index === 0) ?? rows[0];
    familyParents.set(parent, family);
    if (collapsedFamilies.has(family)) {
      for (const row of rows) {
        if (row !== parent) previewParents.set(row, parent);
      }
    }
  }

  const rowsByFolder = new Map(sampleFolderOrder.map(group => [group, []]));
  const looseRows = [];
  for (const row of eligible) {
    const entries = rowsByFolder.get(scene.lanes[row].group) ?? looseRows;
    entries.push(row);
  }
  laneRows = [];
  for (const [group, rows] of rowsByFolder) {
    if (!rows.length) continue;
    laneRows.push(group);
    if (collapsedFolders.has(group)) {
      for (const row of rows) previewParents.set(row, group);
    } else {
      laneRows.push(...rows.filter(row => !previewParents.has(row)));
    }
  }
  laneRows.push(...looseRows.filter(row => !previewParents.has(row)));
  lanePositions = new Map(laneRows.map((row, position) => [row, position]));
  const lanePanel = $("lanes");
  const showNames = view.panels?.laneNames !== false;
  const laneKey = JSON.stringify([laneRows, scene.lanes, view.activeLane, scene.rowHeight,
    [...familyParents], showNames, showLaneInfo]);
  if (lanePanel.dataset.state !== laneKey) {
    lanePanel.dataset.state = laneKey;
    lanePanel.replaceChildren(...laneRows.map((row) => {
      if (typeof row === "string") return sampleFolderHeader(row);
      const lane = scene.lanes[row];
      const strip = document.createElement("div");
      strip.className = "lane" + (view.activeLane === row ? " active" : "");
      strip.style.height = scene.rowHeight + "px";
      const laneAction = (operation) => act({ kind: "lane", row, operation });
      const laneLabel = showLaneInfo && lane.index != null
        ? lane.name + (lane.index === 0 ? " · skin" : " · idx " + lane.index)
        : lane.name;
      const name = makeButton(showNames ? laneLabel : "▶", () => laneAction("audition"));
      name.className = "name";
      name.title = (lane.sample ?? laneLabel) + " · Audition";
      name.setAttribute("aria-label", "Audition " + laneLabel);
      const mute = makeButton("M", () => laneAction("mute"));
      mute.setAttribute("aria-label", "Mute " + lane.name);
      mute.setAttribute("aria-pressed", lane.muted);
      const solo = makeButton("S", () => laneAction("solo"));
      solo.setAttribute("aria-label", "Solo " + lane.name);
      solo.setAttribute("aria-pressed", lane.solo);
      if (familyParents.has(row)) {
        const family = familyParents.get(row);
        const collapsed = collapsedFamilies.has(family);
        const disclosure = makeButton(collapsed ? "▸" : "▾", () => {
          if (drag || view.busy) return;
          if (!collapsedFamilies.delete(family)) collapsedFamilies.add(family);
          renderLanes();
          paintSoon();
          saveLaneLayout();
        });
        disclosure.setAttribute("aria-expanded", !collapsed);
        disclosure.setAttribute("aria-label", (collapsed ? "Expand " : "Collapse ") + lane.name + " layers");
        strip.append(disclosure);
      }
      strip.append(name, mute, solo);
      if (lane.canAddLayer) {
        const layer = makeButton("+", () => {
          if (!view.busy) laneAction("add_layer");
        });
        layer.dataset.doc = "";
        layer.disabled = !!view.busy;
        layer.title = "Add numbered " + lane.name + " layer";
        layer.setAttribute("aria-label", layer.title);
        strip.append(layer);
      }
      return strip;
    }));
  }
}

function selectedBookmark() {
  return scene?.bookmarks?.[$("bookmarks").selectedIndex];
}

function renderBookmarks() {
  const list = $("bookmarks");
  const marks = scene?.bookmarks ?? [];
  const state = marks.length ? JSON.stringify([scene.path, marks]) : "";
  if ((list.dataset.state ?? "") !== state) {
    const wasSelected = list.dataset.path === scene?.path ? list.value : "";
    list.replaceChildren(...marks.map(({ time, label }) =>
      new Option(label ? label + " · " + clock(time) : clock(time), String(time))));
    if (marks.some(mark => String(mark.time) === wasSelected)) list.value = wasSelected;
    list.dataset.state = state;
    list.dataset.path = scene?.path ?? "";
  }
  list.disabled = !!view.busy || marks.length === 0;
  for (const id of ["jump-bookmark", "rename-bookmark", "remove-bookmark"]) {
    $(id).disabled = !!view.busy || !selectedBookmark();
  }
}

function drawBookmarks(ctx, marks, start, zoom, width) {
  const visible = marks.map(mark => [mark, (mark.time - start) * zoom])
    .filter(([, x]) => x >= 0 && x <= width);
  ctx.fillStyle = "#79d7dc";
  for (const [i, [mark, x]] of visible.entries()) {
    ctx.fillRect(x, 21, 2, 22);
    ctx.fillRect(x, 21, 9, 5);
    const labelWidth = Math.min(180, (visible[i + 1]?.[1] ?? width) - x - 10);
    if (mark.label && labelWidth > 24) ctx.fillText(mark.label, x + 5, 36, labelWidth);
  }
}

function renderScene() {
  renderLanes();
  $("snap").checked = scene.snap;
  $("object-lines").checked = view.objectLines;
  setValue("snap-mode", scene.snapMode);
  setValue("beats", scene.beats);
  setValue("divisor", scene.divisor);
  setValue("insert", scene.insertVolume);
  setValue("zoom", Math.log10(scene.zoom));
  setValue("lane-height", scene.rowHeight);
  $("protect").checked = scene.protect;
  $("hs").checked = scene.hs;
  renderSelection();
  $("undo").disabled = view.busy || scene.historyPosition === 0;
  $("redo").disabled = view.busy || scene.historyPosition === scene.history.length;
  const difficulty = $("difficulty");
  const pathKey = JSON.stringify([scene.path, scene.difficulties]);
  if (difficulty.dataset.state !== pathKey) {
    difficulty.dataset.state = pathKey;
    const paths = [...new Set([scene.path, ...scene.difficulties])];
    difficulty.replaceChildren(...paths.map((path) => {
      const name = nameOf(path);
      return new Option(name.match(/\[([^\]]+)\]\.osu$/i)?.[1] || name, path);
    }));
    difficulty.value = scene.path;
  }
  const history = $("history");
  const historyKey = JSON.stringify([scene.historyPosition, scene.history]);
  if (history.dataset.state !== historyKey) {
    history.dataset.state = historyKey;
    history.replaceChildren(...["Opened difficulty", ...scene.history].map((label, position) => {
      const button = makeButton(label, () => act({ kind: "history", position }));
      button.classList.toggle("active", position === scene.historyPosition);
      return button;
    }).reverse());
  }
}

function renderSelection() {
  const selection = scene?.selection;
  $("selected").textContent = selection
    ? selection.count + (selection.count === 1 ? " hit" : " hits")
    : "Nothing selected";
  $("selection-fields").hidden = !selection;
  $("selection-fields").disabled = !!view.busy;
  const body = selection?.sliderBody;
  const fields = $("slider-body-fields");
  fields.hidden = !body;
  fields.disabled = !!view.busy;
  if (body) {
    const state = JSON.stringify([scene.path, body]);
    if (fields.dataset.state !== state) {
      $("slider-body-set").value = body.set ?? "";
      $("slider-body-set").disabled = !body.set;
      $("slider-body-whistle").checked = body.whistle;
      $("slider-body-volume").value = body.volume;
      fields.dataset.state = state;
    }
  } else {
    delete fields.dataset.state;
  }
  if (!selection) return;
  setValue("selection-start-input", +selection.start.toFixed(3));
  $("selection-object").textContent = selection.object;
}

function clock(ms) {
  const rounded = Math.max(Math.round(ms), 0);
  const minutes = Math.floor(rounded / 60000);
  const seconds = (rounded % 60000 / 1000).toFixed(3);
  return String(minutes).padStart(2, "0") + ":" + seconds.padStart(6, "0");
}

function prepare(canvas, height = canvas.clientHeight) {
  const width = canvas.clientWidth;
  const scale = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(width * scale));
  const h = Math.max(1, Math.round(height * scale));
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;
  const ctx = canvas.getContext("2d");
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  return { ctx, width, height };
}

function line(ctx, color, x1, y1, x2, y2) {
  ctx.strokeStyle = color;
  ctx.beginPath();
  ctx.moveTo(x1 + .5, y1);
  ctx.lineTo(x2 + .5, y2);
  ctx.stroke();
}

// Divisor palette follows osu!lazer BindableBeatDivisor.cs and OsuColour.cs at
// 131fe72a15581049401a588a13684938d4d0e0c7. Copyright (c) 2025 ppy Pty Ltd.
// MIT licence, see THIRD-PARTY-NOTICES.md.
const snapColours = {
  1: "#ffffff", 2: "#ed1121", 3: "#8866ee", 4: "#66ccff",
  5: "#b3d944", 6: "#eeaa00", 7: "#b3d944", 8: "#ffcc22",
  9: "#b3d944", 12: "#cc6600", 16: "#6644cc",
};

function snapColour(tick, divisor) {
  let remainder = Math.abs(tick % divisor);
  let factor = divisor;
  while (remainder) [factor, remainder] = [remainder, factor % remainder];
  return snapColours[divisor / factor] ?? "#ff0000";
}

function drawTimingGrid(ctx, start, end, zoom, top, bottom, ruler = false) {
  const useSnap = scene.snap && scene.snapMode !== "objects";
  for (const [i, [origin, beat, meter]] of scene.sections.entries()) {
    if (beat <= 0 || !meter) continue;
    const begin = Math.max(start, i ? origin : start);
    const until = Math.min(end, scene.sections[i + 1]?.[0] ?? end);
    if (begin >= until) continue;
    const widthOfBar = beat * meter * zoom;
    const stride = widthOfBar < 10 ? meter * Math.ceil(24 / widthOfBar) :
      beat * zoom < 8 ? meter : 1;

    if (useSnap && scene.beats === 1 && scene.divisor > 1) {
      const step = beat / scene.divisor;
      if (step * zoom >= 6) {
        for (let tick = Math.ceil((begin - origin) / step); origin + tick * step < until; tick++) {
          if (tick % scene.divisor === 0) continue;
          const x = (origin + tick * step - start) * zoom;
          const color = snapColour(tick, scene.divisor);
          line(ctx, ruler ? color : color + "42", x, ruler ? bottom - 5 : top, x, bottom);
        }
      }
    }

    const step = beat * stride;
    for (let n = Math.ceil((begin - origin) / step); origin + n * step < until; n++) {
      const tick = n * stride;
      const x = (origin + n * step - start) * zoom;
      const bar = !scene.editorBars?.length && tick % meter === 0;
      const longSnap = useSnap && scene.beats > 1 && tick % scene.beats === 0;
      const color = ruler ? snapColours[1] : bar || longSnap ? "#4b6276" : "#29384a";
      line(ctx, color, x, ruler ? bottom - (bar || longSnap ? 9 : 7) : top, x, bottom);
    }
  }
  for (const [i, section] of (scene.editorBars ?? []).entries()) {
    const begin = Math.max(start, section.start);
    const until = Math.min(end, scene.editorBars[i + 1]?.start ?? end);
    const length = section.beat * section.numerator;
    const stride = Math.max(1, Math.ceil((ruler ? 60 : 10) / (length * zoom)));
    const step = length * stride;
    if (begin >= until || !Number.isFinite(step) || step <= 0) continue;
    for (let n = Math.ceil((begin - section.start) / step); section.start + n * step < until; n++) {
      const x = (section.start + n * step - start) * zoom;
      line(ctx, "#607189", x, ruler ? 3 : top, x, bottom);
      if (ruler) {
        ctx.fillStyle = "#c6d3e4";
        ctx.fillText(String(section.bar + n * stride), x + 5, 14);
      }
    }
  }
}

function waveform(ctx, width, center, height, start, span) {
  const bins = scene.waveform;
  if (!bins.length || !view.duration) return;
  for (const [channel, color] of [[0, "#334a63"], [1, "#47618b"], [2, "#657ab5"], [3, "#7aaac6"]]) {
    ctx.fillStyle = color;
    for (let x = 0; x < width; x += 2) {
      const a = Math.floor((start + x * span / width) / view.duration * bins.length);
      const b = Math.min(bins.length, Math.ceil((start + (x + 2) * span / width) / view.duration * bins.length));
      let peak = 0;
      for (let i = Math.max(0, a); i < b; i++) peak = Math.max(peak, bins[i][channel]);
      const extent = limit(peak, 0, 1) * height;
      ctx.fillRect(x, center - extent, 2, extent * 2);
    }
  }
}

function visibleWarningMarkers(problems, path, start, zoom, width) {
  const visible = problems.filter(problem => {
    const x = (problem.time - start) * zoom;
    return problem.timeline === path && x >= 0 && x <= width;
  }).sort((first, second) => first.time - second.time);
  const markers = [];
  for (const problem of visible) {
    const x = (problem.time - start) * zoom;
    const last = markers.at(-1);
    if (last && x - last.x < 8) last.problems.push(problem);
    else markers.push({ x, problems: [problem] });
  }
  return markers;
}

function paintSoon() {
  if (!frame) frame = requestAnimationFrame(() => { frame = 0; paint(); });
}

function fillHitDiamond(ctx, x, y) {
  ctx.beginPath();
  ctx.moveTo(x, y - 6);
  ctx.lineTo(x + 6, y);
  ctx.lineTo(x, y + 6);
  ctx.lineTo(x - 6, y);
  ctx.closePath();
  ctx.fill();
}

function paint() {
  if (!scene || $("workspace").hidden) return;
  const rowHeight = scene.rowHeight;
  const contentHeight = Math.max($("scroll").clientHeight, laneRows.length * rowHeight);
  const scrollTop = $("scroll").scrollTop;
  const canvasHeight = Math.min(contentHeight, $("scroll").clientHeight);
  grid.style.height = canvasHeight + "px";
  const g = prepare(grid, canvasHeight);
  const r = prepare(ruler);
  const v = $("volume-controller-panel").hidden ? null : prepare(controller);
  const o = prepare(overview);
  const { start, zoom } = { start: scene.viewStart, zoom: scene.zoom };
  const xOf = (time) => (time - start) * zoom;
  const end = start + g.width / zoom;
  const range = scene.range;
  if (v) {
    v.ctx.fillStyle = "#172230";
    v.ctx.fillRect(0, 0, v.width, v.height);
    line(v.ctx, "#304054", 0, v.height / 2, v.width, v.height / 2);
  }
  for (let row = Math.floor(scrollTop / rowHeight); row <= Math.ceil((scrollTop + g.height) / rowHeight); row++) {
    const y = row * rowHeight - scrollTop;
    const lane = laneRows[row];
    g.ctx.fillStyle = lane === view.activeLane ? "#1d3440" : row % 2 ? "#192532" : "#15212e";
    if (lane === undefined) g.ctx.fillStyle = "#121a24";
    else if (typeof lane === "string") g.ctx.fillStyle = "#233246";
    g.ctx.fillRect(0, y, g.width, rowHeight);
    if (lane !== undefined) line(g.ctx, "#304054", 0, y + rowHeight - .5, g.width, y + rowHeight - .5);
  }
  if (range) {
    g.ctx.fillStyle = "#4eafbb35";
    const width = xOf(range[1]) - xOf(range[0]);
    if (markedRange?.path === scene.path) {
      for (const row of markedRange.rows) {
        const position = lanePositions.get(row);
        if (position !== undefined) g.ctx.fillRect(xOf(range[0]), position * rowHeight - scrollTop, width, rowHeight);
      }
    } else {
      g.ctx.fillRect(xOf(range[0]), 0, width, g.height);
    }
  }
  drawTimingGrid(g.ctx, start, end, zoom, 0, g.height);
  let lastObjectPixel = null;
  for (const [time, volume, object, hits] of scene.moments) {
    const x = xOf(time);
    if (x < -8 || x > g.width + 8) continue;
    const objectPixel = Math.round(x);
    if (object && view.objectLines && objectPixel !== lastObjectPixel) {
      line(g.ctx, "#69809965", objectPixel, 0, objectPixel, g.height);
      lastObjectPixel = objectPixel;
    }
    const colour = volumeColour(volume);
    if (v) {
      const volumeHeight = limit(volume, 0, 100) / 100 * (v.height - 3);
      v.ctx.fillStyle = colour;
      v.ctx.fillRect(x - 1, v.height - volumeHeight, 2, volumeHeight);
    }
    if (previewParents.size) {
      for (const [row, , listenMuted] of hits) {
        const parent = previewParents.get(row);
        if (parent === undefined) continue;
        const y = (lanePositions.get(parent) + .5) * rowHeight - scrollTop;
        if (y < -8 || y > g.height + 8) continue;
        g.ctx.save();
        g.ctx.globalAlpha *= .3;
        g.ctx.fillStyle = listenMuted ? "#637484" : colour;
        fillHitDiamond(g.ctx, x, y);
        g.ctx.restore();
      }
    }
    for (const [row, selected, listenMuted] of hits) {
      const visibleRow = lanePositions.get(row);
      if (visibleRow === undefined) continue;
      const y = (visibleRow + .5) * rowHeight - scrollTop;
      if (y < -8 || y > g.height + 8) continue;
      g.ctx.fillStyle = listenMuted ? "#637484" : colour;
      fillHitDiamond(g.ctx, x, y);
      if (selected) {
        g.ctx.lineWidth = 2;
        g.ctx.strokeStyle = "#e5fbff";
        g.ctx.stroke();
        g.ctx.lineWidth = 1;
      }
    }
  }
  if (drag?.surface === grid && drag.lastPoint && ["Select", "Range"].includes(view.tool)) {
    const x = drag.firstPoint.x;
    const y = drag.firstPoint.y - scrollTop;
    const width = drag.lastPoint.x - x;
    const height = drag.lastPoint.y - drag.firstPoint.y;
    if ((view.tool === "Range" || !drag.hit) && Math.hypot(width, height) > 3) {
      g.ctx.fillStyle = "#74d3e02f";
      g.ctx.fillRect(x, y, width, height);
      g.ctx.strokeStyle = "#87dbe7";
      g.ctx.strokeRect(x, y, width, height);
    }
  }
  if (v && scene.rampPreview?.length) {
    v.ctx.strokeStyle = "#a5feff";
    v.ctx.beginPath();
    for (const [i, [time, volume]] of scene.rampPreview.entries()) {
      const x = xOf(time);
      const y = v.height - volume / 100 * (v.height - 3);
      if (i === 0) v.ctx.moveTo(x, y);
      else v.ctx.lineTo(x, y);
    }
    v.ctx.stroke();
  }
  r.ctx.fillStyle = "#192532";
  r.ctx.fillRect(0, 0, r.width, r.height);
  waveform(r.ctx, r.width, (r.height + 20) / 2, (r.height - 21) / 2, start, r.width / zoom);
  r.ctx.fillStyle = "#1f2b3b";
  r.ctx.fillRect(0, 0, r.width, 21);
  r.ctx.font = "11px Consolas, monospace";
  drawTimingGrid(r.ctx, start, end, zoom, 0, 21, true);
  const rulerSteps = [25, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000, 60000];
  const labelStep = rulerSteps.find((step) => step * zoom >= 76) ?? 120000;
  for (let time = Math.ceil(start / labelStep) * labelStep; !scene.editorBars?.length && time < end; time += labelStep) {
    const x = xOf(time);
    line(r.ctx, "#607189", x, 3, x, 21);
    r.ctx.fillStyle = "#c6d3e4";
    r.ctx.fillText(clock(time), x + 5, 14);
  }
  if (view.cycle) {
    r.ctx.fillStyle = "#4eafbb46";
    r.ctx.fillRect(xOf(view.left), 0, xOf(view.right) - xOf(view.left), 20);
  }
  for (const time of [view.left, view.right]) {
    line(r.ctx, "#8bcaac", xOf(time), 0, xOf(time), r.height);
  }
  drawBookmarks(r.ctx, scene.bookmarks ?? [], start, zoom, r.width);
  line(g.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), g.height);
  line(r.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), r.height);
  rulerWarningMarkers = visibleWarningMarkers(view.outputProblems ?? [], scene.path, start, zoom, r.width);
  r.ctx.fillStyle = "#f0aa70";
  for (const marker of rulerWarningMarkers) {
    r.ctx.fillRect(marker.x - 3, r.height - 9, 6, 6);
  }
  if (v) line(v.ctx, "#64e1e8", xOf(view.position), 0, xOf(view.position), v.height);
  o.ctx.fillStyle = "#192532";
  o.ctx.fillRect(0, 0, o.width, o.height);
  waveform(o.ctx, o.width, o.height / 2, o.height / 2 - 2, 0, view.duration);
  if (view.duration > 0) {
    const overviewX = start * o.width / view.duration;
    const overviewWidth = (g.width / zoom) * o.width / view.duration;
    o.ctx.fillStyle = "#72e3ed3b";
    o.ctx.fillRect(overviewX, 0, overviewWidth, o.height);
    o.ctx.strokeStyle = "#6ebdc7";
    o.ctx.strokeRect(overviewX, .5, overviewWidth, o.height - 1);
    const playX = view.position * o.width / view.duration;
    line(o.ctx, "#a5feff", playX, 0, playX, o.height);
  }
}

function pointer(event, phase) {
  const rect = drag.surface.getBoundingClientRect();
  const x = event.clientX - rect.x;
  const valueLane = drag.surface === controller;
  const y = event.clientY - rect.y + (valueLane || drag.panning ? 0 : $("scroll").scrollTop);
  const visibleRow = limit(Math.floor(y / scene.rowHeight), 0, drag.rows.length - 1);
  const lane = drag.rows[visibleRow];
  if (typeof lane === "number") drag.lastRow = lane;
  const row = valueLane || drag.panning ? view.activeLane : drag.lastRow;
  let ms = drag.start + x / drag.zoom;
  if (phase === "down" && !valueLane && ["Select", "Erase", "Mute", "Ramp"].includes(view.tool)) {
    let best = 6.5 / drag.zoom;
    for (const [time, , , hits] of scene.moments) {
      if (Math.abs(time - ms) < best && hits.some(([lane]) => row === lane)) {
        best = Math.abs(time - ms);
        drag.hit = time;
      }
    }
    if (drag.hit != null) ms = drag.hit;
  }
  return {
    kind: "pointer", phase, ms, row, x, y,
    value: valueLane ? limit((rect.height - y) / Math.max(1, rect.height - 3) * 100, 1, 100) : null,
    shift: event.shiftKey, ctrl: event.ctrlKey || event.metaKey, alt: event.altKey,
  };
}

function revealLayerPreview(row, time, pointerY) {
  const family = familyParents.get(row);
  const folder = typeof row === "string" && collapsedFolders.has(row);
  if (!folder && !collapsedFamilies.has(family)) return false;
  const tolerance = 6.5 / scene.zoom;
  let nearest;
  let distance = tolerance;
  for (const [at, , , hits] of scene.moments) {
    const offset = Math.abs(at - time);
    if (offset > tolerance) continue;
    if (hits.some(([hitRow]) => hitRow === row)) return false;
    for (const [hitRow] of hits) {
      if (previewParents.get(hitRow) !== row || offset > distance) continue;
      nearest = hitRow;
      distance = offset;
    }
  }
  if (nearest === undefined) return false;
  if (folder) {
    collapsedFolders.delete(row);
    collapsedFamilies.delete(scene.lanes[nearest].family);
  } else {
    collapsedFamilies.delete(family);
  }
  renderLanes();
  $("scroll").scrollTop = lanePositions.get(nearest) * scene.rowHeight - pointerY;
  paintSoon();
  saveLaneLayout();
  return true;
}

function scrollGrab(point) {
  if (drag.panning) $("scroll").scrollTop = drag.scrollTop + drag.firstPoint.y - point.y;
}

for (const surface of [grid, controller]) {
  surface.addEventListener("pointerdown", (event) => {
    if (!scene || view.busy || drag || event.button !== 0) return;
    const panning = surface === grid && view.tool === "Grab" && !event.altKey;
    if (surface === grid && !panning) {
      const rect = grid.getBoundingClientRect();
      const y = event.clientY - rect.y;
      const row = laneRows[Math.floor((y + $("scroll").scrollTop) / scene.rowHeight)];
      if (row === undefined) return;
      const time = scene.viewStart + (event.clientX - rect.x) / scene.zoom;
      if (revealLayerPreview(row, time, y)) {
        event.preventDefault();
        return;
      }
      if (typeof row !== "number") return;
    }
    event.preventDefault();
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    surface.setPointerCapture(event.pointerId);
    drag = { surface, id: event.pointerId, start: scene.viewStart, zoom: scene.zoom,
      rows: laneRows, last: 0, panning, scrollTop: $("scroll").scrollTop };
    drag.firstPoint = pointer(event, "down");
    drag.markingRange = surface === grid && view.tool === "Range";
    act(drag.firstPoint);
  });
  surface.addEventListener("pointermove", (event) => {
    if (event.pointerId !== drag?.id || performance.now() - drag.last < 32 || requests > 2) return;
    drag.last = performance.now();
    drag.lastPoint = pointer(event, "move");
    scrollGrab(drag.lastPoint);
    act(drag.lastPoint);
  });
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
    surface.addEventListener(type, (event) => {
      if (event.pointerId !== drag?.id) return;
      const released = type === "pointerup";
      const end = pointer(event, released ? "up" : "cancel");
      const moved = released && Math.hypot(end.x - drag.firstPoint.x, end.y - drag.firstPoint.y) > 3;
      if (released && drag.markingRange) {
        const first = drag.rows.indexOf(drag.firstPoint.row);
        const last = drag.rows.indexOf(end.row);
        markedRange = moved && first >= 0 && last >= 0 ? {
          path: scene.path,
          rows: drag.rows.slice(Math.min(first, last), Math.max(first, last) + 1)
            .filter(row => typeof row === "number"),
        } : null;
      }
      if (moved) {
        scrollGrab(end);
        act({ ...end, phase: "move" });
      }
      act(end);
      drag = null;
    });
  }
}
$("scroll").addEventListener("scroll", paintSoon);
new ResizeObserver(paintSoon).observe($("scroll"));
ruler.addEventListener("pointermove", (event) => {
  const bounds = ruler.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;
  const marker = y >= bounds.height - 13 ? rulerWarningMarkers.find(marker => Math.abs(marker.x - x) <= 7) : null;
  const nearby = marker?.problems ?? [];
  const lines = nearby.slice(0, 8).map(describeOutputProblem);
  if (nearby.length > lines.length) lines.push("…and " + (nearby.length - lines.length) + " more.");
  ruler.title = lines.join("\n");
});
ruler.addEventListener("pointerleave", () => { ruler.title = ""; });
ruler.addEventListener("pointerdown", (event) => {
  if (!scene) return;
  const x = event.clientX - ruler.getBoundingClientRect().x;
  act({ kind: "seek", ms: limit(scene.viewStart + x / scene.zoom, 0, view.duration) }, false);
});
overview.addEventListener("pointerdown", (event) => {
  if (!scene) return;
  const position = (event.clientX - overview.getBoundingClientRect().x) / overview.clientWidth * view.duration;
  changeView(position - grid.clientWidth / scene.zoom / 2, scene.zoom);
});

function timelineStart(start, zoom) {
  return limit(start, 0, view.duration - grid.clientWidth / zoom);
}

function followPlayhead() {
  if (!view.follow || !view.playing || view.busy || drag || !grid.clientWidth) return;
  const visible = grid.clientWidth / scene.zoom;
  if (view.position >= scene.viewStart && view.position <= scene.viewStart + visible * .98) return;
  const start = timelineStart(view.position - visible * .02, scene.zoom);
  if (start !== scene.viewStart) changeView(start, scene.zoom);
}

function resizeLaneHeaders(height) {
  for (const lane of $("lanes").children) lane.style.height = height + "px";
}

function changeView(start, zoom, rowHeight = scene.rowHeight, rowAnchorY = 0) {
  zoom = limit(zoom, .002, 3);
  rowHeight = limit(rowHeight, 20, 65);
  const path = scene.path;
  return run(async () => {
    if (!scene || scene.path !== path) return;
    const current = scene;
    const previousStart = current.viewStart;
    const previousZoom = current.zoom;
    const previousHeight = current.rowHeight;
    const scroll = $("scroll");
    const previousScroll = scroll.scrollTop;
    current.viewStart = timelineStart(start, zoom);
    current.zoom = zoom;
    current.rowHeight = rowHeight;
    if (rowHeight !== previousHeight) {
      resizeLaneHeaders(rowHeight);
      scroll.scrollTop = (previousScroll + rowAnchorY) * rowHeight / previousHeight - rowAnchorY;
    }
    setValue("zoom", Math.log10(zoom));
    setValue("lane-height", rowHeight);
    paintSoon();

    try {
      await update({ kind: "view", start: current.viewStart, zoom, row_height: rowHeight }, false);
    } catch (error) {
      current.viewStart = previousStart;
      current.zoom = previousZoom;
      current.rowHeight = previousHeight;
      if (rowHeight !== previousHeight) {
        resizeLaneHeaders(previousHeight);
        scroll.scrollTop = previousScroll;
      }
      setValue("zoom", Math.log10(previousZoom));
      setValue("lane-height", previousHeight);
      paintSoon();
      throw error;
    }
  });
}

$("scroll").addEventListener("wheel", (event) => {
  if (!scene || !event.ctrlKey || !event.shiftKey) return;
  event.preventDefault();
  if (requests > 1 || view.busy || drag || !event.deltaY) return;
  const height = Math.round(scene.rowHeight * (event.deltaY < 0 ? 1.15 : 1 / 1.15));
  const anchor = event.clientY - $("scroll").getBoundingClientRect().top;
  changeView(scene.viewStart, scene.zoom, height, anchor);
}, { passive: false });

grid.addEventListener("wheel", (event) => {
  if (!scene || (event.ctrlKey && event.shiftKey) || !(event.ctrlKey || event.shiftKey)) return;
  event.preventDefault();
  if (requests > 1) return;
  const x = event.clientX - grid.getBoundingClientRect().x;
  const zoom = event.ctrlKey ? limit(scene.zoom * Math.exp(-event.deltaY * .003), .002, 3) : scene.zoom;
  const start = event.ctrlKey ? scene.viewStart + x * (1 / scene.zoom - 1 / zoom) :
    scene.viewStart + (event.deltaX || event.deltaY) / scene.zoom;
  changeView(start, zoom);
}, { passive: false });

function modal(kind, title, content, buttons, cancel = null) {
  if (modalKind === kind && dialog.open) return;
  modalKind = kind;
  modalClose = cancel;
  $("dialog-title").textContent = title;
  $("dialog-body").replaceChildren(...content);
  $("dialog-actions").replaceChildren(...buttons.map(([label, task, primary]) => {
    const button = makeButton(label, task);
    if (primary) button.className = "primary";
    return button;
  }));
  if (!dialog.open) dialog.showModal();
}
function closeModal() {
  modalKind = "";
  modalClose = null;
  dialog.close();
}
dialog.addEventListener("cancel", (event) => {
  event.preventDefault();
  if (modalClose) modalClose();
  else closeModal();
});

function paragraph(value) {
  const node = document.createElement("p");
  node.textContent = value;
  return node;
}

function describeOutputProblem(problem) {
  const time = (problem.time < 0 ? "−" : "") + clock(Math.abs(problem.time));
  return time + " · " + nameOf(problem.target) + " · " + problem.message;
}

async function showOutputProblem(index) {
  const problems = view.outputProblems ?? [];
  if (!problems.length || view.busy) return;
  outputProblemIndex = index % problems.length;
  const problem = problems[outputProblemIndex];
  if (scene && scene.path === problem.timeline) {
    await act({ kind: "seek", ms: problem.time }, false);
    if (scene && scene.path === problem.timeline) {
      await changeView(problem.time - grid.clientWidth / scene.zoom / 2, scene.zoom);
    }
  }
  closeModal();
  modal("output-problem", "Problem " + (outputProblemIndex + 1) + " of " + problems.length,
    [paragraph(describeOutputProblem(problem))], [
      ["Close", closeModal],
      ...(problems.length > 1 ? [["Next problem", () => showOutputProblem(outputProblemIndex + 1), true]] : []),
    ]);
}

function renderDialogs() {
  if (view.error) {
    const dismiss = () => { closeModal(); act({ kind: "dismiss_error" }); };
    modal("error", "Editor message", [paragraph(view.error)], [["OK", dismiss, true]], dismiss);
  } else if (view.saveProblemReview) {
    const dismiss = () => { closeModal(); return act({ kind: "dismiss_save_problem_review" }); };
    const first = async () => { await dismiss(); await showOutputProblem(0); };
    const save = () => { closeModal(); act({ kind: "save" }); };
    modal("save-problems", "Review problems before saving",
      (view.outputProblems ?? []).map(problem => paragraph(describeOutputProblem(problem))),
      [["Cancel", dismiss], ["Show first problem", first], ["Continue to Save", save, true]], dismiss);
  } else if (view.pendingHome) {
    const resolve = (choice) => { closeModal(); act({ kind: "home_choice", choice }); };
    modal("pending-home", "Save changes before going Home?", [paragraph("The open difficulty has unsaved changes.")],
      [["Cancel", () => resolve("cancel")], ["Discard", () => resolve("discard")], ["Save", () => resolve("save"), true]],
      () => resolve("cancel"));
  } else if (view.pendingOpen) {
    const resolve = (choice) => { closeModal(); act({ kind: "pending", choice }); };
    modal("pending", "Save changes before opening?", [paragraph("The open difficulty has unsaved changes.")],
      [["Cancel", () => resolve("cancel")], ["Discard", () => resolve("discard")], ["Save", () => resolve("save"), true]],
      () => resolve("cancel"));
  } else if (view.revert) {
    const cancel = () => { closeModal(); act({ kind: "cancel_revert" }); };
    modal("revert", "Revert to saved difficulty?", [paragraph("This reloads the saved difficulty from disk.")],
      [["Cancel", cancel], ["Revert", () => { closeModal(); act({ kind: "confirm_revert" }); }, true]], cancel);
  } else if (view.review) {
    const answer = (apply) => { closeModal(); act({ kind: "output", apply }); };
    const heading = document.createElement("h3");
    heading.textContent = "Files this action will write";
    const files = view.review.files.flatMap((file) => [
      paragraph((file.replace ? "Replace: " : "Create: ") + file.path),
      paragraph(file.changed + " objects changed · " + file.added + " added · " + file.removed +
        " removed · " + file.timing + " timing lines affected"),
    ]);
    const details = view.review.details.map(paragraph);
    const mixes = (view.review.mixes ?? []).map((path, index) => {
      const row = document.createElement("div");
      row.className = "row";
      const label = paragraph(nameOf(path));
      label.title = path;
      row.append(label, makeButton("Audition mix", () => act({ kind: "audition_output_mix", index }, false)));
      return row;
    });
    const mixHeading = document.createElement("h3");
    mixHeading.textContent = "Sample mixes";
    const mixSection = mixes.length ? [mixHeading, ...mixes] : [];
    const combinations = view.review.combinations ?? [];
    const includeSliderAudio = Boolean(view.review.includeSliderAudio);
    const choices = combinations.length ? [["Edit merge rules…", () => {
      closeModal();
      run(async () => {
        await update({ kind: "output", apply: false });
        await openMergeSettings(combinations, true, includeSliderAudio);
      });
    }]] : [];
    modal("review", view.review.label, [heading, ...files, ...mixSection, ...details],
      [["Cancel", () => answer(false)], ...choices, ["Write files", () => answer(true), true]], () => answer(false));
  } else if (view.editDialog && scene) {
    showTimelineEdit(view.editDialog);
  } else if (view.firstLaunch) {
    const choose = (name) => act({ kind: "key_preset", name });
    modal("key-preset", "Choose your keyboard preset", [
      paragraph("DAW-like uses the editor’s default timeline shortcuts."),
      paragraph("osu! lets you seek through the beat grid with the arrow keys."),
    ], [["DAW-like", () => choose("DAW"), true], ["osu!", () => choose("osu!")]], () => {});
  } else if (["error", "pending", "pending-home", "revert", "review", "save-problems", "key-preset", "timeline-edit"].includes(modalKind)) {
    closeModal();
  }
}

async function openMergeSettings(combinations = [], regenerate = false, includeSliderAudio = false) {
  if (!scene || view.busy) return;
  const result = await invoke("editor_export_settings");
  if (!result || result[0] !== scene?.path) return;
  const [map, current] = result;
  const reviewedPairing = current.PairSamples;
  let value = current;
  const form = document.createElement("div");
  const status = paragraph("");
  status.setAttribute("role", "status");
  let priorityIndex = 0;
  function renderSettings() {
    const paired = document.createElement("input");
    paired.type = "checkbox";
    paired.checked = value.PairSamples;
    paired.addEventListener("change", () => {
      value.PairSamples = paired.checked;
      priorityIndex = 0;
      renderSettings();
    });
    const priority = document.createElement("select");
    priority.setAttribute("aria-label", "Sample slot priority");
    priority.replaceChildren(...value.Priorities.map((slot, i) => new Option((i + 1) + ". " + slot, i)));
    priority.value = String(priorityIndex);
    const move = step => {
      const from = Number(priority.value);
      const to = from + step;
      if (to < 0 || to >= value.Priorities.length) return;
      [value.Priorities[from], value.Priorities[to]] = [value.Priorities[to], value.Priorities[from]];
      priorityIndex = to;
      renderSettings();
    };
    const order = document.createElement("div");
    order.className = "row";
    order.append(priority, makeButton("Move up", () => move(-1)), makeButton("Move down", () => move(1)));
    const nodes = [
      labeled("Pair samples by hitsound role", paired),
      paragraph("Priority chooses the output sample slot. A merge rule can override the priority for its combination."),
      order,
    ];
    if (reviewedPairing === value.PairSamples && combinations.length) {
      const combo = document.createElement("select");
      combo.setAttribute("aria-label", "Sample combination");
      combo.replaceChildren(...combinations.map(({ layers, time }, i) =>
        new Option(layers.join(" + ") + " · " + clock(time), i)));
      combo.value = "0";
      const slot = document.createElement("select");
      slot.setAttribute("aria-label", "Override output slot");
      const displayRule = () => {
        const group = combinations[Number(combo.value)];
        slot.replaceChildren(new Option("Use priority", ""),
          ...value.Priorities.map(value => new Option(value, value)));
        slot.value = value.Rules[group.key] ?? "";
      };
      combo.addEventListener("change", displayRule);
      slot.addEventListener("change", () => {
        const { key } = combinations[Number(combo.value)];
        if (slot.value) value.Rules[key] = slot.value;
        else delete value.Rules[key];
      });
      displayRule();
      nodes.push(labeled("Combination", combo), labeled("Output slot", slot));
    }
    form.replaceChildren(...nodes);
  }
  renderSettings();
  const load = async () => {
    try {
      const loaded = await invoke("pick_merge_preset");
      if (!loaded || modalKind !== "merge-settings" || scene?.path !== map) return;
      value = loaded;
      priorityIndex = 0;
      renderSettings();
      status.textContent = "Loaded " + Object.keys(value.Rules).length + " combination rules.";
    } catch (error) {
      status.textContent = String(error);
    }
  };
  const save = async () => {
    try {
      if (await invoke("save_merge_preset", { value })) status.textContent = "Preset saved.";
    } catch (error) {
      status.textContent = String(error);
    }
  };
  const apply = () => {
    closeModal();
    if (scene?.path !== map) return;
    run(async () => {
      await update({ kind: "export_settings", map, value });
      if (regenerate && !view.error && scene?.path === map) {
        await update({ kind: "generate", include_slider_audio: includeSliderAudio });
      }
    });
  };
  modal("merge-settings", "Merge settings", [form, status],
    [["Cancel", closeModal], ["Load preset…", load], ["Save preset…", save], [regenerate ? "Review again" : "Apply", apply, true]]);
}

$("merge-settings").addEventListener("click", () => run(() => openMergeSettings()));

function sampleTarget() {
  const lane = scene?.lanes?.[view.activeLane];
  return !drag && !view.busy && lane?.canReplace ? { map: scene.path, lane: lane.sourceKey } : null;
}

function pickSample(target) {
  run(async () => show(await invoke("pick_sample", { target })));
}

function mapsetSamples() {
  const samples = new Map();
  for (const [row, lane] of scene.lanes.entries()) {
    if (lane.sample && !samples.has(lane.sample)) samples.set(lane.sample, row);
  }
  return samples;
}

function requestLibraryFolder(folder) {
  if (!browserLibrary || browserLibraryFolders.has(folder)) return;
  const cache = browserLibraryFolders;
  const location = { root: browserLibrary, path: folder };
  cache.set(folder, null);
  run(async () => {
    try {
      cache.set(folder, await invoke("list_library", { location }));
    } catch (error) {
      cache.set(folder, String(error));
    }
    if (cache === browserLibraryFolders && scene && browserSource === "library") {
      browserState = "";
      renderSampleBrowser();
    }
  });
}

function resetBrowserFolder() {
  browserFolder = "";
  browserSample = null;
  browserExpanded.clear();
}

function renderSampleBrowser() {
  if (browserMap !== scene.path) {
    browserMap = scene.path;
    resetBrowserFolder();
  }
  if (browserLibrary !== (view.hitsoundLibrary ?? null)) {
    browserLibrary = view.hitsoundLibrary ?? null;
    browserLibraryFolders = new Map();
    if (browserSource === "library") resetBrowserFolder();
  }
  const library = browserSource === "library";
  const filter = ($("sample-browser-filter").value ?? "").trim().toLowerCase();
  const classic = !!view.classicBrowser;
  const state = JSON.stringify([classic, browserSource, browserLibrary, browserFolder,
    browserSample, filter, [...browserExpanded], view.busy, view.activeLane]);
  if (browserScene === scene && browserState === state) return;
  browserScene = scene;
  browserState = state;
  const samples = library ? [] : mapsetSamples();
  const tree = new Map([["", new Map()]]);
  for (const [path, row] of samples) {
    const parts = path.split("/");
    let folder = "";
    for (const part of parts.slice(0, -1)) {
      const child = folder ? folder + "/" + part : part;
      if (!tree.has(child)) tree.set(child, new Map());
      tree.get(folder).set(child, null);
      folder = child;
    }
    tree.get(folder).set(path, row);
  }
  if (!library && !tree.has(browserFolder)) browserFolder = "";
  const buttons = [];
  const visibleSamples = new Set();
  const addRows = (folder, depth) => {
    let entries;
    if (library) {
      if (!browserLibrary) {
        buttons.push(paragraph("Choose a hitsound library folder in Folders."));
        return;
      }
      requestLibraryFolder(folder);
      const children = browserLibraryFolders.get(folder);
      if (!Array.isArray(children)) {
        buttons.push(paragraph(children || "Loading samples…"));
        return;
      }
      entries = children.map(([name, directory]) => {
        const path = folder ? folder + "/" + name : name;
        return [path, directory ? null : path];
      });
    } else {
      entries = [...tree.get(folder)];
    }
    entries.sort(([left, a], [right, b]) =>
      Number(a !== null) - Number(b !== null) || left.localeCompare(right));
    for (const [path, row] of entries) {
      const directory = row === null;
      const label = nameOf(path);
      if (!directory && !label.toLowerCase().includes(filter)) continue;
      if (!directory) visibleSamples.add(path);
      const open = browserExpanded.has(path);
      const entry = makeButton(label, () => {
        if (directory) {
          if (classic) browserFolder = path;
          else if (!browserExpanded.delete(path)) browserExpanded.add(path);
        } else {
          if (view.busy) return;
          browserSample = path;
          if (library) act({ kind: "preview_library_sample", location: { root: browserLibrary, path } }, false);
          else act({ kind: "lane", row, operation: "preview_sample" }, false);
        }
        renderSampleBrowser();
        $("sample-browser-entries").children[buttons.indexOf(entry)]?.focus?.();
      });
      entry.className = "sample-browser-entry";
      entry.title = path;
      entry.style.paddingInlineStart = (8 + 8 * Math.min(depth, 6)) + "px";
      if (directory) entry.setAttribute("aria-expanded", String(classic ? false : open));
      else {
        entry.setAttribute("aria-pressed", String(browserSample === path));
        entry.disabled = !!view.busy;
      }
      buttons.push(entry);
      if (!classic && directory && open) addRows(path, depth + 1);
    }
  };
  addRows(classic ? browserFolder : "", 0);
  $("sample-browser-entries").replaceChildren(...(buttons.length ? buttons
    : [paragraph(filter ? "No matching samples." : "No samples in this folder.")]));
  $("sample-browser-source").value = browserSource;
  $("sample-browser-up").hidden = !classic;
  $("sample-browser-up").disabled = !browserFolder;
  $("sample-browser-folder").hidden = !classic;
  $("sample-browser-folder").textContent = browserFolder || (library ? "Library" : "Map folder");
  $("sample-browser-folder").title = library ? [browserLibrary, browserFolder].filter(Boolean).join("/") : browserFolder;
  $("sample-browser-assign").disabled = !sampleTarget() || !visibleSamples.has(browserSample);
  $("sample-browser-pick").disabled = !sampleTarget();
}

$("sample-browser-source").addEventListener("change", () => {
  if (!scene) return;
  browserSource = $("sample-browser-source").value;
  resetBrowserFolder();
  if (browserSource === "library") browserLibraryFolders = new Map();
  renderSampleBrowser();
});
$("sample-browser-filter").addEventListener("input", () => { if (scene) renderSampleBrowser(); });
$("sample-browser-up").addEventListener("click", () => {
  browserFolder = browserFolder.split("/").slice(0, -1).join("/");
  renderSampleBrowser();
});
$("sample-browser-close").addEventListener("click", () => act({ kind: "panel", panel: "browser", on: false }));
$("sample-browser-assign").addEventListener("click", () => {
  const target = sampleTarget();
  if (!target || !browserSample || $("sample-browser-assign").disabled) return;
  if (browserSource === "library") {
    const location = { root: browserLibrary, path: browserSample };
    act({ kind: "replace_library_sample", target, location });
  } else {
    act({ kind: "replace_sample", target, path: browserSample, mapset: true });
  }
});
$("sample-browser-pick").addEventListener("click", () => {
  const target = sampleTarget();
  if (target) pickSample(target);
});

$("replace-sample").addEventListener("click", () => {
  const target = sampleTarget();
  if (!target) return;
  const select = document.createElement("select");
  const samples = new Map();
  for (const [path, row] of mapsetSamples()) {
    samples.set(row, path);
    const option = document.createElement("option");
    option.value = String(row);
    option.textContent = path;
    select.append(option);
  }
  const chosen = () => samples.has(Number(select.value)) && select.value !== "" ? Number(select.value) : null;
  const assign = () => {
    const row = chosen();
    if (row === null) return;
    closeModal();
    act({ kind: "replace_sample", target, path: samples.get(row), mapset: true });
  };
  const browse = () => {
    closeModal();
    pickSample(target);
  };
  const available = samples.size > 0;
  modal("replace-sample", "Replace sample on " + scene.lanes[view.activeLane].name,
    available ? [labeled("Mapset sample", select)] : [], [
      ["Cancel", closeModal],
      ...(available ? [["Audition", () => {
        const row = chosen();
        if (row !== null && scene?.path === target.map) {
          act({ kind: "lane", row, operation: "preview_sample" }, false);
        }
      }], ["Replace", assign, true]] : []),
      ["Browse…", browse, !available],
    ]);
});

const open = () => run(async () => show(await invoke("pick_map")));
$("open").addEventListener("click", open);
$("home-open").addEventListener("click", open);
$("go-home").addEventListener("click", () => { if (!drag && !view.busy) act({ kind: "home" }); });
$("home-resume").addEventListener("click", () => { if (!view.busy) act({ kind: "resume" }); });
$("folders").addEventListener("click", () => {
  if (view.busy) return;
  const openFolder = (folder) => {
    closeModal();
    run(() => invoke("open_editor_folder", { folder }));
  };
  const chooseSongsFolder = () => {
    closeModal();
    run(async () => {
      const next = await invoke("pick_songs_folder");
      if (next) show(next);
    });
  };
  const chooseLibrary = () => {
    closeModal();
    run(async () => {
      const next = await invoke("pick_hitsound_library");
      if (!next) return;
      browserSource = "library";
      browserLibraryFolders = new Map();
      resetBrowserFolder();
      browserState = "";
      show(next);
    });
  };
  const actions = [["Cancel", closeModal]];
  if (scene) {
    actions.push(["Open map folder", () => openFolder("map")],
      ["Open project folder", () => openFolder("project")]);
  }
  actions.push(["Choose Songs folder…", chooseSongsFolder]);
  if (view.hitsoundLibrary) actions.push(["Open hitsound library", () => openFolder("library")]);
  actions.push(["Choose hitsound library…", chooseLibrary]);
  modal("folders", "Folders", [], actions);
});
$("ramp-shape").addEventListener("change", () => {
  act({ kind: "ramp_shape", value: $("ramp-shape").value }, false);
});

const toolIcons = {
  Select: "MousePointer2", Draw: "Drumstick", Erase: "Eraser", Mute: "VolumeX",
  Grab: "Hand", Range: "Scan", Ramp: "TrendingUp",
};
for (const [name, glyph] of Object.entries(toolIcons)) {
  const button = makeButton("", () => act({ kind: "tool", name }));
  button.title = name;
  button.ariaLabel = name;
  const icon = document.createElement("img");
  icon.src = "studio-assets/icons/" + glyph + ".svg";
  icon.alt = "";
  icon.className = "button-icon";
  const label = document.createElement("span");
  label.className = "ribbon-tool-label";
  label.textContent = name;
  button.prepend(label);
  button.prepend(icon);
  $("tools").append(button);
}
for (const button of document.querySelectorAll("[data-action]")) {
  button.addEventListener("click", () => act({ kind: button.dataset.action }));
}
for (const button of document.querySelectorAll("[data-command]")) {
  button.addEventListener("click", () => command(button.dataset.command));
}
for (const button of document.querySelectorAll("[data-volume]")) {
  button.addEventListener("click", () => act({ kind: "volume", relative: true, value: +button.dataset.volume }));
}
for (const button of document.querySelectorAll("[data-locator]")) {
  button.addEventListener("click", (event) => act({ kind: "locator", end: button.dataset.locator, jump: event.shiftKey }));
}
for (const id of ["snap", "snap-mode", "beats", "divisor"]) {
  $(id).addEventListener("change", () => {
    act({ kind: "snap", on: $("snap").checked, mode: $("snap-mode").value,
      beats: limit(number("beats", scene.beats), 1, 64), divisor: limit(number("divisor", scene.divisor), 1, 64) });
  });
}
$("time-signatures").addEventListener("click", () => {
  if (!scene || view.busy) return;
  const path = scene.path;
  const input = document.createElement("textarea");
  input.rows = 6;
  input.cols = 28;
  input.setAttribute("aria-label", "Bar and time signature changes");
  input.placeholder = "33 7/8";
  input.value = (scene.timeSignatures ?? []).map(({ bar, numerator, denominator }) =>
    `${bar} ${numerator}/${denominator}`).join("\n");
  const error = paragraph("");
  error.setAttribute("role", "alert");
  modal("time-signatures", "Time signatures", [
    paragraph("Enter one change per line, such as 33 7/8 for 7/8 from bar 33. These changes affect the editor ruler."),
    input,
    error,
  ], [
    ["Cancel", closeModal],
    ["Apply", async () => {
      if (scene?.path !== path) return closeModal();
      const value = [];
      for (const line of input.value.trim().split("\n").filter(line => line.trim())) {
        const parts = /^(\d+)\s+(\d+)\s*\/\s*(\d+)$/.exec(line.trim());
        const [bar, numerator, denominator] = parts?.slice(1).map(Number) ?? [];
        if (!bar || bar > 4294967295 || numerator < 1 || numerator > 32 || ![1, 2, 4, 8, 16].includes(denominator)) {
          error.textContent = "Use bar 1 or later, 1–32 beats, and a beat unit of 1, 2, 4, 8 or 16.";
          return;
        }
        value.push({ bar, numerator, denominator });
      }
      closeModal();
      await act({ kind: "time_signatures", map: path, value });
    }, true],
  ]);
});
toggle("hs", "hs_mode");
toggle("object-lines", "object_lines");
toggle("protect", "protect");
function setLaneHeight(height) {
  if (scene && !view.busy && !drag) changeView(scene.viewStart, scene.zoom, height);
}

$("lane-height-in").addEventListener("click", () => {
  if (scene) setLaneHeight(Math.round(scene.rowHeight * 1.15));
});
$("lane-height-out").addEventListener("click", () => {
  if (scene) setLaneHeight(Math.round(scene.rowHeight / 1.15));
});
$("lane-height").addEventListener("input", () => {
  if (requests < 2) setLaneHeight(+$("lane-height").value);
});
$("lane-height").addEventListener("change", () => setLaneHeight(+$("lane-height").value));
$("collapse-layers").addEventListener("click", () => {
  if (!scene || view.busy || drag) return;
  for (const family of familyParents.values()) collapsedFamilies.add(family);
  renderLanes();
  paintSoon();
  saveLaneLayout();
});
$("lane-filter").addEventListener("input", () => {
  if (!scene || drag) return;
  renderLanes();
  paintSoon();
});
$("used-only").addEventListener("change", () => {
  if (!scene || view.busy || drag) {
    $("used-only").checked = usedLaneKeys !== null;
    return;
  }
  if ($("used-only").checked) {
    // Snapshot used lanes so a row does not disappear when its last hit is moved or erased.
    usedLaneKeys = new Set();
    for (const [, , , hits] of scene.moments) {
      for (const [row] of hits) usedLaneKeys.add(scene.lanes[row].key);
    }
  } else {
    usedLaneKeys = null;
  }
  renderLanes();
  paintSoon();
});
toggle("cycle", "cycle");
toggle("audition", "audition");
$("difficulty").addEventListener("change", () => {
  if (!scene) return;
  const path = $("difficulty").value;
  $("difficulty").value = scene.path;
  if (!view.busy && !drag && path && path !== scene.path) act({ kind: "open", path });
});
$("fit").addEventListener("click", () => {
  if (scene && view.duration > 0 && grid.clientWidth > 0) changeView(0, grid.clientWidth / view.duration);
});
$("follow").addEventListener("change", () => act({ kind: "follow", on: $("follow").checked }, false));
$("zoom").addEventListener("input", () => {
  if (requests < 2) changeView(scene.viewStart, 10 ** +$("zoom").value);
});
$("zoom").addEventListener("change", () => changeView(scene.viewStart, 10 ** +$("zoom").value));
$("speed").addEventListener("change", () => act({ kind: "speed", value: +$("speed").value }));
$("insert").addEventListener("change", () => act({ kind: "insert_volume", value: limit(number("insert", 100), 0, 100) }));
$("volume-mode").addEventListener("change", () => {
  const relative = $("volume-mode").value === "relative";
  $("volume").min = relative ? -100 : 0;
  $("volume").value = relative ? 0 : 100;
});
$("selection-start-edit").addEventListener("submit", (event) => {
  event.preventDefault();
  const field = $("selection-start-input");
  const position = field.valueAsNumber;
  if (!scene?.selection || view.busy || !Number.isFinite(position)) return;
  field.blur();
  act({ kind: "timeline_edit", path: scene.path, edit: { kind: "selection_start", position } });
});
$("selection-volume").addEventListener("submit", (event) => {
  event.preventDefault();
  const value = $("volume").valueAsNumber;
  if (!scene?.selection || view.busy || !Number.isFinite(value)) return;
  const relative = $("volume-mode").value === "relative";
  act({ kind: "volume", value: limit(Math.round(value), relative ? -100 : 0, 100), relative });
});

function input(type, value, min, max, step = 1) {
  const field = document.createElement("input");
  field.type = type;
  field.value = value;
  field.min = min;
  field.max = max;
  field.step = step;
  return field;
}
function showTimelineEdit(kind) {
  if (modalKind === "timeline-edit" && dialog.open) return;
  const path = scene.path;
  const cancel = () => { closeModal(); act({ kind: "cancel_timeline_edit" }); };
  const submit = (edit) => { closeModal(); act({ kind: "timeline_edit", path, edit: { kind, ...edit } }); };
  const fail = paragraph("");
  fail.setAttribute("role", "alert");
  const choice = (options, initial) => {
    const select = document.createElement("select");
    for (const [value, label] of options) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      select.append(option);
    }
    select.value = initial;
    return select;
  };
  const openEdit = (title, controls, apply, label = "Apply") =>
    modal("timeline-edit", title, [...controls, fail], [["Cancel", cancel], [label, apply, true]], cancel);
  if (kind === "quantize") {
    const strength = input("number", view.quantizeStrength ?? 100, 1, 100, "any");
    openEdit("Quantize", [labeled("Strength (%)", strength)], () => {
      const value = strength.valueAsNumber;
      if (Number.isFinite(value) && value >= 1 && value <= 100) {
        submit({ strength: value });
      } else fail.textContent = "Enter a strength from 1% to 100%.";
    }, "Quantize");
  } else if (kind === "repeat") {
    const copies = input("number", 1, 1, 64);
    const gap = input("number", scene.repeatGap ?? 1, 1, 1024);
    openEdit("Repeat", [labeled("Copies", copies), labeled("Gap in snap steps", gap)], () => {
      const n = copies.valueAsNumber, steps = gap.valueAsNumber;
      if (Number.isInteger(n) && n >= 1 && n <= 64 && Number.isInteger(steps) && steps >= 1 && steps <= 1024) {
        submit({ copies: n, gap_steps: steps });
      } else fail.textContent = "Enter 1–64 copies and a gap of 1–1024 steps.";
    }, "Repeat");
  } else if (kind === "range" && scene.range) {
    const operation = choice([["copy", "Copy"], ["move", "Move"], ["delete", "Delete"]], "copy");
    const offset = input("number", +(scene.range[1] - scene.range[0]).toFixed(3), "", "", "any");
    operation.addEventListener("change", () => { offset.disabled = operation.value === "delete"; });
    openEdit("Edit range", [labeled("Operation", operation), labeled("Offset (ms)", offset)], () => {
      const ms = operation.value === "delete" ? 0 : offset.valueAsNumber;
      if (Number.isFinite(ms)) {
        submit({ operation: operation.value, offset: ms, rows: markedRange?.path === path ? markedRange.rows : null });
      } else fail.textContent = "Enter a time offset in milliseconds.";
    });
  } else if (kind === "scale") {
    const scope = choice([["selected", "Selected hits"], ["selected_lanes", "Lanes of selected hits"],
      ["all", "All hits"]], scene.selection ? "selected" : "all");
    const low = input("number", 1, 0, 100);
    const high = input("number", 100, 0, 100);
    const percent = input("number", 100, 0, "", "any");
    openEdit("Scale by volume", [labeled("Applies to", scope), labeled("From volume (%)", low),
      labeled("To volume (%)", high), labeled("Scale by (%)", percent),
      paragraph("A moment has one volume. Scaling it changes every sound at that time.")], () => {
      const min = low.valueAsNumber, max = high.valueAsNumber, scale = percent.valueAsNumber;
      if ([min, max].every(value => Number.isInteger(value) && value >= 0 && value <= 100) &&
          Number.isFinite(scale) && scale >= 0) {
        submit({ scope: scope.value, low: min, high: max, percent: scale });
      } else fail.textContent = "Enter volumes from 0% to 100% and a scale of at least 0%.";
    }, "Scale");
  }
}

function labeled(label, control) {
  const node = document.createElement("label");
  node.append(label, control);
  return node;
}
$("generate").addEventListener("click", () => {
  if (!scene || view.busy) return;
  const sliderAudio = input("checkbox", "", 0, 0);
  sliderAudio.checked = false;
  modal("generate", "Generate hitsound difficulty",
    [labeled("Include slider slide, whistle and tick audio", sliderAudio)],
    [["Cancel", closeModal], ["Review output", () => {
      closeModal();
      act({ kind: "generate", include_slider_audio: sliderAudio.checked });
    }, true]]);
});
$("new").addEventListener("click", () => {
  const name = input("text", "", 0, 0);
  name.placeholder = "Difficulty name";
  const submit = () => {
    if (name.value.trim()) {
      closeModal();
      act({ kind: "new_difficulty", name: name.value.trim() });
    }
  };
  name.addEventListener("keydown", (event) => { if (event.key === "Enter") submit(); });
  modal("new", "New difficulty", [labeled("Name", name)], [["Cancel", closeModal], ["Review output", submit, true]]);
  name.focus();
});
function editBookmark(mark) {
  if (!scene || view.busy) return;
  const path = scene.path;
  const time = mark?.time ?? view.position;
  const label = input("text", mark?.label ?? "", 0, 0);
  const submit = () => {
    closeModal();
    act({ kind: mark ? "bookmark_edit" : "bookmark_add", path, time, label: label.value });
  };
  label.addEventListener("keydown", (event) => { if (event.key === "Enter") submit(); });
  modal("bookmark", mark ? "Rename bookmark" : "Add bookmark",
    [paragraph(clock(time)), labeled("Label", label)],
    [["Cancel", closeModal], [mark ? "Rename" : "Add", submit, true]]);
  label.focus();
}

$("bookmarks").addEventListener("change", renderBookmarks);
$("apply-slider-body").addEventListener("click", () => {
  const body = scene?.selection?.sliderBody;
  const volume = $("slider-body-volume").valueAsNumber;
  if (!body || view.busy || !Number.isInteger(volume) || volume < 0 || volume > 100) return;
  act({ kind: "slider_body", path: scene.path, index: body.index, set: $("slider-body-set").value || null,
    whistle: $("slider-body-whistle").checked, volume });
});
$("add-bookmark").addEventListener("click", () => editBookmark(null));
$("rename-bookmark").addEventListener("click", () => {
  const mark = selectedBookmark();
  if (mark) editBookmark(mark);
});
$("jump-bookmark").addEventListener("click", () => {
  const mark = selectedBookmark();
  if (mark) act({ kind: "bookmark_jump", time: mark.time });
});
$("remove-bookmark").addEventListener("click", () => {
  const mark = selectedBookmark();
  if (mark) act({ kind: "bookmark_edit", path: scene.path, time: mark.time, label: null });
});

$("copy").addEventListener("click", () => {
  const paths = scene.difficulties.filter((path) => path !== scene.path);
  if (!paths.length) {
    modal("copy", "Copy hitsounds", [paragraph("There are no other difficulties in this mapset.")], [["Close", closeModal, true]]);
    return;
  }

  const checkboxes = paths.map(() => input("checkbox", "", 0, 0));
  const choices = paths.map((path, i) => labeled(nameOf(path), checkboxes[i]));
  const leniency = input("number", 5, 0, 1000, "any");
  const mode = document.createElement("select");
  for (const [value, label] of [
    ["overwrite_everything", "Overwrite everything"], ["overwrite_only_defined", "Keep unmatched hits"],
  ]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    mode.append(option);
  }
  mode.value = "overwrite_everything";

  const check = (on) => {
    const box = input("checkbox", "", 0, 0);
    box.checked = on;
    return box;
  };
  const hitsounds = check(true);
  const bodies = check(true);
  const sets = check(true);
  const volumes = check(true);
  const keep5 = check(true);
  const storyboard = check(false);
  const satisfied = check(true);
  const whenever = check(false);
  const mute = check(false);
  const defaultDivisors = "1, 4, 3, 8, 6, 16, 12";
  const defaultMutedDivisors = "4, 3, 8, 6, 16, 12";
  const defaultMinLength = 0.5;
  const maxSampleIndex = 4294967295;
  const divisors = input("text", defaultDivisors, 0, 0);
  const mutedDivisors = input("text", defaultMutedDivisors, 0, 0);
  const minLength = input("number", defaultMinLength, 0, "", "any");
  const mutedIndex = input("number", "", 0, maxSampleIndex);
  mutedIndex.placeholder = "Keep";
  const mutedSet = document.createElement("select");
  for (const [value, label] of [["", "Keep"], ["normal", "Normal"], ["soft", "Soft"], ["drum", "Drum"]]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    mutedSet.append(option);
  }
  mutedSet.value = "";

  const updateEnabled = () => {
    bodies.disabled = !hitsounds.checked;
    keep5.disabled = !volumes.checked;
    satisfied.disabled = whenever.disabled = !storyboard.checked;
    for (const field of [divisors, mutedDivisors, minLength, mutedIndex, mutedSet]) {
      field.disabled = !mute.checked;
    }
  };
  for (const field of [hitsounds, volumes, storyboard, mute]) field.addEventListener("change", updateEnabled);
  updateEnabled();

  const positiveDivisors = (text) => {
    const tokens = text.split(/[,\s]+/).filter(Boolean);
    if (tokens.some((value) => !/^\d+$/.test(value) || Number(value) < 1 || Number(value) > 4294967295)) return null;
    return tokens.map(Number);
  };
  const error = paragraph("");
  error.setAttribute("role", "alert");
  const submit = () => {
    const targets = paths.filter((_, i) => checkboxes[i].checked);
    if (!targets.length) {
      error.textContent = "Select at least one target difficulty.";
      return;
    }
    const tolerance = leniency.valueAsNumber;
    if (!Number.isFinite(tolerance) || tolerance < 0 || tolerance > 1000) {
      error.textContent = "Enter a timing tolerance from 0 to 1000 ms.";
      return;
    }
    const beatDivisors = positiveDivisors(mute.checked ? divisors.value : defaultDivisors);
    const mutedBeats = positiveDivisors(mute.checked ? mutedDivisors.value : defaultMutedDivisors);
    const length = mute.checked ? minLength.valueAsNumber : defaultMinLength;
    const index = mute.checked && mutedIndex.value !== "" ? mutedIndex.valueAsNumber : null;
    if (!beatDivisors || !mutedBeats) {
      error.textContent = "Use positive whole numbers for beat divisors.";
      return;
    }
    if (!Number.isFinite(length) || length < 0) {
      error.textContent = "Enter a minimum slider length of 0 beats or more.";
      return;
    }
    if (index !== null && (!Number.isInteger(index) || index < 0 || index > maxSampleIndex)) {
      error.textContent = "Enter a valid custom sample index, or leave it blank to keep the existing index.";
      return;
    }
    closeModal();
    act({ kind: "copy", targets, options: {
      mode: mode.value, leniency: tolerance,
      copy_hitsounds: hitsounds.checked, copy_body_hitsounds: bodies.checked, copy_sample_sets: sets.checked,
      copy_volumes: volumes.checked, always_preserve_5_volume: keep5.checked,
      copy_storyboarded_samples: storyboard.checked,
      ignore_hitsound_satisfied_samples: satisfied.checked, ignore_whenever_hitsound: whenever.checked,
      mute_slider_ends: mute.checked, beat_divisors: beatDivisors, muted_divisors: mutedBeats,
      min_length: length, muted_index: index, muted_sample_set: mute.checked && mutedSet.value ? mutedSet.value : null,
    } });
  };

  const details = document.createElement("details");
  const summary = document.createElement("summary");
  summary.textContent = "Advanced options";
  details.append(summary, labeled("Copy storyboard samples", storyboard),
    labeled("Skip samples a target hit already plays", satisfied), labeled("Skip samples at any hit", whenever),
    labeled("Mute unmatched slider and spinner ends", mute), labeled("Beat divisors (highest priority first)", divisors),
    labeled("Muted beat divisors", mutedDivisors), labeled("Minimum slider length (beats)", minLength),
    labeled("Muted sample index", mutedIndex), labeled("Muted sample set", mutedSet));
  modal("copy", "Copy hitsounds", [
    ...choices, labeled("Timing tolerance (ms)", leniency), labeled("Unmatched target hits", mode),
    labeled("Copy hitsounds", hitsounds), labeled("Copy slider-body sounds", bodies),
    labeled("Copy sample sets and indexes", sets), labeled("Copy volumes", volumes), labeled("Keep target hits at 5%", keep5),
    details, error,
  ], [["Cancel", closeModal], ["Review output", submit, true]]);
});
$("next-output-problem").addEventListener("click", () => showOutputProblem(outputProblemIndex + 1));
$("warnings").addEventListener("click", () => {
  modal("warnings", "Audio warnings", view.warnings.map(paragraph), [["Close", closeModal, true]]);
});
const playbackStartChoices = [
  ["cursor", "Cursor"], ["left", "Left locator"],
  ["selection", "Selection"], ["selection_or_left", "Selection or left locator"],
];

$("audio").addEventListener("click", () => run(async () => {
  const { current, devices, bufferMs } = await invoke("audio_outputs");
  const output = document.createElement("select");
  if (!devices.includes(current)) {
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = current;
    placeholder.disabled = true;
    output.append(placeholder);
  }
  for (const name of devices) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    output.append(option);
  }
  output.value = devices.includes(current) ? current : "";
  output.disabled = !!view.playing || !!view.busy || !devices.length;
  output.addEventListener("change", () => {
    if (!view.playing && !view.busy && output.value) act({ kind: "output_device", name: output.value }, false);
  });
  const buffer = input("number", bufferMs ?? "", 10, 500);
  buffer.id = "audio-buffer";
  buffer.placeholder = "Device default";
  buffer.title = "Clear to use the device default buffer";
  buffer.disabled = output.disabled;
  buffer.addEventListener("change", () => {
    if (view.playing || view.busy) return;
    const ms = buffer.value === "" ? null : Number(buffer.value);
    if (ms !== null && (!Number.isInteger(ms) || ms < 10 || ms > 500)) {
      buffer.reportValidity();
      return;
    }
    act({ kind: "output_buffer", ms }, false);
  });

  const song = input("range", view.song, 0, 1, .01);
  const main = input("range", view.main, .2, 1, .01);
  const mute = input("checkbox", "", 0, 0);
  mute.checked = view.songMuted;
  const offset = input("number", view.offset, -60000, 60000, 1);
  const start = document.createElement("select");
  for (const [value, label] of playbackStartChoices) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    start.append(option);
  }
  start.value = view.playbackStart ?? "cursor";
  start.addEventListener("change", () => act({ kind: "playback_start", value: start.value }, false));
  const returnOnStop = input("checkbox", "", 0, 0);
  returnOnStop.checked = view.returnOnStop;
  returnOnStop.addEventListener("change", () => act({ kind: "return_on_stop", on: returnOnStop.checked }, false));
  const gains = () => act({ kind: "gains", song: +song.value, main: +main.value }, false);
  song.addEventListener("change", gains);
  main.addEventListener("change", gains);
  mute.addEventListener("change", () => act({ kind: "song_muted", on: mute.checked }, false));
  offset.addEventListener("change", () => act({ kind: "offset", ms: offset.valueAsNumber || 0 }, false));
  modal("audio", "Audio", [
    labeled("Output device", output), labeled("Requested buffer (ms)", buffer),
    ...(view.playing ? [paragraph("Stop playback to change audio output.")] : []),
    labeled("Song volume", song), labeled("Master volume", main),
    labeled("Mute song", mute), labeled("Song offset (ms)", offset),
    labeled("Start from", start), labeled("Return on stop", returnOnStop),
  ], [["Done", closeModal, true]]);
}));

$("panel-view").addEventListener("click", () => {
  if (!scene || !view.panels || view.busy) return;
  const options = [
    ["Inspector", "inspector", view.panels.inspector],
    ["Volume controller", "controller", view.panels.controller],
    ["Lane names", "lane_names", view.panels.laneNames],
    ["File browser", "browser", view.panels.browser],
  ];
  const info = document.createElement("input");
  info.type = "checkbox";
  info.checked = showLaneInfo;
  info.disabled = !view.panels.laneNames;
  info.addEventListener("change", () => {
    showLaneInfo = info.checked;
    renderLanes();
    paintSoon();
  });
  const controls = options.map(([label, panel, on]) => {
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = on;
    checkbox.addEventListener("change", () => {
      if (panel === "lane_names") info.disabled = !checkbox.checked;
      act({ kind: "panel", panel, on: checkbox.checked });
    });
    return labeled(label, checkbox);
  });
  const style = document.createElement("select");
  for (const [value, label] of [["tree", "Folder tree · sidebar"], ["classic", "Classic · inspector"]]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    style.append(option);
  }
  style.value = view.classicBrowser ? "classic" : "tree";
  style.addEventListener("change", () => act({ kind: "browser_style", classic: style.value === "classic" }));
  controls.push(labeled("File browser style", style), labeled("Lane info (sample indexes)", info));
  modal("editor-view", "View", controls, [["Done", closeModal, true]]);
});

async function showHelp() {
  const groups = await invoke("editor_help");
  const search = document.createElement("input");
  search.type = "search";
  search.value = "";
  const results = document.createElement("div");

  function renderMatches() {
    const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const content = [];
    for (const [title, entries] of groups) {
      const matches = entries.filter(([keys, description]) => {
        const text = [title, keys, description].join(" ").toLowerCase();
        return terms.every(term => text.includes(term));
      });
      if (!matches.length) continue;
      const heading = document.createElement("h3");
      heading.textContent = title;
      content.push(heading, ...matches.map(([keys, description]) =>
        paragraph(keys ? [keys, description].join(" — ") : description)));
    }
    results.replaceChildren(...(content.length ? content : [paragraph("No matching shortcuts or mouse actions.")]));
  }

  search.addEventListener("input", renderMatches);
  renderMatches();
  modal("help", "Shortcuts and mouse actions", [labeled("Search", search), results], [
    ["Customise shortcuts…", () => run(showShortcutEditor)],
    ["Close", closeModal, true],
  ]);
  search.focus();
}

$("help").addEventListener("click", () => run(showHelp));

async function showShortcutEditor() {
  let state = await invoke("editor_shortcuts");
  let chord = null;
  const search = document.createElement("input");
  search.type = "search";
  search.value = "";
  const command = document.createElement("select");
  const preset = document.createElement("select");
  for (const [label, value] of [["DAW-like", "DAW"], ["osu!", "osu!"]]) {
    const option = document.createElement("option");
    option.textContent = label;
    option.value = value;
    preset.append(option);
  }
  preset.value = state.preset;
  const current = document.createElement("output");
  const summary = document.createElement("p");
  summary.className = "dim";
  summary.setAttribute("role", "status");
  const capture = document.createElement("input");
  capture.type = "text";
  capture.readOnly = true;
  capture.placeholder = "Press a key";
  const add = makeButton("Add key", () => change(chord));
  const clear = makeButton("Clear keys", () => change(null));
  const controls = document.createElement("div");
  controls.className = "row";
  controls.append(add, clear);

  function selectCommand() {
    const entry = state.entries.find(([id]) => id === command.value);
    current.textContent = entry?.[2].join(", ") || "Unassigned";
    capture.disabled = clear.disabled = !entry;
    add.disabled = !entry || !chord;
  }

  function filter() {
    const selected = command.value;
    const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const options = state.entries.filter(([id, description, keys]) => {
      const text = [id, description, ...keys].join(" ").toLowerCase();
      return terms.every(term => text.includes(term));
    }).map(([id, description]) => {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = description;
      return option;
    });
    command.replaceChildren(...options);
    command.value = options.some(option => option.value === selected) ? selected : options[0]?.value ?? "";
    selectCommand();
  }

  function refresh(task) {
    return run(async () => {
      try {
        await task();
        if (modalKind !== "shortcut-editor" || view.error) return;
        state = await invoke("editor_shortcuts");
        summary.textContent = view.status ?? "";
        chord = null;
        capture.value = "";
        filter();
      } catch (error) {
        summary.textContent = String(error);
      }
    });
  }

  function change(key) {
    if (!command.value || (key && add.disabled)) return;
    const id = command.value;
    refresh(() => update({ kind: "change_shortcut", id, chord: key }));
  }

  search.addEventListener("input", filter);
  command.addEventListener("change", selectCommand);
  preset.addEventListener("change", () => {
    const name = preset.value;
    refresh(() => update({ kind: "key_preset", name }));
  });
  capture.addEventListener("keydown", (event) => {
    if (["Control", "Meta", "Alt", "Shift"].includes(event.key) || event.repeat) return;
    chord = keyInput(event);
    if (chord.key !== "Tab" || chord.ctrl || chord.alt) event.preventDefault();
    capture.value = [chord.ctrl ? "Ctrl" : "", chord.shift ? "Shift" : "", chord.alt ? "Alt" : "",
      chord.key === " " ? "Space" : chord.key].filter(Boolean).join("+");
    selectCommand();
  });
  filter();
  modal("shortcut-editor", "Customise shortcuts", [
    labeled("Preset", preset), labeled("Search", search), labeled("Command", command),
    labeled("Assigned keys", current), labeled("New shortcut", capture), controls, summary,
  ], [
    ["Import…", () => refresh(async () => { if (await invoke("import_key_file")) await update(null, false); })],
    ["Export…", () => refresh(async () => { if (await invoke("export_key_file")) await update(null, false); })],
    ["Close", closeModal, true],
  ]);
  search.focus();
}

function keyInput(event) {
  return {key: event.key, code: event.code ?? "", ctrl: !!(event.ctrlKey || event.metaKey),
    shift: !!event.shiftKey, alt: !!event.altKey};
}
document.addEventListener("keydown", (event) => {
  if (event.defaultPrevented || event.repeat || event.isComposing || dialog.open ||
      event.target.closest("input, select, textarea, [contenteditable]")) return;
  const { key, code, ctrl, shift, alt } = keyInput(event);
  if (key === "Escape" && drag && !ctrl && !shift && !alt) {
    event.preventDefault();
    const { surface, firstPoint, lastPoint, id } = drag;
    drag = null;
    if (surface.hasPointerCapture(id)) surface.releasePointerCapture(id);
    act({ ...(lastPoint ?? firstPoint), phase: "cancel" });
    return;
  }

  if (key === "Escape" && view.showHome && view.hasDocument && !view.busy && !ctrl && !shift && !alt) {
    event.preventDefault();
    act({ kind: "resume" });
    return;
  }
  if (view.busy || ((key === " " || key === "Enter") && event.target.closest("button"))) return;
  if (key === "Tab") {
    const bound = view.tabRoutes?.some(([control, shifted, alternate, global]) =>
      control === ctrl && shifted === shift && alternate === alt && (scene || global));
    if (!bound) return;
  } else if (key.length !== 1 &&
      !/^(Arrow(Left|Right|Up|Down)|Delete|Backspace|Escape|Home|End|PageUp|PageDown|Insert|Enter|F([1-9]|1[0-2]))$/.test(key) &&
      !code.startsWith("Numpad")) return;

  event.preventDefault();
  act({ kind: "key", key, code, ctrl, shift, alt });
});

window.__TAURI__.event.listen("tauri://drag-drop", ({ payload }) => {
  const map = payload.paths.find((path) => /\.(osu|osz)$/i.test(path));
  if (map) {
    act({ kind: "open", path: map });
    return;
  }
  const target = sampleTarget();
  const path = payload.paths.find((path) => /\.(wav|ogg|mp3)$/i.test(path));
  if (target && path) act({ kind: "replace_sample", target, path, mapset: false });
});

window.__TAURI__.event.listen("editor-close-requested", ({ payload }) => {
  if (view.busy) {
    modal("close-busy", "Job in progress", [paragraph(payload)], [["Keep editor open", closeModal, true]]);
    return;
  }
  modal("close-editor", "Quit dawsu!?", [paragraph(payload)], [
    ["Cancel", closeModal],
    ["Discard changes", () => run(() => invoke("close_editor", { discard: true }))],
    ["Save and quit", async () => {
      closeModal();
      await act({ kind: "save" });
      if (view.busy) quitAfterSave = true;
      else if (!view.dirty) run(() => invoke("close_editor", { discard: false }));
    }, true],
  ]);
});

let quitAfterSave = false;
setInterval(async () => {
  const now = performance.now();
  if (requests || refreshing || now - lastRefresh < (view.playing || view.busy ? 85 : 1000)) return;
  refreshing = true;
  lastRefresh = now;
  await run(() => update(null, false));
  refreshing = false;
  if (quitAfterSave && !view.busy) {
    quitAfterSave = false;
    if (!view.dirty && !view.error) run(() => invoke("close_editor", { discard: false }));
  }
}, 85);
act(null);
