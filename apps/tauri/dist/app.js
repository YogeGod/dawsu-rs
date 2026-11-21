"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const grid = $("grid");
const controller = $("controller");
const ruler = $("ruler");
const overview = $("overview");
const dialog = $("dialog");
const dock = window.editorDocking?.attach($("editor-dock"));
let view = { busy: false };
let scene = null;
let drag = null;
let rulerDrag = null;
let overviewDrag = null;
let overviewTarget = null;
let overviewNavigation = null;
const rulerBandHeight = 21;
const defaultWaveformHeight = 56;
let waveformHeight = defaultWaveformHeight;
let visibleWaveformHeight = defaultWaveformHeight;
const bookmarkFlagHeight = 22;
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
let laneVisualPositions = new Map();
const selectionLaneKeys = new Set();
let selectionLaneAnchor = null;
let selectionLanesOnly = false;
const collapsedFamilies = new Set();
const laneColours = new Map();
let familyParents = new Map();
let previewParents = new Map();
const speedButtons = [...document.querySelectorAll("[data-playback-speed]")];
const sampleFolderNames = { normal: "Normal set", soft: "Soft set", drum: "Drum set" };
let sampleFolderOrder = Object.keys(sampleFolderNames);
let layoutWrites = 0;
const collapsedFolders = new Set();
let modalKind = "";
let modalClose = null;
let outputProblemIndex = -1;
let outputProblemList = "";
let rulerWarningMarkers = [];
// Canvas surfaces match the surrounding editor chrome.
const slateTimeline = {
  arrange: "#151515", alternateRow: "#1c1c1c", activeRow: "#454545", group: "#292929",
  ruler: "#252525", rulerBar: "#c4c4c4", overview: "#1c1c1c", waveform: "#1c2727",
  volume: "#151515", divider: "#383838", beatLine: "#383838", barLine: "#545454",
  cycle: "#efc16a", loop: "#77bdfa0f", locator: "#858585", playhead: "#a1d0f7",
};
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

// The colour a lane has until the mapper picks one: v3's lane lightness with no chroma, the
// bundle's laneGrey (ui/src/lib/colour.js). Hit diamonds keep the volume colours (Yoge, 2026-11-19).
const laneGrey = "#9e9e9e";
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
  if (!next.fault && !full && (view.busy !== next.busy || view.hasDocument !== next.hasDocument ||
      view.laneCount !== next.laneCount || view.showHome !== next.showHome)) {
    // The editor names a Previous or Next bookmark step once, in the first answer.
    const step = next.bookmarkStep;
    next = await invoke("editor_view", { scene: true, action: null });
    if (!next.fault) next.bookmarkStep ??= step;
  }
  show(next);
  if (next.shortcutHelp) await showHelp();
  if (next.shortcutOpen && !next.busy && !chooseSiblingDifficulty()) show(await invoke("pick_map"));
  if (!next.busy) {
    switch (next.shortcutAction) {
      case "filter": {
        const filter = $("lane-filter");
        const panel = filter.closest("[popover]");
        if (panel && !panel.matches(":popover-open")) panel.showPopover();
        filter.focus();
        break;
      }
      case "newDiff":
        $("new")?.click();
        break;
      case "openFolder":
        await invoke("open_editor_folder", { folder: "map" });
        break;
      case "importOsz":
        show(await invoke("pick_map", { archiveOnly: true }));
        break;
      case "openHitsoundFolder":
        if (next.hitsoundLibrary) await invoke("open_editor_folder", { folder: "library" });
        else $("folders").click();
        break;
      case "reviewOutput":
        await openMergeSettings();
        break;
    }
  }
}

function act(action, full = true) {
  return run(() => update(action, full));
}

let orderedLaneKeys = [];
const renamedLanes = new Map();

function hydrateLaneLayout(layout) {
  orderedLaneKeys = [...layout?.lane_order ?? []];
  renamedLanes.clear();
  for (const [key, name] of Object.entries(layout?.lane_names ?? {})) renamedLanes.set(key, name);
  sampleFolderOrder = layout?.folder_order?.slice() ?? Object.keys(sampleFolderNames);
  collapsedFolders.clear();
  for (const group of layout?.collapsed_folders ?? []) collapsedFolders.add(group);
  laneColours.clear();
  for (const [key, colour] of Object.entries(layout?.lane_colours ?? {})) {
    if (/^#[\da-f]{6}$/i.test(colour)) laneColours.set(key, colour);
  }
  collapsedFamilies.clear();
  for (const family of layout?.collapsed_families ?? []) collapsedFamilies.add(family);
}

function saveLaneLayout(persist = false) {
  const path = scene.path;
  const value = {
    folder_order: [...sampleFolderOrder],
    collapsed_folders: [...collapsedFolders],
    collapsed_families: [...collapsedFamilies],
    lane_colours: Object.fromEntries(laneColours),
    ...(renamedLanes.size ? { lane_names: Object.fromEntries(renamedLanes) } : {}),
    ...(orderedLaneKeys.length ? { lane_order: [...orderedLaneKeys] } : {}),
  };
  layoutWrites++;
  run(async () => {
    try {
      if (path === scene?.path) await update({ kind: "lane_layout", path, persist, value }, false);
    } finally {
      layoutWrites--;
    }
  });
}

function show(next) {
  if (next.fault) {
    // A new object, so the Svelte parts see the fault and disable.
    view = { ...view, fault: next.fault };
    try { window.dawsuUi?.update(view, scene); } catch (error) { console.error(error); }
    $("status").textContent = next.fault;
    const close = () => run(() => invoke("close_editor", { discard: false }));
    modal("fault", "Editor stopped", [paragraph(next.fault)], [["Close dawsu!", close, true]], close);
    return;
  }
  const previousTool = view.tool;
  const wasBusy = view.busy;
  view = next;
  if (view.tool !== previousTool) refreshToolCursor();
  if (!view.hasDocument || view.showHome) scene = null;
  if (view.scene) {
    const loaded = view.scene.path !== scene?.path || (wasBusy && view.status === "Difficulty loaded");
    if (loaded) {
      overviewDrag = null;
      overviewTarget = null;
      usedLaneKeys = null;
      markedRange = null;
      selectionLaneKeys.clear();
      selectionLaneAnchor = null;
      if (selectionLanesOnly) act({ kind: "volume_scope", path: view.scene.path, lanes: null }, false);
      selectionLanesOnly = false;
      $("used-only").checked = false;
      $("lane-filter").value = "";
    }
    if (loaded || !layoutWrites) hydrateLaneLayout(view.scene.laneLayout);
    scene = view.scene;
  }
  followBookmarkPick();
  if (rulerPointer || rulerDrag) refreshRulerCursor();
  $("home").hidden = !!scene;
  $("workspace").hidden = !scene;
  $("go-home").hidden = !scene;
  $("home-resume").hidden = !view.showHome || !view.hasDocument;
  $("home-resume").disabled = !!view.busy;
  const path = scene?.path ?? "";
  const title = $("title");
  title.textContent = path ? (view.dirty ? "● " : "") + nameOf(path) : "";
  title.title = path;
  title.setAttribute("data-placeholder", view.hasDocument ? "" : "No map open");
  $("status").textContent = missingUiParts() || view.status || (view.busy ? "Working…" : "");
  $("status").title = $("status").textContent;
  $("editor-header").dataset.empty = String(!scene);
  for (const control of document.querySelectorAll("[data-map]")) control.disabled = !scene;
  $("status-view").hidden = !scene;
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
  $("problem-summary").hidden = problems.length === 0;
  $("problem-summary-text").textContent = problems.length + (problems.length === 1 ? " moment" : " moments") + " can't be saved";
  $("next-output-problem").hidden = problems.length === 0;
  $("next-output-problem").disabled = !!view.busy;
  $("playback-rates").hidden = !scene;
  $("hit-lane-counts").hidden = !scene;
  $("cancel-job").hidden = !view.busy;
  $("difficulty").hidden = !scene;
  for (const button of document.querySelectorAll("[data-doc]")) button.disabled = !scene || !!view.busy;
  $("samples-view").ariaPressed = String(!view.drumView?.enabled);
  $("drums-view").ariaPressed = String(!!view.drumView?.enabled);
  $("drums-view").title = view.drumView?.name ?? "Open a drum map";
  $("ramp-shape").value = view.rampShape ?? "Linear";
  $("song-mute").ariaPressed = String(!!view.songMuted);
  $("song-solo").ariaPressed = String(!!view.songSolo);
  $("open").disabled = $("home-open").disabled = $("folders").disabled = !!view.busy;
  $("home-import").disabled = $("home-folders").disabled = !!view.busy;

  renderActiveLane();
  renderPanelLayout();
  if (!scene && view.recent) renderRecent(view.recent);
  if (!scene && view.recovery) renderRecovery(view.recovery);
  else if (view.hasDocument && !$("recovery").hidden) {
    // Home with an open difficulty cannot restore, so hide the start-screen offers.
    $("recovery").hidden = true;
    $("recovery-entries").dataset.state = "";
  }
  if (view.scene) renderScene();
  else renderSelection();
  if (scene) {
    for (const button of speedButtons) button.ariaPressed = String(Number(button.dataset.playbackSpeed) === view.speed);
    $("ruler-follow").setAttribute?.("aria-pressed", String(!!view.follow));
    followPlayhead();
  }
  refreshLaneDelete();
  renderDialogs();
  try { window.refreshEditorMenu?.(); } catch (error) { console.error(error); }
  paintSoon();
  try { window.dawsuUi?.update(view, scene); } catch (error) { console.error(error); }
  $("triplet-snap").checked = window.dawsuUi?.snapGroup?.() === "triplets";
}

// The Svelte bundle loads after this script, so a part counts as missing only after the page has loaded.
function missingUiParts() {
  if (document.readyState !== "complete") return "";
  return document.querySelectorAll("[data-ui-mount]:not([data-ui-ready])").length ? "Part of the editor did not load" : "";
}

function setValue(id, value) {
  if (document.activeElement !== $(id)) $(id).value = value;
}

// "Artist - Title (Creator) [Difficulty].osu" gives the map name and "[Difficulty]"; other names show whole.
function recentMapName(path) {
  const file = nameOf(path).replace(/\.(osu|osz)$/i, "");
  const parts = /^(.+?)\s*\([^()]*\)\s*(\[[^\]]*\])$/.exec(file);
  return parts ? [parts[1], parts[2]] : [file, ""];
}

function renderRecent(paths) {
  const panel = $("recent");
  $("home-recent").hidden = paths.length === 0;
  if (panel.dataset.paths === JSON.stringify(paths)) return;
  panel.dataset.paths = JSON.stringify(paths);
  panel.replaceChildren(...paths.map((path) => {
    const button = makeButton("", () => act({ kind: "open", path }));
    const [name, difficulty] = recentMapName(path);
    const title = document.createElement("strong");
    title.textContent = name;
    const detail = document.createElement("span");
    detail.textContent = difficulty;
    button.append(title, detail);
    button.className = "home-recent-map";
    button.title = path;
    return button;
  }));
}

function renderRecovery({ offers = [], error }) {
  const section = $("recovery");
  const entries = $("recovery-entries");
  const state = JSON.stringify([offers, error, view.busy]);
  if (entries.dataset.state === state) return;
  entries.dataset.state = state;
  section.hidden = !error && offers.length === 0;
  entries.replaceChildren();
  entries.style.maxHeight = "200px";
  entries.style.overflowY = "auto";
  if (error) entries.append(paragraph(error));
  for (const offer of offers) {
    const name = offer.mapset + " [" + offer.difficulty + "]";
    const date = new Date(offer.created_ms);
    const label = paragraph(name);
    const time = document.createElement("time");
    time.textContent = date.toLocaleString();
    label.append(document.createElement("br"), time);
    const restore = makeButton("Restore", () => act({ kind: "restore_recovery", id: offer.id }));
    restore.setAttribute("aria-label", "Restore " + name);
    restore.disabled = view.busy;
    entries.append(label, restore);
  }
}

function renderPanelLayout() {
  if (!view.panels) return;
  const { inspector, controller, laneNames, browser: showBrowser } = view.panels;
  $("view-inspector").ariaPressed = String(!!scene && !!inspector);
  $("view-browser").ariaPressed = String(!!scene && !!showBrowser);
  $("editor-dock").dataset.inspector = String(inspector);
  $("editor-arrange").dataset.controller = String(controller);
  $("editor-arrange").dataset.laneNames = String(laneNames);
  const namesToggle = $("toggle-lane-names");
  namesToggle.ariaExpanded = String(laneNames !== false);
  namesToggle.title = namesToggle.ariaLabel = laneNames === false ? "Show lane names" : "Hide lane names";
  namesToggle.textContent = laneNames === false ? "›" : "‹";
  $("volume-controller-panel").hidden = !controller;
  const browser = scene && showBrowser ? (view.classicBrowser ? "classic" : "tree") : "off";
  $("editor-dock").dataset.browser = browser;
  if (dock) {
    if (scene) dock(browser, !!inspector);
  } else {
    $("editor-inspector").hidden = !inspector;
    $("tree-browser-host").hidden = browser !== "tree";
    $("classic-browser-host").hidden = browser !== "classic";
    if (browser !== "off") {
      const host = $(browser + "-browser-host");
      const panel = $("sample-browser");
      if (panel.parentNode !== host) host.append(panel);
    }
  }
  if (browser !== "off") renderSampleBrowser();
}

function reorderSampleFolder(from, to, after) {
  if (view.busy || drag || from === to || !sampleFolderOrder.includes(from) || !sampleFolderOrder.includes(to)) return;
  sampleFolderOrder = sampleFolderOrder.filter(group => group !== from);
  sampleFolderOrder.splice(sampleFolderOrder.indexOf(to) + (after ? 1 : 0), 0, from);
  renderLanes();
  paintSoon();
  saveLaneLayout();
}

// A folder header row for the Svelte lane headers: count and used lanes, and the group's mix.
function folderHeaderModel(group) {
  const rows = orderedLaneRows().filter(row => scene.lanes[row]?.group === group);
  const members = scene.lanes.filter(lane => lane.group === group);
  return {
    type: "folder", key: "folder:" + group, set: group, name: sampleFolderNames[group],
    count: rows.length, used: rows.filter(row => scene.lanes[row].hitCount > 0).length,
    collapsed: collapsedFolders.has(group),
    muted: members.every(lane => lane.muted), solo: members.every(lane => lane.solo),
  };
}

function laneDisplayName(lane) {
  return renamedLanes.get(lane?.colourKey) ?? lane?.name ?? "";
}

const laneSounds = {
  hitnormal: ["Normal", "normal"], hitwhistle: ["Whistle", "whistle"], hitfinish: ["Finish", "finish"],
  hitclap: ["Clap", "clap"], sliderslide: ["Slide", "slide"], sliderwhistle: ["Slide whistle", "whistle"],
  slidertick: ["Tick", "tick"],
};
// A set lane reads as its sound ("Clap", or "Clap 2" for a numbered layer) with the set as a tag;
// the skin's default sample adds "skin" to the tag. A map file lane keeps its file name and the tag "File".
function laneSound(lane) {
  if (!lane?.group) return { name: lane?.name ?? "", set: "File", icon: "file" };
  const set = lane.group[0].toUpperCase() + lane.group.slice(1) + (lane.index === 0 ? " · skin" : "");
  const [sound, icon] = laneSounds[lane.family?.slice(lane.family.indexOf("-") + 1)] ?? [];
  return sound ? { name: sound + (lane.index > 1 ? " " + lane.index : ""), set, icon }
    : { name: lane.name, set, icon: "file" };
}

// The lane's name in the Inspector and the lane filter: "Soft Clap", or the mapper's own name.
function laneTitle(lane) {
  if (!lane || renamedLanes.has(lane.colourKey)) return laneDisplayName(lane);
  const { name } = laneSound(lane);
  return lane.group ? lane.group[0].toUpperCase() + lane.group.slice(1) + " " + name : name;
}

function orderedLaneRows() {
  const rows = scene.laneOrder ?? scene.lanes.map((_, row) => row);
  if (!orderedLaneKeys.length) return rows;
  const rank = new Map(orderedLaneKeys.map((key, index) => [key, index]));
  const position = row => rank.get(scene.lanes[row].colourKey) ?? rank.size;
  return [...rows].sort((left, right) => position(left) - position(right));
}

function adjacentLaneForMove(row, direction) {
  if (view.drumView?.enabled || !scene?.lanes[row]?.colourKey) return null;
  const position = laneRows.indexOf(row);
  if (position < 0) return null;
  const adjacent = laneRows[position + direction];
  return typeof adjacent === "number" && scene.lanes[adjacent].colourKey &&
    scene.lanes[adjacent].group === scene.lanes[row].group ? adjacent : null;
}

function moveLane(row, direction) {
  const adjacent = adjacentLaneForMove(row, direction);
  if (adjacent === null || view.busy || drag) return;
  const rows = orderedLaneRows().filter(candidate => candidate !== row);
  rows.splice(rows.indexOf(adjacent) + (direction > 0 ? 1 : 0), 0, row);
  const present = new Set(rows.map(candidate => scene.lanes[candidate].colourKey));
  orderedLaneKeys = [...rows.map(candidate => scene.lanes[candidate].colourKey),
    ...orderedLaneKeys.filter(key => !present.has(key))];
  refreshLanePresentation();
}

function renameLane(row) {
  const lane = scene?.lanes[row];
  if (!lane?.colourKey || view.busy || drag || view.drumView?.enabled) return;
  const path = scene.path;
  const field = input("text", renamedLanes.get(lane.colourKey) ?? "", "", "");
  field.placeholder = lane.name;
  field.maxLength = 120;
  const apply = () => {
    if (view.busy || scene?.path !== path || !scene.lanes.some(item => item.colourKey === lane.colourKey)) return;
    const name = field.value.trim();
    if (name && name !== lane.name) renamedLanes.set(lane.colourKey, name);
    else renamedLanes.delete(lane.colourKey);
    closeModal();
    refreshLanePresentation();
  };
  field.addEventListener("keydown", event => {
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      apply();
    }
  });
  modal("rename-lane", "Rename lane", [labeled("Display name", field),
    paragraph("Leave blank to restore the sample name.")],
    [["Cancel", closeModal], ["Rename", apply, true]]);
  field.focus();
}

function refreshLanePresentation() {
  renderLanes();
  renderActiveLane();
  paintSoon();
  saveLaneLayout(true);
}

function groupedLaneRows(eligible) {
  const families = new Map();
  for (const row of eligible) {
    const family = scene.lanes[row].family;
    if (family == null) continue;
    if (!families.has(family)) families.set(family, []);
    families.get(family).push(row);
  }
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
}

function renderLanes() {
  const filter = ($("lane-filter").value ?? "").trim().toLowerCase();
  const eligible = (row, alias) => {
    const lane = scene.lanes[row];
    if (usedLaneKeys !== null && (!lane || !usedLaneKeys.has(lane.key))) return false;
    return !filter || [alias, laneDisplayName(lane), laneTitle(lane), lane?.name, lane?.sample]
      .some(name => name?.toLowerCase().includes(filter));
  };
  familyParents = new Map();
  previewParents = new Map();
  let drumNames = [];
  if (view.drumView?.enabled && scene.drumRows) {
    const rows = scene.drumRows.filter(({ row, name }) => eligible(row, name));
    laneRows = rows.map(({ row }) => row);
    drumNames = rows.map(({ name }) => name);
  } else {
    groupedLaneRows(orderedLaneRows().filter(row => eligible(row)));
  }
  lanePositions = new Map();
  laneVisualPositions = new Map();
  for (const [position, row] of laneRows.entries()) {
    if (!lanePositions.has(row)) lanePositions.set(row, position);
    if (typeof row !== "number") continue;
    if (!laneVisualPositions.has(row)) laneVisualPositions.set(row, []);
    laneVisualPositions.get(row).push(position);
  }
  refreshLaneDelete();
  const lanePanel = $("lanes");
  lanePanel.style.setProperty("--lane-row-height", scene.rowHeight + "px");
  const laneKey = JSON.stringify([laneRows, drumNames, scene.lanes, view.activeLane, [...selectionLaneKeys],
    [...familyParents], view.panels?.laneNames !== false, showLaneInfo, [...laneColours], [...renamedLanes], !!view.busy,
    !!view.drumView?.enabled]);
  if (lanePanel.dataset.state !== laneKey) {
    lanePanel.dataset.state = laneKey;
    laneHeaders = laneHeaderModel(drumNames);
    try { window.dawsuUi?.lanes(laneHeaders); } catch (error) { console.error(error); }
  }
}

// The rows the Svelte lane headers draw ("lanes" part), as plain data. Lane state stays here.
let laneHeaders = { names: true, busy: false, rows: [] };

function currentLaneHeaders() {
  return laneHeaders;
}

function laneHeaderModel(drumNames) {
  const drum = !!view.drumView?.enabled;
  const numbers = new Map(orderedLaneRows().map((row, index) => [row, index + 1]));
  const anySolo = scene.lanes.some(lane => lane?.solo);
  const rows = laneRows.map((row, position) => {
    if (typeof row === "string") return folderHeaderModel(row);
    if (row === null) {
      return { type: "drum", key: "drum:" + position, name: drumNames[position], title: drumNames[position] + " · Unassigned" };
    }
    const lane = scene.lanes[row];
    const displayName = drumNames[position] ?? laneDisplayName(lane);
    const label = showLaneInfo && lane.index != null
      ? displayName + (lane.index === 0 ? " · skin" : " · idx " + lane.index)
      : displayName;
    const sound = laneSound(lane);
    const family = familyParents.get(row);
    const collapsed = collapsedFamilies.has(family);
    return {
      type: "lane", key: drum ? "drum:" + position : "lane:" + row, row,
      number: drum ? position + 1 : numbers.get(row),
      name: !showLaneInfo && !drum && !renamedLanes.has(lane.colourKey) ? sound.name : label, label, display: displayName,
      title: (lane.sample ?? label) + " · Select and audition · Ctrl/Cmd toggles · Shift selects a range",
      tag: drum ? null : sound.set, icon: sound.icon,
      // Yoge, 2026-11-20 (ADR 0068): a lane is grey unless the mapper picked a colour.
      colour: laneColours.get(lane.colourKey) ?? null,
      folder: !drum && lane.group ? lane.group : null,
      active: view.activeLane === row, selected: selectionLaneKeys.has(lane.key),
      muted: !!lane.muted, solo: !!lane.solo, quiet: !!lane.muted || (anySolo && !lane.solo),
      layer: !drum && !!lane.group && lane.index > 1,
      family: family == null ? null : { collapsed, label: (collapsed ? "Expand " : "Collapse ") + lane.name + " layers" },
      addLayer: lane.canAddLayer ? "Add numbered " + lane.name + " layer" : null,
    };
  });
  return { names: view.panels?.laneNames !== false, busy: !!view.busy, rows };
}

// Every click on a lane or folder header comes back here from the Svelte part.
function laneHeaderAction(kind, ...args) {
  if (!scene || view.busy || drag) return;
  switch (kind) {
    case "select": {
      const [row, event = {}] = args;
      selectVolumeLane(row, event);
      if (!event.ctrlKey && !event.metaKey && !event.shiftKey) act({ kind: "lane", row, operation: "audition" });
      break;
    }
    case "mix":
      mixSelectedLanes(args[0], args[1]);
      break;
    case "family": {
      const family = familyParents.get(args[0]);
      if (family == null) return;
      if (!collapsedFamilies.delete(family)) collapsedFamilies.add(family);
      renderLanes();
      paintSoon();
      saveLaneLayout();
      break;
    }
    case "folder": {
      const [group] = args;
      if (!collapsedFolders.delete(group)) collapsedFolders.add(group);
      renderLanes();
      paintSoon();
      saveLaneLayout();
      break;
    }
    case "folderMix":
      act({ kind: "group_mix", set: args[0], operation: args[1] });
      break;
    case "folderMove":
      reorderSampleFolder(...args);
      break;
    case "folderStep": {
      const [group, after] = args;
      const headers = laneRows.filter(row => typeof row === "string");
      const next = headers[headers.indexOf(group) + (after ? 1 : -1)];
      if (next) reorderSampleFolder(group, next, after);
      break;
    }
    case "addLayer":
      act({ kind: "lane", row: args[0], operation: "add_layer" });
      break;
  }
}

function mixSelectedLanes(row, operation) {
  const lane = scene?.lanes[row];
  if (!lane || view.busy || drag) return;
  if (!selectionLaneKeys.has(lane.key)) selectVolumeLane(row, {});
  const lanes = scene.lanes.filter(lane => selectionLaneKeys.has(lane.key)).map(lane => lane.sourceKey);
  act({ kind: "lane_selection_mix", target: { map: scene.path, lane: lane.sourceKey }, operation, lanes });
}

// The Inspector's line under Change volume.
function volumeScopeHint() {
  return selectionLanesOnly
    ? (selectionLaneKeys.size ? "Changes selected lanes at the selected times." : "Select lanes to change their volume.")
    : "Changes all sounds at the selected times.";
}

function updateVolumeScope() {
  renderSelection();
  if (scene) {
    const lanes = selectionLanesOnly ? [...selectionLaneKeys] : null;
    act({ kind: "volume_scope", path: scene.path, lanes }, false);
  }
}

function selectVolumeLane(row, event) {
  const key = scene.lanes[row].key;
  const toggle = !!(event.ctrlKey || event.metaKey);
  if (!toggle) selectionLaneKeys.clear();
  const visible = event.shiftKey
    ? [...new Set(laneRows.filter(row => typeof row === "number").map(row => scene.lanes[row].key))]
    : [];
  const start = visible.indexOf(selectionLaneAnchor);
  const end = visible.indexOf(key);
  if (start >= 0 && end >= 0) {
    for (const key of visible.slice(Math.min(start, end), Math.max(start, end) + 1)) {
      selectionLaneKeys.add(key);
    }
  } else {
    if (!toggle || !selectionLaneKeys.delete(key)) selectionLaneKeys.add(key);
    selectionLaneAnchor = key;
  }
  if (selectionLanesOnly) updateVolumeScope();
  renderLanes();
}


function requestDrumMap() {
  if (!scene || view.busy || drag) return;
  const map = scene.path;
  run(async () => {
    if (scene?.path !== map || view.busy || drag) return;
    const next = await invoke("pick_drum_map", { map });
    if (next) show(next);
  });
}

function setDrumView(on) {
  const map = scene.path;
  run(async () => {
    if (scene?.path === map && !view.busy && !drag) show(await invoke("set_drum_view", { map, on }));
  });
}

$("samples-view").addEventListener("click", () => {
  if (scene && !view.busy && !drag && view.drumView?.enabled) {
    setDrumView(false);
  }
});
$("drums-view").addEventListener("click", () => {
  if (!scene || view.busy || drag || view.drumView?.enabled) return;
  if (view.drumView?.name) setDrumView(true);
  else requestDrumMap();
});

function deletableLane() {
  if (!scene || view.busy || drag || !lanePositions.has(view.activeLane)) return null;
  const lane = scene.lanes?.[view.activeLane];
  return lane?.canDelete ? lane : null;
}

function refreshLaneDelete() {
  const button = $("delete-lane");
  button.disabled = !deletableLane();
  const lane = scene?.lanes?.[view.activeLane];
  button.title = lane?.canDelete === false
    ? "Protected hitnormals and slider body lanes cannot be deleted here"
    : "Delete active lane";
}

$("add-lane").addEventListener("click", () => {
  if (!scene || view.busy || drag) return;
  const map = scene.path;
  run(async () => {
    if (scene?.path !== map || view.busy || drag) return;
    const candidates = await invoke("lane_add_candidates", { map });
    if (scene?.path !== map || view.busy || drag) return;
    if (!candidates.length) {
      modal("add-lane", "Add lane", [paragraph("No new samples or hidden lanes. Put a WAV, OGG, or MP3 file in the map folder to add a lane.")],
        [["Done", closeModal, true]]);
      return;
    }
    const lane = document.createElement("select");
    lane.append(...candidates.map(candidate => new Option(candidate.name, candidate.key)));
    lane.value = candidates[0].key;
    modal("add-lane", "Add lane", [labeled("Sample", lane)], [
      ["Cancel", closeModal],
      ["Add", () => {
        const target = { map, lane: lane.value };
        closeModal();
        act({ kind: "show_lane", target });
      }, true],
    ]);
  });
});

$("delete-lane").addEventListener("click", () => {
  const lane = deletableLane();
  if (!lane) return;
  const target = { map: scene.path, lane: lane.sourceKey };
  const revision = scene.laneRevision;
  const remove = () => act({ kind: "delete_lane", target, revision });
  if (!lane.hitCount) {
    remove();
    return;
  }
  modal("delete-lane", "Delete lane", [
    paragraph("Delete " + lane.name + " and its " + lane.hitCount + (lane.hitCount === 1 ? " hit?" : " hits?")),
    paragraph("Undo restores the lane and its hits. The sample file stays in the map folder."),
  ], [
    ["Cancel", closeModal, true],
    ["Delete", () => { closeModal(); remove(); }],
  ]);
});

// The bookmark row picked in the Inspector, kept by its difficulty and time so it survives each update.
// since is when it was picked, until the playhead reaches it.
let bookmarkPick = null;
// With audio open, a jump's position shows an update or two late; a pick waits this long for it.
const bookmarkJumpWait = 1000;

// The pick follows Previous and Next, and goes when the playhead moves off its bookmark or the
// difficulty changes, so Go to, Rename and Remove never act on a row away from the playhead.
function followBookmarkPick() {
  if (bookmarkPick?.path !== scene?.path) bookmarkPick = null;
  if (scene && view.bookmarkStep != null) bookmarkPick = { path: scene.path, time: view.bookmarkStep, since: Date.now() };
  if (!bookmarkPick) return;
  // A jump stops at the song's end.
  const at = Math.min(bookmarkPick.time, view.duration ?? Infinity);
  if (!scene.bookmarks?.some(mark => mark.time === bookmarkPick.time)) bookmarkPick = null;
  else if (view.position === at) bookmarkPick.since = null;
  else if (bookmarkPick.since === null || Date.now() - bookmarkPick.since > bookmarkJumpWait) bookmarkPick = null;
}

function selectedBookmark() {
  return bookmarkPick?.path === scene?.path ? scene?.bookmarks?.find(mark => mark.time === bookmarkPick.time) : undefined;
}

function drawLocatorFlags(ctx, width, left, right, cycling) {
  const flags = locatorFlags(width, left, right);
  ctx.font = "600 10px Inter, system-ui, sans-serif";
  for (const [side, letter] of [["left", "L"], ["right", "R"]]) {
    const { x, from } = flags[side];
    // The flag's box runs from `from`; a flag pushed off its line keeps to its own box.
    const pointsRight = from >= x;
    const [base, tip] = pointsRight ? [from, from + locatorFlagWidth] : [from + locatorFlagWidth, from];
    ctx.fillStyle = cycling ? slateTimeline.cycle : slateTimeline.locator;
    ctx.beginPath();
    ctx.moveTo(base, 0);
    ctx.lineTo(tip, 0);
    ctx.lineTo(tip + (pointsRight ? -3 : 3), locatorFlagHeight / 2);
    ctx.lineTo(tip, locatorFlagHeight);
    ctx.lineTo(base, locatorFlagHeight);
    ctx.closePath();
    ctx.fill();
    ctx.fillStyle = slateTimeline.ruler;
    ctx.fillText(letter, from + (pointsRight ? 3 : 5), 10);
  }
}

function drawBookmarks(ctx, marks, start, zoom, width) {
  const visible = marks.map(mark => [mark, (mark.time - start) * zoom])
    .filter(([, x]) => x >= 0 && x <= width);
  ctx.fillStyle = "#79d7dc";
  for (const [i, [mark, x]] of visible.entries()) {
    ctx.fillRect(x, rulerBandHeight, 2, bookmarkFlagHeight);
    ctx.fillRect(x, rulerBandHeight, 9, 5);
    const labelWidth = Math.min(180, (visible[i + 1]?.[1] ?? width) - x - 10);
    if (mark.label && labelWidth > 24) ctx.fillText(mark.label, x + 5, 36, labelWidth);
  }
}

function renderScene() {
  renderLanes();
  $("snap").checked = scene.snap;
  const hits = (scene.moments ?? []).reduce((sum, moment) => sum + (moment[3]?.length ?? 0), 0);
  const lanes = scene.lanes?.length ?? 0;
  $("hit-lane-counts").textContent = hits + (hits === 1 ? " hit · " : " hits · ") + lanes + (lanes === 1 ? " lane" : " lanes");
  $("object-lines").checked = view.objectLines;
  setValue("snap-mode", scene.snapMode);
  setValue("beats", scene.beats);
  setValue("divisor", scene.divisor);
  setValue("zoom", Math.log10(scene.zoom));
  setValue("lane-height", scene.rowHeight);
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
}

// The Svelte Inspector (ui/src/parts/InspectorSections.svelte) shows the active lane and the selection.
function renderActiveLane() {
  handOverInspector();
}

let selectedHitsScene = null;
let selectedHits = { lanes: [], volumes: [] };

// The lanes and volumes of the selected hits, worked out once per scene from its moments.
function selectedHitDetails() {
  if (selectedHitsScene === scene) return selectedHits;
  const lanes = new Set();
  const volumes = new Set();
  for (const [, volume, , hits] of scene?.moments ?? []) {
    for (const [row, selected, , gain] of hits) {
      if (!selected) continue;
      lanes.add(row);
      volumes.add(gain ?? volume);
    }
  }
  selectedHitsScene = scene;
  selectedHits = { lanes: [...lanes], volumes: [...volumes].sort((a, b) => a - b) };
  return selectedHits;
}

function renderSelection() {
  handOverInspector();
}

let inspectorState = null;

function handOverInspector() {
  inspectorState = inspectorModel();
  try {
    window.dawsuUi?.inspector?.(inspectorState);
  } catch (error) {
    console.error(error);
  }
}

// v3's Inspector names a lane "Clap · Soft set" and a selection's lane "Clap (Soft)". The lane
// headers and the lane filter keep laneTitle(); a lane the mapper renamed keeps its own name.
function inspectorLaneNames(lane) {
  if (!lane?.group || renamedLanes.has(lane.colourKey)) {
    const name = laneTitle(lane);
    return { title: name, short: name };
  }
  const { name } = laneSound(lane);
  const set = lane.group[0].toUpperCase() + lane.group.slice(1);
  return { title: name + " · " + set + " set", short: name + " (" + set + ")" };
}

// The bundle loads after this script, so it reads the last model at load.
function currentInspector() {
  return inspectorState;
}

// The Inspector's bookmark and history lists stay the same arrays while nothing in them changes,
// so the 85 ms updates during playback rebuild no rows.
const inspectorLists = new Map();
function keptList(name, list) {
  const key = JSON.stringify(list);
  const kept = inspectorLists.get(name);
  if (kept?.key === key) return kept.list;
  inspectorLists.set(name, { key, list });
  return list;
}

// Everything the Inspector's sections show; app.js keeps the state.
function inspectorModel() {
  const selection = scene?.selection;
  const { lanes, volumes } = selection ? selectedHitDetails() : { lanes: [], volumes: [] };
  const body = selection?.sliderBody;
  const lane = scene?.lanes?.[view?.activeLane];
  // A fault stops the editor like a running job does.
  const busy = !!view?.busy || !!view?.fault;
  return {
    busy, doc: !!scene && !busy,
    selection: selection ? {
      summary: selection.count + (selection.count === 1 ? " hit" : " hits") + " selected" +
        (lanes.length > 1 ? " · " + lanes.length + " lanes"
          : lanes.length ? " · " + inspectorLaneNames(scene.lanes?.[lanes[0]]).short : ""),
      start: clock(selection.start),
      volume: !volumes.length ? ""
        : volumes.length === 1 ? volumes[0] + "%" : volumes[0] + "–" + volumes[volumes.length - 1] + "%",
      object: selection.object,
      // The fields re-seed only when the difficulty or the slider body changes.
      sliderBody: body ? {
        key: JSON.stringify([scene.path, body]), set: body.set ?? "", whistle: !!body.whistle,
        whistleSet: body.whistleSet ?? "auto", volume: body.volume,
      } : null,
    } : null,
    scope: selectionLanesOnly ? "selected" : "all", hint: volumeScopeHint(),
    range: !!scene?.range, insertVolume: scene?.insertVolume ?? 100,
    protect: !!scene?.protect, audition: !!view?.audition, hs: !!scene?.hs,
    lane: lane ? {
      title: inspectorLaneNames(lane).title, sample: lane.sample && lane.sample !== lane.name ? lane.sample : "",
      colour: laneColours.get(lane.colourKey) ?? null, canColour: !!lane.colourKey, canReplace: !!lane.canReplace,
    } : null,
    bookmarks: keptList("bookmarks", (scene?.bookmarks ?? []).map(({ time, label }) => ({ time, label, clock: clock(time) }))),
    bookmark: selectedBookmark()?.time ?? null,
    history: keptList("history", scene ? ["Opened difficulty", ...scene.history ?? []] : []),
    historyPosition: scene?.historyPosition ?? 0,
  };
}

// Every Inspector control comes back here from the Svelte part and runs the code its old listener ran.
function inspectorAction(kind, ...args) {
  switch (kind) {
    case "laneColour": return openLaneColour();
    case "replaceSample": return openReplaceSample();
    case "generate": return openGenerate();
    case "new": return openNewDifficulty();
    case "copy": return openCopy();
    case "mergeSettings": return run(() => openMergeSettings());
    case "command": return command(args[0]);
    case "insert": {
      // A blank or unreadable field sets 100 %, as the old Insert volume field's fallback did.
      const value = typeof args[0] === "number" && Number.isFinite(args[0]) ? args[0] : 100;
      return act({ kind: "insert_volume", value: limit(Math.round(value), 0, 100) });
    }
    case "toggle": return act({ kind: args[0], on: !!args[1] });
    case "history": return act({ kind: "history", position: args[0] });
  }
  if (!scene || view.busy) return false;
  switch (kind) {
    case "moveStart": {
      // The time already shown is the hits' start rounded to the millisecond; moving there would only round them.
      if (!scene.selection || args[0].trim() === clock(scene.selection.start)) return false;
      const position = parseClock(args[0]);
      if (!Number.isFinite(position)) return false;
      act({ kind: "timeline_edit", path: scene.path, edit: { kind: "selection_start", position } });
      return true;
    }
    case "volume": {
      const [value, relative] = args;
      if (!scene.selection || !Number.isFinite(value)) return false;
      act({ kind: "volume", value: limit(Math.round(value), relative ? -100 : 0, 100), relative: !!relative });
      return true;
    }
    case "nudge":
      act({ kind: "volume", relative: true, value: args[0] });
      return true;
    case "scope":
      if (drag) return false;
      selectionLanesOnly = args[0] === "selected";
      updateVolumeScope();
      return true;
    case "sliderBody": {
      const body = scene.selection?.sliderBody;
      const { set, whistleSet, whistle, volume } = args[0];
      if (!body || !Number.isInteger(volume) || volume < 0 || volume > 100) return false;
      act({ kind: "slider_body", path: scene.path, index: body.index, set: set || null,
        whistle_set: whistleSet, whistle: !!whistle, volume });
      return true;
    }
    case "pickBookmark": {
      const mark = scene.bookmarks?.find(item => item.time === args[0]);
      if (!mark) return false;
      // No show() follows a pick, so the Inspector gets it now.
      bookmarkPick = { path: scene.path, time: mark.time, since: Date.now() };
      handOverInspector();
      act({ kind: "bookmark_jump", time: mark.time });
      return true;
    }
    case "bookmark": {
      if (args[0] === "add") {
        editBookmark(null);
        return true;
      }
      const mark = selectedBookmark();
      if (!mark) return false;
      if (args[0] === "goTo") act({ kind: "bookmark_jump", time: mark.time });
      else if (args[0] === "rename") editBookmark(mark);
      else if (args[0] === "remove") act({ kind: "bookmark_edit", path: scene.path, time: mark.time, label: null });
      else return false;
      return true;
    }
  }
  return false;
}

function clock(ms) {
  const rounded = Math.max(Math.round(ms), 0);
  const minutes = Math.floor(rounded / 60000);
  const seconds = (rounded % 60000 / 1000).toFixed(3);
  return String(minutes).padStart(2, "0") + ":" + seconds.padStart(6, "0");
}

// Reads an osu! editor time ("01:45.263", or "01:45:263" as in modding timestamps) or plain milliseconds.
function parseClock(text) {
  const value = String(text).trim();
  const time = value.match(/^(\d+):(\d{1,2})(?:([.:])(\d{1,3}))?(?![\d.:a-z])/i);
  if (time) {
    const [, minutes, seconds, mark, part = ""] = time;
    const milliseconds = mark === "." ? Number(part.padEnd(3, "0")) : Number(part || 0);
    return Number(minutes) * 60000 + Number(seconds) * 1000 + milliseconds;
  }
  return /^\d+(\.\d+)?$/.test(value) ? Number(value) : NaN;
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
      const color = ruler ? snapColours[1] : bar || longSnap ? slateTimeline.barLine : slateTimeline.beatLine;
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
      line(ctx, slateTimeline.barLine, x, ruler ? 3 : top, x, bottom);
      if (ruler) {
        ctx.fillStyle = slateTimeline.rulerBar;
        ctx.fillText(String(section.bar + n * stride), x + 5, 14);
      }
    }
  }
}

function waveform(ctx, width, center, height, start, span) {
  const bins = scene.waveform;
  if (!bins.length || !view.duration) return;
  // Playback reads the song at position - offset, so the drawn song moves by the same offset.
  const songStart = start - (view.offset ?? 0);
  for (const [channel, color] of [[0, "#315c59"], [1, "#3c7673"], [2, "#46908c"], [3, "#51aaa5"]]) {
    ctx.fillStyle = color;
    for (let x = 0; x < width; x += 2) {
      const a = Math.floor((songStart + x * span / width) / view.duration * bins.length);
      const b = Math.min(bins.length, Math.ceil((songStart + (x + 2) * span / width) / view.duration * bins.length));
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

// Object circles grow with zoom as in the design (f11): half a beat on screen × 0.8, kept
// between 18 and 30 px, from the timing at the object's own time and never taller than the lane.
const objectReach = 15;
let beatLengthCache = null;

// [start, beat length] of each timing, sorted by start: built once per timing snapshot.
function beatLengths(sections) {
  if (beatLengthCache?.sections !== sections) {
    const beats = (sections ?? []).filter(([, beat]) => beat > 0).map(([origin, beat]) => [origin, beat])
      .sort((first, second) => first[0] - second[0]);
    beatLengthCache = { sections, beats };
  }
  return beatLengthCache.beats;
}

function objectDiameter(beats, time, zoom, laneHeight) {
  // Binary search for the last timing starting at or before the time; before it, the first timing.
  let low = 0;
  let high = beats.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (beats[middle][0] <= time) low = middle + 1;
    else high = middle;
  }
  const beat = beats[Math.max(0, low - 1)]?.[1];
  const size = beat ? limit(Math.round(beat * zoom / 2 * .8), 18, 30) : 18;
  return Math.max(0, Math.min(size, laneHeight));
}

function drawGameplay(ctx, gameplay, xOf, y, width, diameterAt) {
  if (!gameplay) return;
  for (const [kind, start, end] of gameplay.spans) {
    const left = xOf(start);
    const right = xOf(end);
    if (right < -objectReach || left > width + objectReach) continue;
    if (kind === "slider") {
      const track = diameterAt(start) * .66;
      const top = y - track / 2;
      const bottom = top + track;
      const corner = Math.min(track, right - left) / 2;
      ctx.fillStyle = "#62d6cfd9";
      ctx.beginPath();
      if (ctx.roundRect) ctx.roundRect(left, top, right - left, track, corner);
      else {
        // Older macOS and Linux WebViews have no roundRect(); arcTo() draws the same shape.
        ctx.moveTo(left + corner, top);
        ctx.arcTo(right, top, right, bottom, corner);
        ctx.arcTo(right, bottom, left, bottom, corner);
        ctx.arcTo(left, bottom, left, top, corner);
        ctx.arcTo(left, top, right, top, corner);
        ctx.closePath();
      }
      ctx.fill();
      continue;
    }
    ctx.fillStyle = "#e6eef2";
    const from = Math.max(0, left);
    const to = Math.min(width, right);
    ctx.fillRect(from, y - 2.5, Math.max(1, to - from), 5);
  }
  const radiusOf = nodeRadii(gameplay.points, diameterAt);
  ctx.lineWidth = 2;
  ctx.strokeStyle = "#1c2727";
  for (const [kind, time, object] of gameplay.points) {
    const x = xOf(time);
    if (x < -objectReach || x > width + objectReach) continue;
    ctx.fillStyle = "#e6eef2";
    ctx.beginPath();
    ctx.arc(x, y, radiusOf(kind, time, object), 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  }
  ctx.lineWidth = 1;
}

// A slider's first and last nodes are full circles; repeats between them sit on the track.
function nodeRadii(points, diameterAt) {
  const sliderEnds = new Map();
  for (const [kind, time, object] of points) {
    if (kind !== "slider") continue;
    const ends = sliderEnds.get(object);
    if (ends) sliderEnds.set(object, [Math.min(ends[0], time), Math.max(ends[1], time)]);
    else sliderEnds.set(object, [time, time]);
  }
  return (kind, time, object) => {
    const ends = kind === "slider" ? sliderEnds.get(object) : null;
    const repeat = ends && time !== ends[0] && time !== ends[1];
    return repeat ? diameterAt(ends[0]) * .66 / 2 : diameterAt(time) / 2;
  };
}

function laneTint(lane, active = false) {
  if (!lane) return null;
  // Picked colour or none: an uncoloured lane's row stays grey (Yoge, ADR 0068). v3: 8 %, 16 % when active.
  const colour = laneColours.get(lane.colourKey);
  return colour ? colour + (active ? "29" : "14") : null;
}

const overviewGroups = ["soft", "normal", "drum", null, "skin"];
let overviewHitCache = null;

function drawOverviewHits({ ctx, width, height }, { moments, lanes }, duration) {
  if (width <= 0 || height <= 6 || duration <= 0) return;
  if (overviewHitCache?.moments !== moments || overviewHitCache.lanes !== lanes ||
      overviewHitCache.width !== width || overviewHitCache.duration !== duration) {
    const columns = overviewGroups.map(() => new Set());
    const laneBands = lanes?.map(lane => overviewGroups.indexOf(lane.group));
    const scale = width / duration;
    for (const [time, , , hits] of moments) {
      if (time < 0 || time > duration) continue;
      const x = Math.min(width - 1, Math.floor(time * scale));
      for (const [row] of hits) columns[laneBands?.[row]]?.add(x);
    }
    overviewHitCache = { moments, lanes, width, duration, columns };
  }

  const bandHeight = (height - 6) / overviewGroups.length;
  ctx.fillStyle = "#a3a3a3";
  for (const [band, columns] of overviewHitCache.columns.entries()) {
    const y = 3 + band * bandHeight;
    for (const x of columns) ctx.fillRect(x, y, 1.2, Math.max(1, bandHeight - 1));
  }
}

function paint() {
  if (!scene || $("workspace").hidden || $("editor-arrange").hidden) return;
  const rowHeight = scene.rowHeight;
  const contentHeight = Math.max($("scroll").clientHeight, laneRows.length * rowHeight);
  const scrollTop = $("scroll").scrollTop;
  const canvasHeight = Math.min(contentHeight, $("scroll").clientHeight);
  grid.style.height = canvasHeight + "px";
  const g = prepare(grid, canvasHeight);
  const r = prepare(ruler);
  const volumePanel = $("volume-controller-panel");
  const v = volumePanel.hidden || volumePanel.dataset?.layoutBarHidden !== undefined ? null : prepare(controller);
  const o = prepare(overview);
  const { start, zoom } = { start: scene.viewStart, zoom: scene.zoom };
  const xOf = (time) => (time - start) * zoom;
  const end = start + g.width / zoom;
  const range = scene.range;
  if (v) {
    v.ctx.fillStyle = slateTimeline.volume;
    v.ctx.fillRect(0, 0, v.width, v.height);
    v.ctx.font = '10px "Roboto Mono", monospace';
    for (const percent of [100, 50, 0]) {
      const y = v.height - 1 - percent * (v.height - 3) / 100;
      line(v.ctx, slateTimeline.beatLine, 0, y, v.width, y);
      v.ctx.fillStyle = "#a3a3a3";
      if (v.height >= 64) v.ctx.fillText(String(percent), 5, percent === 0 ? y - 3 : y + 11);
    }
  }
  for (let row = Math.floor(scrollTop / rowHeight); row <= Math.ceil((scrollTop + g.height) / rowHeight); row++) {
    const y = row * rowHeight - scrollTop;
    const lane = laneRows[row];
    g.ctx.fillStyle = lane === view.activeLane ? slateTimeline.activeRow : row % 2 ? slateTimeline.alternateRow : slateTimeline.arrange;
    if (lane === undefined) g.ctx.fillStyle = slateTimeline.arrange;
    else if (typeof lane === "string") g.ctx.fillStyle = slateTimeline.group;
    g.ctx.fillRect(0, y, g.width, rowHeight);
    const tint = typeof lane === "number" ? laneTint(scene.lanes?.[lane], lane === view.activeLane) : null;
    if (tint) {
      g.ctx.fillStyle = tint;
      g.ctx.fillRect(0, y, g.width, rowHeight);
    }
    if (lane !== undefined) line(g.ctx, slateTimeline.divider, 0, y + rowHeight - .5, g.width, y + rowHeight - .5);
  }
  if (range) {
    g.ctx.fillStyle = "#77bdfa35";
    const width = xOf(range[1]) - xOf(range[0]);
    if (markedRange?.path === scene.path) {
      for (const row of markedRange.rows) {
        for (const position of laneVisualPositions.get(row) ?? []) {
          g.ctx.fillRect(xOf(range[0]), position * rowHeight - scrollTop, width, rowHeight);
        }
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
      line(g.ctx, "#62d6cf65", objectPixel, 0, objectPixel, g.height);
      lastObjectPixel = objectPixel;
    }
    const colour = volumeColour(volume);
    if (v) {
      const volumeHeight = limit(volume, 0, 100) / 100 * (v.height - 3);
      v.ctx.fillStyle = colour;
      v.ctx.fillRect(x - 1, v.height - volumeHeight, 2, volumeHeight);
    }
    if (previewParents.size) {
      for (const [row, , listenMuted, hitVolume] of hits) {
        const parent = previewParents.get(row);
        if (parent === undefined) continue;
        const y = (lanePositions.get(parent) + .5) * rowHeight - scrollTop;
        if (y < -8 || y > g.height + 8) continue;
        g.ctx.save();
        g.ctx.globalAlpha *= .3;
        g.ctx.fillStyle = listenMuted ? "#858585" : volumeColour(hitVolume ?? volume);
        fillHitDiamond(g.ctx, x, y);
        g.ctx.restore();
      }
    }
    for (const [row, selected, listenMuted, hitVolume] of hits) {
      for (const visibleRow of laneVisualPositions.get(row) ?? []) {
        const y = (visibleRow + .5) * rowHeight - scrollTop;
        if (y < -8 || y > g.height + 8) continue;
        g.ctx.fillStyle = listenMuted ? "#858585" : volumeColour(hitVolume ?? volume);
        fillHitDiamond(g.ctx, x, y);
        if (selected) {
          g.ctx.lineWidth = 2;
          g.ctx.strokeStyle = "#ffffff";
          g.ctx.stroke();
          g.ctx.lineWidth = 1;
        }
      }
    }
  }
  if (drag?.surface === grid && drag.lastPoint && ["Select", "Range"].includes(view.tool)) {
    const x = drag.firstPoint.x;
    const y = drag.firstPoint.y - scrollTop;
    const width = drag.lastPoint.x - x;
    const height = drag.lastPoint.y - drag.firstPoint.y;
    if ((view.tool === "Range" || !drag.hit) && Math.hypot(width, height) > 3) {
      g.ctx.fillStyle = "#77bdfa14";
      g.ctx.fillRect(x, y, width, height);
      g.ctx.strokeStyle = "#77bdfa";
      g.ctx.strokeRect(x, y, width, height);
    }
  }
  if (v && scene.rampPreview?.length) {
    v.ctx.strokeStyle = "#62d6cf";
    v.ctx.beginPath();
    for (const [i, [time, volume]] of scene.rampPreview.entries()) {
      const x = xOf(time);
      const y = v.height - volume / 100 * (v.height - 3);
      if (i === 0) v.ctx.moveTo(x, y);
      else v.ctx.lineTo(x, y);
    }
    v.ctx.stroke();
  }
  r.ctx.fillStyle = slateTimeline.waveform;
  r.ctx.fillRect(0, 0, r.width, r.height);
  const waveformMiddle = (r.height + rulerBandHeight) / 2;
  const waveAmplitude = Math.min(waveformHeight, Math.max(0, r.height - rulerBandHeight)) / 2;
  if (waveAmplitude > 0) waveform(r.ctx, r.width, waveformMiddle, waveAmplitude, start, r.width / zoom);
  const beats = beatLengths(scene.sections);
  const objectSize = time => objectDiameter(beats, time, zoom, r.height - rulerBandHeight);
  drawGameplay(r.ctx, scene.gameplay, xOf, waveformMiddle, r.width, objectSize);
  r.ctx.fillStyle = slateTimeline.ruler;
  r.ctx.fillRect(0, 0, r.width, rulerBandHeight);
  r.ctx.font = "11px Consolas, monospace";
  drawTimingGrid(r.ctx, start, end, zoom, 0, rulerBandHeight, true);
  const rulerSteps = [25, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000, 60000];
  const labelStep = rulerSteps.find((step) => step * zoom >= 76) ?? 120000;
  for (let time = Math.ceil(start / labelStep) * labelStep; !scene.editorBars?.length && time < end; time += labelStep) {
    const x = xOf(time);
    line(r.ctx, slateTimeline.barLine, x, 3, x, 21);
    r.ctx.fillStyle = slateTimeline.rulerBar;
    r.ctx.fillText(clock(time), x + 5, 14);
  }
  const preview = rulerDrag?.path === scene.path ? rulerDrag.preview : null;
  const [left, right, cycling] = preview ?? [view.left, view.right, view.cycle];
  const spanStart = xOf(Math.min(left, right));
  const spanWidth = xOf(Math.max(left, right)) - spanStart;
  if (cycling && left <= right) {
    // The design's loop region: a faint accent band under a warning-yellow bar.
    r.ctx.fillStyle = slateTimeline.loop;
    r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
    r.ctx.fillStyle = slateTimeline.cycle;
    r.ctx.fillRect(spanStart, 1, spanWidth, 4);
  } else {
    const colour = left > right ? "#f0aa70" : slateTimeline.locator;
    r.ctx.fillStyle = colour + (cycling ? "46" : "22");
    r.ctx.fillRect(spanStart, 0, spanWidth, rulerBandHeight - 1);
    r.ctx.fillStyle = colour;
    r.ctx.fillRect(spanStart, 0, spanWidth, 2);
  }
  for (const time of [left, right]) {
    line(r.ctx, slateTimeline.locator, xOf(time), 0, xOf(time), r.height);
  }
  drawLocatorFlags(r.ctx, r.width, left, right, cycling && left <= right);
  drawBookmarks(r.ctx, scene.bookmarks ?? [], start, zoom, r.width);
  line(g.ctx, slateTimeline.playhead, xOf(view.position), 0, xOf(view.position), g.height);
  line(r.ctx, slateTimeline.playhead, xOf(view.position), 0, xOf(view.position), r.height);
  rulerWarningMarkers = visibleWarningMarkers(view.outputProblems ?? [], scene.path, start, zoom, r.width);
  refreshRulerWarningTooltip();
  r.ctx.fillStyle = "#f0aa70";
  for (const marker of rulerWarningMarkers) {
    r.ctx.fillRect(marker.x - 3, r.height - 9, 6, 6);
  }
  if (v) line(v.ctx, slateTimeline.playhead, xOf(view.position), 0, xOf(view.position), v.height);
  o.ctx.fillStyle = slateTimeline.overview;
  o.ctx.fillRect(0, 0, o.width, o.height);
  waveform(o.ctx, o.width, o.height / 2, o.height / 2 - 2, 0, view.duration);
  drawOverviewHits(o, scene, view.duration);
  if (view.duration > 0) {
    const overviewX = start * o.width / view.duration;
    const overviewWidth = (g.width / zoom) * o.width / view.duration;
    o.ctx.fillStyle = "#ffffff14";
    o.ctx.fillRect(overviewX, 0, overviewWidth, o.height);
    o.ctx.strokeStyle = "#858585";
    o.ctx.strokeRect(overviewX, .5, overviewWidth, o.height - 1);
    const playX = view.position * o.width / view.duration;
    line(o.ctx, slateTimeline.playhead, playX, 0, playX, o.height);
  }
}

function pointer(event, phase, gesture = drag) {
  const rect = gesture.surface.getBoundingClientRect();
  const x = event.clientX - rect.x;
  const valueLane = gesture.surface === controller;
  const y = event.clientY - rect.y + (valueLane || gesture.panning ? 0 : $("scroll").scrollTop);
  const rowUnderPointer = Math.floor(y / scene.rowHeight);
  const visibleRow = phase === "double_click" ? rowUnderPointer : limit(rowUnderPointer, 0, gesture.rows.length - 1);
  const lane = gesture.rows[visibleRow];
  if (typeof lane === "number") {
    gesture.lastRow = lane;
    gesture.lastVisualRow = visibleRow;
  }
  const row = valueLane || gesture.panning ? view.activeLane : gesture.lastRow;
  let ms = gesture.start + x / gesture.zoom;
  if ((phase === "down" || phase === "double_click") && !valueLane &&
      ["Select", "Erase", "Mute", "Ramp"].includes(view.tool)) {
    let best = 6.5 / gesture.zoom;
    for (const [time, , , hits] of scene.moments) {
      if (Math.abs(time - ms) < best && hits.some(([lane]) => row === lane)) {
        best = Math.abs(time - ms);
        gesture.hit = time;
      }
    }
    if (gesture.hit != null) ms = gesture.hit;
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

let cursorSurface = null;
let cursorAltDraw = false;

function refreshToolCursor() {
  if (!cursorSurface) return;
  const tool = view.tool === "Grab" || !cursorAltDraw ? view.tool : "Draw";
  const cursor = {
    Select: "default",
    Grab: "pointer",
    Draw: 'url("studio-assets/icons/cursor-draw.svg") 4 4, crosshair',
    Erase: 'url("studio-assets/icons/cursor-erase.svg") 5 27, crosshair',
  }[tool] ?? "crosshair";
  if (cursorSurface.style.cursor !== cursor) cursorSurface.style.cursor = cursor;
}

function updateCursorModifiers(event) {
  const altDraw = event.altKey && !event.ctrlKey && !event.metaKey && !event.getModifierState?.("AltGraph");
  if (altDraw === cursorAltDraw) return;
  cursorAltDraw = altDraw;
  refreshToolCursor();
}

window.addEventListener?.("keydown", updateCursorModifiers, true);
window.addEventListener?.("keyup", updateCursorModifiers, true);
window.addEventListener?.("blur", () => {
  cursorAltDraw = false;
  refreshToolCursor();
});

for (const surface of [grid, controller]) {
  const trackToolCursor = (event) => {
    cursorSurface = surface;
    updateCursorModifiers(event);
    refreshToolCursor();
  };
  surface.addEventListener("pointerenter", trackToolCursor);
  surface.addEventListener("pointermove", trackToolCursor);
  surface.addEventListener("pointerleave", () => {
    if (cursorSurface === surface) cursorSurface = null;
  });
}

for (const surface of [grid, controller]) {
  surface.addEventListener("pointerdown", (event) => {
    if (!scene || view.busy || drag || rulerDrag || event.button !== 0) return;
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
    drag.firstVisualRow = drag.lastVisualRow;
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
        const first = drag.firstVisualRow ?? -1;
        const last = drag.lastVisualRow ?? -1;
        markedRange = moved && first >= 0 && last >= 0 ? {
          path: scene.path,
          rows: [...new Set(drag.rows.slice(Math.min(first, last), Math.max(first, last) + 1)
            .filter(row => typeof row === "number"))],
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
grid.addEventListener("dblclick", (event) => {
  if (!scene || view.busy || drag || view.tool !== "Select" || event.button !== 0 ||
      event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
  const gesture = { surface: grid, start: scene.viewStart, zoom: scene.zoom, rows: laneRows };
  const point = pointer(event, "double_click", gesture);
  if (Number.isInteger(point.row)) {
    event.preventDefault();
    act(point);
  }
});
$("scroll").addEventListener("scroll", paintSoon);
new ResizeObserver(paintSoon).observe($("scroll"));

const waveformGrip = $("waveform-resize");
let waveformDrag = null;

function setWaveformHeight(requested) {
  waveformHeight = limit(Math.round(requested), 0, 200);
  if (waveformHeight) visibleWaveformHeight = waveformHeight;
  $("ruler-waveform").setAttribute?.("aria-pressed", String(waveformHeight > 0));
  $("editor-arrange").style.setProperty("--wave-height", waveformHeight + "px");
  waveformGrip.setAttribute("aria-valuenow", String(waveformHeight));
  waveformGrip.setAttribute("aria-valuetext", waveformHeight ? waveformHeight + " pixels" : "Waveform hidden");
  paintSoon();
}

function finishWaveformDrag(cancel) {
  const previous = waveformDrag;
  waveformDrag = null;
  delete waveformGrip.dataset.dragging;
  if (cancel) setWaveformHeight(previous.height);
  if (waveformGrip.hasPointerCapture(previous.id)) waveformGrip.releasePointerCapture(previous.id);
}

waveformGrip.addEventListener("pointerdown", event => {
  if (event.button !== 0 || drag || rulerDrag || waveformDrag) return;
  const growsDown = !waveformGrip.parentElement?.hasAttribute("data-layout-size-from-top");
  waveformDrag = { id: event.pointerId, y: event.clientY, height: waveformHeight, direction: growsDown ? 1 : -1 };
  waveformGrip.dataset.dragging = "";
  waveformGrip.setPointerCapture(event.pointerId);
});
waveformGrip.addEventListener("pointermove", event => {
  if (waveformDrag && waveformDrag.id === event.pointerId) {
    setWaveformHeight(waveformDrag.height + (event.clientY - waveformDrag.y) * waveformDrag.direction);
  }
});
for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
  waveformGrip.addEventListener(type, event => {
    if (waveformDrag && waveformDrag.id === event.pointerId) {
      if (type === "pointerup") setWaveformHeight(waveformDrag.height + (event.clientY - waveformDrag.y) * waveformDrag.direction);
      finishWaveformDrag(type !== "pointerup");
    }
  });
}
waveformGrip.addEventListener("dblclick", () => setWaveformHeight(defaultWaveformHeight));
waveformGrip.addEventListener("keydown", event => {
  if (event.key === "Escape" && waveformDrag) {
    finishWaveformDrag(true);
  } else {
    const step = waveformGrip.parentElement?.hasAttribute("data-layout-size-from-top") ? -8 : 8;
    const heights = { ArrowUp: waveformHeight - step, ArrowDown: waveformHeight + step,
      Home: 0, End: 200, Enter: defaultWaveformHeight };
    if (!Object.hasOwn(heights, event.key)) return;
    setWaveformHeight(heights[event.key]);
  }
  event.preventDefault();
});

let rulerPointer = null;

function refreshRulerCursor() {
  let gesture = rulerDrag?.mode;
  if (!gesture && scene && rulerPointer) {
    const { left, top, width } = ruler.getBoundingClientRect();
    const x = rulerPointer.clientX - left;
    const y = rulerPointer.clientY - top;
    if (x >= 0 && x < width && y >= 0 && y < rulerBandHeight) gesture = rulerGestureAt(x, y);
  }
  const cursor = gesture === "left" || gesture === "right" ? "ew-resize"
    : gesture === "move" ? "pointer" : "default";
  if (ruler.style.cursor !== cursor) ruler.style.cursor = cursor;
}

function trackRulerPointer(event) {
  const bounds = ruler.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  rulerPointer = { clientX: event.clientX, clientY: event.clientY };
  refreshRulerCursor();
  if (rulerDrag && rulerDrag.id === event.pointerId) {
    rulerDrag.preview = locatorPreview(rulerDrag, x);
    paintSoon();
    return;
  }
  refreshRulerWarningTooltip(bounds);
}

function refreshRulerWarningTooltip(bounds) {
  let nearest = null;
  if (rulerPointer && !rulerDrag && rulerWarningMarkers.length) {
    const rect = bounds ?? ruler.getBoundingClientRect();
    const x = rulerPointer.clientX - rect.left;
    const y = rulerPointer.clientY - rect.top;
    if (x >= 0 && x < rect.width && y >= rect.height - 13 && y < rect.height) {
      let distance = Infinity;
      for (const marker of rulerWarningMarkers) {
        const gap = Math.abs(marker.x - x);
        if (gap <= 7 && gap < distance) {
          nearest = marker;
          distance = gap;
        }
      }
    }
  }
  const nearby = nearest?.problems ?? [];
  const lines = nearby.slice(0, 8).map(describeOutputProblem);
  if (nearby.length > lines.length) lines.push("…and " + (nearby.length - lines.length) + " more.");
  const title = lines.join("\n");
  if (ruler.title !== title) ruler.title = title;
}
ruler.addEventListener("pointerenter", trackRulerPointer);
ruler.addEventListener("pointermove", trackRulerPointer);
ruler.addEventListener("pointerleave", () => {
  ruler.title = "";
  rulerPointer = null;
  refreshRulerCursor();
});
function waveformTarget(ms, y, height) {
  const dy = Math.abs(y - (height + rulerBandHeight) / 2);
  if (!scene.gameplay || dy > objectReach) return null;
  // Hit the circles and slider tracks at the sizes drawGameplay() draws them.
  const beats = beatLengths(scene.sections);
  const diameterAt = time => objectDiameter(beats, time, scene.zoom, height - rulerBandHeight);
  const radiusOf = nodeRadii(scene.gameplay.points, diameterAt);
  let target = null;
  let nearest = Infinity;
  for (const [kind, time, object] of scene.gameplay.points) {
    const distance = Math.hypot((time - ms) * scene.zoom, dy);
    if (distance > objectReach || distance > radiusOf(kind, time, object)) continue;
    if (distance < nearest) {
      target = { object, time, node: true };
      nearest = distance;
    }
  }
  if (target) return target;
  // The last matching span wins; a reverse loop, as older WebKit WebViews lack findLast().
  const spans = scene.gameplay.spans;
  for (let i = spans.length - 1; i >= 0; i--) {
    const [kind, start, end, object] = spans[i];
    if (start <= ms && ms <= end && dy <= (kind === "slider" ? diameterAt(start) * .66 / 2 : 2.5)) {
      return { object, time: start, node: false };
    }
  }
  return null;
}

function bookmarkNearFlag(ms, y) {
  if (y >= rulerBandHeight + bookmarkFlagHeight) return null;
  let nearest = 5 / scene.zoom;
  let bookmark = null;
  for (const mark of scene.bookmarks ?? []) {
    const distance = Math.abs(mark.time - ms);
    if (distance <= nearest) {
      nearest = distance;
      bookmark = mark;
    }
  }
  return bookmark;
}

const locatorFlagWidth = 14;
const locatorFlagHeight = 13;

// The L and R flags at the top of the ruler: L hangs right of its line, R left of its line. Near an
// edge a flag flips to stay on screen; on a range too short for both, they hang outward. A flag
// never covers the other one.
function locatorFlags(width, leftTime = view.left, rightTime = view.right) {
  const flag = (time, side) => {
    const x = (time - scene.viewStart) * scene.zoom;
    const from = side > 0 ? x : x - locatorFlagWidth;
    const flipped = from < 0 || from + locatorFlagWidth > width;
    return { x, from: flipped ? (side > 0 ? x - locatorFlagWidth : x) : from, flipped };
  };
  const left = flag(leftTime, 1);
  const right = flag(rightTime, -1);
  const overlap = () => Math.abs(left.from - right.from) < locatorFlagWidth;
  if (overlap()) {
    if (left.flipped) left.from = right.from - locatorFlagWidth;
    else if (right.flipped) right.from = left.from + locatorFlagWidth;
    else {
      if (left.x >= locatorFlagWidth) left.from = left.x - locatorFlagWidth;
      if (right.x + locatorFlagWidth <= width) right.from = right.x;
      if (overlap()) right.from = left.from + locatorFlagWidth;
    }
  }
  return { left, right };
}

function rulerGestureAt(x, y = Infinity) {
  if (y < locatorFlagHeight) {
    const flags = locatorFlags(ruler.clientWidth || ruler.getBoundingClientRect().width);
    for (const side of ["left", "right"]) {
      if (x >= flags[side].from && x <= flags[side].from + locatorFlagWidth) return side;
    }
  }
  const left = (view.left - scene.viewStart) * scene.zoom;
  const right = (view.right - scene.viewStart) * scene.zoom;
  if (left === right) return "draw";
  if (Math.abs(x - left) <= 5) return "left";
  if (Math.abs(x - right) <= 5) return "right";
  return x > Math.min(left, right) && x < Math.max(left, right) ? "move" : "draw";
}

function locatorPreview(gesture, x) {
  if (Math.abs(x - gesture.x) < 3) return null;
  const { left, right, cycling, duration } = gesture;
  const to = limit(gesture.start + x / gesture.zoom, 0, duration);
  if (gesture.mode === "left" || gesture.mode === "right") {
    const moved = locatorDragTo(gesture, x);
    return gesture.mode === "left" ? [moved, right, cycling] : [left, moved, cycling];
  }
  if (gesture.mode === "move") {
    const shift = limit((x - gesture.x) / gesture.zoom, -Math.min(left, right), Math.max(0, duration - Math.max(left, right)));
    return [left + shift, right + shift, cycling];
  }
  const from = limit(gesture.start + gesture.x / gesture.zoom, 0, duration);
  return [Math.min(from, to), Math.max(from, to), true];
}

// A locator follows the pointer's distance from where its flag or line was grabbed, and stops at
// the other locator: L never passes R, R never passes L.
function locatorDragTo(gesture, x) {
  const to = gesture[gesture.mode] + (x - gesture.x) / gesture.zoom;
  // Locators already reversed by the set shortcuts drag freely, as the backend lets them.
  const ordered = gesture.left <= gesture.right;
  if (gesture.mode === "left") return limit(to, 0, ordered ? gesture.right : gesture.duration);
  return limit(to, ordered ? gesture.left : 0, gesture.duration);
}

function endRulerDrag(event) {
  const current = rulerDrag;
  rulerDrag = null;
  if (ruler.hasPointerCapture(current.id)) ruler.releasePointerCapture(current.id);
  rulerPointer = { clientX: event.clientX, clientY: event.clientY };
  refreshRulerCursor();
  paintSoon();
  return current;
}

ruler.addEventListener("pointerdown", (event) => {
  if (!scene || view.busy || drag || rulerDrag || event.button !== 0) return;
  const bounds = ruler.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;
  const ms = scene.viewStart + x / scene.zoom;
  const side = event.ctrlKey || event.metaKey ? "left" : event.altKey ? "right" : null;
  const flag = event.shiftKey && !side && y < rulerBandHeight ? rulerGestureAt(x, y) : null;
  if (flag === "left" || flag === "right") {
    act({ kind: "locator", end: flag }, false);
  } else if (side) {
    act({ kind: "ruler_locators", path: scene.path, gesture: side, from: ms, to: ms }, false);
  } else if (y < rulerBandHeight) {
    ruler.setPointerCapture(event.pointerId);
    rulerDrag = { path: scene.path, id: event.pointerId, x, start: scene.viewStart,
      zoom: scene.zoom, mode: rulerGestureAt(x, y), left: view.left, right: view.right, cycling: view.cycle,
      duration: view.duration };
    refreshRulerCursor();
  } else {
    const bookmark = bookmarkNearFlag(ms, y);
    const target = bookmark ? null : waveformTarget(ms, y, bounds.height);
    if (target) act({ kind: "waveform_object", path: scene.path, ...target, preview: false });
    else act({ kind: "seek", ms: limit(bookmark?.time ?? ms, 0, view.duration) }, false);
  }
});
ruler.addEventListener("dblclick", event => {
  if (!scene || view.busy || drag || rulerDrag || event.button !== 0 ||
      event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
  const bounds = ruler.getBoundingClientRect();
  const ms = scene.viewStart + (event.clientX - bounds.left) / scene.zoom;
  const y = event.clientY - bounds.top;
  if (bookmarkNearFlag(ms, y)) return;
  const target = waveformTarget(ms, y, bounds.height);
  if (target) act({ kind: "waveform_object", path: scene.path, ...target, preview: true });
});
ruler.addEventListener("pointerup", event => {
  if (!rulerDrag || rulerDrag.id !== event.pointerId) return;
  const gesture = endRulerDrag(event);
  const x = event.clientX - ruler.getBoundingClientRect().left;
  const moved = Math.abs(x - gesture.x) >= 3;
  if (!moved && gesture.mode !== "move") return;
  const locator = gesture.mode === "left" || gesture.mode === "right";
  act({ kind: "ruler_locators", path: gesture.path, gesture: moved ? gesture.mode : "toggle",
    from: gesture.start + gesture.x / gesture.zoom,
    to: locator ? locatorDragTo(gesture, x) : gesture.start + x / gesture.zoom }, false);
});
for (const type of ["pointercancel", "lostpointercapture"]) {
  ruler.addEventListener(type, event => {
    if (rulerDrag && rulerDrag.id === event.pointerId) endRulerDrag(event);
  });
}
async function drainOverviewNavigation() {
  while (overviewTarget) {
    const { path, fraction } = overviewTarget;
    overviewTarget = null;
    if (scene?.path !== path || view.busy) continue;
    const zoom = scene.zoom;
    const start = timelineStart(fraction * view.duration - grid.clientWidth / zoom / 2, zoom);
    if (start !== scene.viewStart) await changeView(start, zoom);
  }
}

function panOverview(event) {
  if (!overviewDrag || overviewDrag.id !== event.pointerId ||
      overviewDrag.path !== scene?.path || !overview.clientWidth) return;
  const fraction = limit((event.clientX - overview.getBoundingClientRect().left) / overview.clientWidth, 0, 1);
  overviewTarget = { path: overviewDrag.path, fraction };
  // Keep the latest position while the previous viewport update is pending.
  overviewNavigation ??= drainOverviewNavigation().finally(() => { overviewNavigation = null; });
}

overview.addEventListener("pointerdown", event => {
  if (!scene || view.busy || drag || rulerDrag || overviewDrag || event.button !== 0) return;
  overviewDrag = { id: event.pointerId, path: scene.path };
  overview.setPointerCapture(event.pointerId);
  panOverview(event);
});
overview.addEventListener("pointermove", panOverview);
overview.addEventListener("pointerup", event => {
  if (overviewDrag?.id !== event.pointerId) return;
  panOverview(event);
  overviewDrag = null;
});
for (const type of ["pointercancel", "lostpointercapture"]) {
  overview.addEventListener(type, event => {
    if (overviewDrag?.id !== event.pointerId) return;
    overviewDrag = null;
    overviewTarget = null;
  });
}

function timelineStart(start, zoom) {
  return limit(start, 0, view.duration - grid.clientWidth / zoom);
}

function followPlayhead() {
  if (!view.follow || !view.playing || view.busy || drag || rulerDrag || overviewDrag || !grid.clientWidth) return;
  const visible = grid.clientWidth / scene.zoom;
  // The Playhead group's Centre (window only): the view moves so the playhead stays in the middle.
  if (window.dawsuUi?.centre?.()) {
    const start = timelineStart(view.position - visible / 2, scene.zoom);
    if (Math.abs(start - scene.viewStart) * scene.zoom >= 1) changeView(start, scene.zoom);
    return;
  }
  if (view.position >= scene.viewStart && view.position <= scene.viewStart + visible * .98) return;
  const start = timelineStart(view.position - visible * .02, scene.zoom);
  if (start !== scene.viewStart) changeView(start, scene.zoom);
}

function resizeLaneHeaders(height) {
  $("lanes").style.setProperty("--lane-row-height", height + "px");
}

// The farthest zoom out, in px per ms, as the editor's MIN_ZOOM; the zoom slider's min is its log10.
const minZoom = .0005;

function changeView(start, zoom, rowHeight = scene.rowHeight, rowAnchorY = 0) {
  zoom = limit(zoom, minZoom, 3);
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

// Like osu!lazer, the wheel steps through the snap group the slider shows (Common 1/1-1/16,
// Triplets 1/1-1/12, or a custom one) and stops at either end (ui/src/lib/snap.js).
function stepWheelSnap(finer) {
  const divisor = window.dawsuUi?.wheelSnap?.(finer);
  if (divisor) act({ kind: "snap", on: scene.snap, mode: scene.snapMode, beats: 1, divisor });
}

// The Triplets box shows the group; ticking it picks 1/6 and unticking 1/4, as lazer's group button does.
$("triplet-snap").addEventListener("change", () => {
  if (scene) act({ kind: "snap", on: scene.snap, mode: scene.snapMode, beats: 1, divisor: $("triplet-snap").checked ? 6 : 4 });
});

$("scroll").addEventListener("wheel", (event) => {
  if (!scene) return;
  const { ctrlKey, shiftKey, altKey, deltaX, deltaY } = event;
  const osu = view.keyPreset === "osu!";
  const x = event.clientX - grid.getBoundingClientRect().x;
  const rows = ctrlKey && shiftKey;
  const horizontal = deltaX && !deltaY;
  const snap = !horizontal && (osu ? ctrlKey : altKey && !ctrlKey);
  const zoomTime = !horizontal && (osu ? altKey && !ctrlKey : ctrlKey);
  const pan = horizontal || (!osu && shiftKey);
  const seek = osu && (shiftKey || x >= 0);
  if (!(rows || snap || zoomTime || pan || seek) || (!deltaX && !deltaY)) return;
  event.preventDefault();
  if (requests > 1 || view.busy || drag) return;
  if (rows) {
    if (!deltaY) return;
    const height = Math.round(scene.rowHeight * (deltaY < 0 ? 1.15 : 1 / 1.15));
    const anchor = event.clientY - $("scroll").getBoundingClientRect().top;
    changeView(scene.viewStart, scene.zoom, height, anchor);
  } else if (snap) {
    stepWheelSnap(deltaY < 0);
  } else if (zoomTime) {
    const zoom = limit(scene.zoom * Math.exp(-deltaY * .003), minZoom, 3);
    changeView(scene.viewStart + Math.max(0, x) * (1 / scene.zoom - 1 / zoom), zoom);
  } else if (pan) {
    changeView(scene.viewStart + (deltaX || deltaY) / scene.zoom, scene.zoom);
  } else {
    const id = (deltaY < 0 ? "seekBack" : "seekForward") + (shiftKey ? "Far" : "");
    act({ kind: "command", id }, false);
  }
}, { passive: false });

function modal(kind, title, content, buttons, cancel = null) {
  if (modalKind === kind && dialog.open) return;
  modalKind = kind;
  modalClose = cancel;
  dialog.className = "dialog-" + kind;
  $("dialog-title").textContent = title;
  $("dialog-body").replaceChildren(...content);
  $("dialog-actions").replaceChildren(...buttons.map(([label, task, primary]) => {
    const button = makeButton(label, task);
    if (primary) button.className = "primary";
    return button;
  }));
  if (!dialog.open) dialog.showModal();
}
// The Inspector's Lane colour (and the lane menu, by its id): opens on grey until the mapper picks a colour.
function openLaneColour() {
  const path = scene?.path;
  const lane = scene?.lanes?.[view.activeLane];
  if (!lane?.colourKey || view.busy || drag) return;
  const picked = laneColours.get(lane.colourKey);
  // Apply writes only a colour the mapper chose, so an unpicked lane stays without one.
  let chosen = !!picked;
  const input = document.createElement("input");
  input.type = "color";
  input.value = picked ?? laneGrey;
  const label = document.createElement("label");
  label.append("Custom colour", input);
  const palette = document.createElement("div");
  palette.className = "tint-palette";
  palette.setAttribute("role", "radiogroup");
  palette.setAttribute("aria-label", "Lane colour");
  const preview = document.createElement("div");
  preview.className = "tint-preview";
  const caption = document.createElement("span");
  caption.textContent = lane.name;
  const sample = document.createElement("small");
  sample.textContent = lane.sample ?? "";
  caption.append(sample);
  preview.append(caption);
  for (let i = 0; i < 5; i++) preview.append(document.createElement("i"));
  const refresh = () => {
    preview.style.setProperty("--preview-colour", input.value);
    for (const radio of palette.children) radio.checked = radio.value === input.value;
  };
  const shades = [
    "d04932 db642b d78628 deab27 c9bc30 a6be32 70be3d 44ba4b 23b76c 21af91 24abb2 289dc9 388ada 5776d4 7464ce 945ac5 b751bb cc50a2",
    "b0412e b95728 b57326 bb9125 aa9f2c 8da12e 61a137 3d9e43 229b5e 20957c 239197 2686aa 3376b8 4c66b3 6457ae 7e4fa7 9b479e ac478a",
    "91392b 984b26 966124 9a7923 8d8429 76862b 538632 36833b 218150 1f7c68 21797e 24708d 2e6498 435794 554b90 6a458a 813f84 8f3e73",
    "74342a 794126 775125 7a6224 706a29 606b2a 476b2f 326935 226845 226456 236266 255c70 2c5378 3b4976 494173 583c6f 68386a 72385e",
  ];
  for (const shade of shades.join(" ").split(" ")) {
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "lane-swatch";
    radio.value = "#" + shade;
    radio.title = radio.value;
    radio.setAttribute("aria-label", radio.value);
    radio.style.backgroundColor = radio.value;
    radio.addEventListener("change", () => { input.value = radio.value; chosen = true; refresh(); });
    palette.append(radio);
  }
  input.addEventListener("input", () => { chosen = true; refresh(); });
  refresh();
  const change = (colour) => {
    closeModal();
    if (scene?.path !== path || view.busy || drag) return;
    if (colour) laneColours.set(lane.colourKey, colour);
    else laneColours.delete(lane.colourKey);
    renderActiveLane();
    renderLanes();
    paintSoon();
    saveLaneLayout(true);
  };
  const buttons = [["Cancel", closeModal]];
  if (laneColours.has(lane.colourKey)) buttons.push(["Clear colour", () => change(null)]);
  buttons.push(["Apply", () => (chosen ? change(input.value) : closeModal()), true]);
  modal("lane-colour", "Lane colour · " + lane.name, [palette, preview, label], buttons);
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

function outputProblemTime(problem) {
  return (problem.time < 0 ? "−" : "") + clock(Math.abs(problem.time));
}

function describeOutputProblem(problem) {
  return outputProblemTime(problem) + " · " + nameOf(problem.target) + " · " + problem.message;
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
    const problems = view.outputProblems ?? [];
    const dismiss = () => { closeModal(); return act({ kind: "dismiss_save_problem_review" }); };
    const showProblem = (index) => async () => { await dismiss(); await showOutputProblem(index); };
    // With a reviewed plan, Save writes everything it can and leaves these moments out; without a
    // project nothing could keep them, so Save still refuses.
    const reviewed = Boolean(view.saveReview);
    const save = reviewed
      ? () => { closeModal(); act({ kind: "answer_save_review", apply: true }); }
      : () => { closeModal(); act({ kind: "save" }); };
    const difficulty = document.createElement("div");
    difficulty.className = "save-problem-difficulty";
    const label = document.createElement("span");
    label.textContent = "Difficulty";
    const file = document.createElement("strong");
    file.textContent = nameOf(scene?.path ?? "");
    difficulty.append(label, file);
    const list = document.createElement("div");
    list.className = "save-problem-list";
    list.setAttribute("role", "list");
    list.append(...problems.map((problem, index) => {
      const row = document.createElement("div");
      row.className = "save-problem";
      row.setAttribute("role", "listitem");
      const message = document.createElement("span");
      message.textContent = problem.message;
      if (problem.target !== scene?.path) {
        const target = document.createElement("small");
        target.className = "save-problem-target";
        target.textContent = nameOf(problem.target);
        message.append(target);
      }
      const time = document.createElement("time");
      time.textContent = outputProblemTime(problem);
      const button = makeButton("Show", showProblem(index));
      button.setAttribute("aria-label", "Show the problem at " + time.textContent);
      row.append(message, time, button);
      return row;
    }));
    const note = paragraph(reviewed
      ? "These parts won't sound in osu! the way they do here. You can still save; the rest is written as drawn. " +
        "Left-out moments stay in dawsu! and on this list, and Revert to saved brings them back."
      : "These moments can't be written to the difficulty as drawn. Save writes nothing until they are fixed.");
    modal("save-problems", "Save hitsounds", [difficulty, note, list], reviewed
      ? [["Show first problem", showProblem(0)], ["Cancel", dismiss], ["Save", save, true]]
      : [["Cancel", dismiss], ["Show first problem", showProblem(0)], ["Continue to Save", save, true]], dismiss);
    const first = $("dialog-actions").children[0];
    if (reviewed && first) first.className = "save-problems-show";
    const count = document.createElement("span");
    count.className = "problem-count";
    count.textContent = problems.length + (problems.length === 1 ? " problem" : " problems");
    $("dialog-title").replaceChildren("Save hitsounds", count);
  } else if (view.saveReview) {
    const answer = (apply) => { closeModal(); act({ kind: "answer_save_review", apply }); };
    const files = view.saveReview.map(([operation, path]) => paragraph(operation + " · " + path));
    modal("save-review", "Review Save", [
      paragraph("Editing " + nameOf(scene.path)),
      ...files.length ? files : [paragraph("No files need to change.")],
    ], [["Cancel", () => answer(false)], [files.length ? "Save" : "Done", () => answer(true), true]], () => answer(false));
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
      const label = paragraph(path);
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
    const source = scene ? [paragraph("Editing " + nameOf(scene.path))] : [];
    modal("review", view.review.label, [...source, heading, ...files, ...mixSection, ...details],
      [["Cancel", () => answer(false)], ...choices, ["Write files", () => answer(true), true]], () => answer(false));
  } else if (view.editDialog && scene) {
    showTimelineEdit(view.editDialog);
  } else if (scene && view.soundSetPrompts?.length && pendingSoundSetPrompt()) {
    showSoundSetPrompt(pendingSoundSetPrompt());
  } else if (view.firstLaunch) {
    const choose = (name) => act({ kind: "key_preset", name });
    modal("key-preset", "Choose your keyboard preset", [
      paragraph("DAW-like uses the editor’s default timeline shortcuts."),
      paragraph("osu! lets you seek through the beat grid with the arrow keys."),
    ], [["DAW-like", () => choose("DAW"), true], ["osu!", () => choose("osu!")]], () => {});
  } else if (["error", "pending", "pending-home", "revert", "review", "save-review", "save-problems", "key-preset", "timeline-edit", "sound-set-update"].includes(modalKind)) {
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

function openReplaceSample() {
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
}

const open = () => run(async () => show(await invoke("pick_map")));
$("open").addEventListener("click", open);
$("home-open").addEventListener("click", open);
$("go-home").addEventListener("click", () => { if (!drag && !view.busy) act({ kind: "home" }); });
$("home-resume").addEventListener("click", () => { if (!view.busy) act({ kind: "resume" }); });
$("home-import").addEventListener("click", () => run(async () => show(await invoke("pick_map", { archiveOnly: true }))));
$("home-folders").addEventListener("click", () => $("folders").click());
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

for (const button of document.querySelectorAll("[data-action]")) {
  button.addEventListener("click", () => act({ kind: button.dataset.action }));
}
for (const button of document.querySelectorAll("[data-command]")) {
  button.addEventListener("click", () => command(button.dataset.command));
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
toggle("object-lines", "object_lines");
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
function openSiblingDifficulty(path) {
  if (scene && !view.busy && !drag && path && path !== scene.path) act({ kind: "open", path });
}

function chooseSiblingDifficulty() {
  const ribbon = $("difficulty");
  if (!scene || !ribbon.getClientRects().length || !ribbon.options.length) return false;
  const currentPath = scene.path;
  const choices = document.createElement("select");
  choices.id = "difficulty-choice";
  choices.size = Math.min(8, ribbon.options.length);
  choices.setAttribute("aria-label", "Difficulty in the current mapset");
  choices.append(...[...ribbon.options].map(option => new Option(option.text, option.value)));
  choices.value = currentPath;
  const open = () => {
    closeModal();
    if (scene?.path === currentPath) openSiblingDifficulty(choices.value);
  };
  choices.addEventListener("keydown", event => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    open();
  });
  choices.addEventListener("dblclick", open);
  modal("difficulty-chooser", "Switch difficulty", [choices],
    [["Cancel", closeModal], ["Open", open, true]]);
  choices.focus();
  return true;
}

$("difficulty").addEventListener("change", () => {
  if (!scene) return;
  const path = $("difficulty").value;
  $("difficulty").value = scene.path;
  openSiblingDifficulty(path);
});
function fitTimeline() {
  if (scene && !view.busy && view.duration > 0 && grid.clientWidth > 0) {
    changeView(0, grid.clientWidth / view.duration);
  }
}
$("fit").addEventListener("click", fitTimeline);
$("ruler-fit").addEventListener("click", fitTimeline);
$("toggle-lane-names").addEventListener("click", () => {
  if (scene && !view.busy) act({ kind: "panel", panel: "lane_names", on: view.panels?.laneNames === false });
});
$("ruler-waveform").addEventListener("click", () => {
  if (scene && !view.busy && !waveformDrag) {
    setWaveformHeight(waveformHeight ? 0 : visibleWaveformHeight);
  }
});
$("ruler-follow").addEventListener("click", () => {
  if (scene && !view.busy) act({ kind: "follow", on: !view.follow }, false);
});
for (const [id, action] of [["ruler-zoom-out", "zoomOut"], ["ruler-zoom-in", "zoomIn"]]) {
  $(id).addEventListener("click", () => {
    if (scene && !view.busy) command(action);
  });
}
$("zoom").addEventListener("input", () => {
  if (requests < 2) changeView(scene.viewStart, 10 ** +$("zoom").value);
});
$("zoom").addEventListener("change", () => changeView(scene.viewStart, 10 ** +$("zoom").value));
for (const button of speedButtons) {
  button.addEventListener("click", () => act({ kind: "speed", value: Number(button.dataset.playbackSpeed) }));
}

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
      paragraph("Every sound it applies to is scaled by the same factor, so their balance stays. A sound that reaches 1% or 100% stops there; a silent sound stays silent.")], () => {
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
function openGenerate() {
  if (!scene || view.busy) return;
  const sliderAudio = input("checkbox", "", 0, 0);
  sliderAudio.checked = false;
  modal("generate", "Generate hitsound difficulty",
    [paragraph("Generate from " + nameOf(scene.path)), labeled("Include slider slide, whistle and tick audio", sliderAudio)],
    [["Cancel", closeModal], ["Review output", () => {
      closeModal();
      act({ kind: "generate", include_slider_audio: sliderAudio.checked });
    }, true]]);
}
function openNewDifficulty() {
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
}
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

// The Export window opens beside the editor. Generate, Copy and Merge settings stay until it replaces them.
$("export-entry").addEventListener("click", () => {
  invoke("open_export_window").catch((error) => { $("status").textContent = String(error); });
});

function openCopy() {
  const paths = scene.difficulties.filter((path) => path !== scene.path);
  if (!paths.length) {
    modal("copy", "Copy hitsounds", [paragraph("There are no other difficulties in this mapset.")], [["Close", closeModal, true]]);
    return;
  }

  const checkboxes = paths.map(() => input("checkbox", "", 0, 0));
  const choices = paths.map((path, i) => labeled(nameOf(path), checkboxes[i]));
  const leniency = input("number", 5, 0, 1000, "any");
  const startIndex = input("number", 100, 2, 2147483647);
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
  const ticks = check(false);
  const slides = check(false);
  const sets = check(true);
  const volumes = check(true);
  const keep5 = check(true);
  const neverSilent = check(false);
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

  const copySliderAudio = () => mode.value === "overwrite_only_defined" && hitsounds.checked && sets.checked;
  const updateEnabled = () => {
    bodies.disabled = !hitsounds.checked;
    ticks.disabled = slides.disabled = !copySliderAudio();
    keep5.disabled = !volumes.checked;
    satisfied.disabled = whenever.disabled = !storyboard.checked;
    for (const field of [divisors, mutedDivisors, minLength, mutedIndex, mutedSet]) {
      field.disabled = !mute.checked;
    }
  };
  for (const field of [mode, hitsounds, sets, volumes, storyboard, mute]) field.addEventListener("change", updateEnabled);
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
    const start = startIndex.valueAsNumber;
    if (!Number.isInteger(start) || start < 2 || start > 2147483647) {
      error.textContent = "The start index must be a whole number from 2 to 2147483647.";
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
      start_index: start, never_silent_hitnormals: neverSilent.checked,
      copy_hitsounds: hitsounds.checked, copy_body_hitsounds: bodies.checked, copy_sample_sets: sets.checked,
      copy_to_slider_ticks: copySliderAudio() && ticks.checked,
      copy_to_slider_slides: copySliderAudio() && slides.checked,
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
  details.append(summary, labeled("Start index", startIndex), labeled("Never create silent hitnormals", neverSilent),
    labeled("Copy storyboard samples", storyboard),
    labeled("Add timed samples at slider ticks", ticks), labeled("Add timed samples during slides", slides),
    paragraph("Timed storyboard samples play independently of slider tracking and preserve the existing slider sounds."),
    paragraph("Enable Keep unmatched hits, Copy hitsounds, and Copy sample sets and indexes to add timed samples."),
    labeled("Skip samples a target hit already plays", satisfied), labeled("Skip samples at any hit", whenever),
    labeled("Mute unmatched slider and spinner ends", mute), labeled("Beat divisors (highest priority first)", divisors),
    labeled("Muted beat divisors", mutedDivisors), labeled("Minimum slider length (beats)", minLength),
    labeled("Muted sample index", mutedIndex), labeled("Muted sample set", mutedSet));
  modal("copy", "Copy hitsounds", [
    paragraph("Copy from " + nameOf(scene.path)),
    ...choices, labeled("Timing tolerance (ms)", leniency), labeled("Unmatched target hits", mode),
    labeled("Copy hitsounds", hitsounds), labeled("Copy slider-body sounds", bodies),
    labeled("Copy sample sets and indexes", sets), labeled("Copy volumes", volumes), labeled("Keep target hits at 5%", keep5),
    details, error,
  ], [["Cancel", closeModal], ["Review output", submit, true]]);
}
$("next-output-problem").addEventListener("click", () => showOutputProblem(outputProblemIndex + 1));
$("warnings").addEventListener("click", () => {
  modal("warnings", "Audio warnings", view.warnings.map(paragraph), [["Close", closeModal, true]]);
});
const playbackStartChoices = [
  ["cursor", "Cursor"], ["left", "Left locator"],
  ["selection", "Selection"], ["selection_or_left", "Selection or left locator"],
];

$("audio").addEventListener("click", () => run(async () => {
  const settings = await invoke("audio_outputs", { backend: null });
  const backend = document.createElement("select");
  const output = document.createElement("select");
  const buffer = input("number", settings.bufferMs ?? "", 10, 500);
  const current = paragraph("");
  const outputError = paragraph("");
  const locked = () => Boolean(view.playing || view.busy);
  let loadedBackend = null;

  function addOption(select, value, label) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    select.append(option);
  }
  function showDevices(config) {
    output.replaceChildren();
    addOption(output, "", config.backend === "asio" ? "Choose an ASIO driver" : "Device default");
    for (const device of config.devices) addOption(output, device.id, device.name);
    output.value = config.device ?? "";
    current.textContent = [config.currentBackend, config.current,
      config.effectiveBufferMs == null ? null : config.effectiveBufferMs + " ms effective buffer"
    ].filter(Boolean).join(" · ");
    const error = config.error ?? "";
    outputError.textContent = error;
    outputError.hidden = !error;
    loadedBackend = error ? null : config.backend;
    output.disabled = locked() || Boolean(error);
    applyOutput.disabled = output.disabled || (config.backend === "asio" && !output.value);
  }

  const applyOutput = makeButton("Apply output", () => {
    if (locked() || applyOutput.disabled || loadedBackend !== backend.value) return;
    const ms = buffer.value === "" ? null : Number(buffer.value);
    if (ms !== null && (!Number.isInteger(ms) || ms < 10 || ms > 500)) {
      buffer.reportValidity();
      return;
    }
    const action = { kind: "output_settings", backend: backend.value, device_id: output.value || null, ms };
    applyOutput.disabled = true;
    run(async () => {
      await update(action, false);
      if (!view.error && modalKind === "audio" && backend.value === action.backend) {
        showDevices(await invoke("audio_outputs", { backend: action.backend }));
      }
    });
  });
  output.addEventListener("change", () => {
    applyOutput.disabled = locked() || loadedBackend !== backend.value || (backend.value === "asio" && !output.value);
  });
  for (const choice of settings.available) addOption(backend, choice.id, choice.label);
  backend.value = settings.backend;
  backend.disabled = locked() || settings.available.length < 2;
  backend.addEventListener("change", () => {
    if (locked()) return;
    const requested = backend.value;
    loadedBackend = null;
    output.disabled = true;
    applyOutput.disabled = true;
    run(async () => {
      const config = await invoke("audio_outputs", { backend: requested });
      if (requested === backend.value && modalKind === "audio") showDevices(config);
    });
  });
  buffer.id = "audio-buffer";
  buffer.placeholder = "Device default";
  buffer.title = "Clear to use the device default buffer";
  buffer.disabled = locked();
  showDevices(settings);

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
    current, labeled("Output backend", backend), labeled("Output device", output),
    labeled("Requested buffer (ms)", buffer), applyOutput, outputError,
    ...(view.playing ? [paragraph("Stop playback to change audio output.")] : []),
    labeled("Song volume", song), labeled("Master volume", main),
    labeled("Mute song", mute), labeled("Song offset (ms)", offset),
    labeled("Start from", start), labeled("Return on stop", returnOnStop),
  ], [["Done", closeModal, true]]);
}));

$("song-mute").addEventListener("click", () => act({ kind: "song_muted", on: !view.songMuted }, false));
$("song-solo").addEventListener("click", () => act({ kind: "song_solo", on: !view.songSolo }, false));

for (const panel of ["browser", "inspector"]) {
  $("view-" + panel).addEventListener("click", () => {
    if (!scene || !view.panels || view.busy) return;
    act({ kind: "panel", panel, on: !view.panels[panel] });
  });
}

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

// A key's name as the editor's key list writes it ("Ctrl+Shift+1", "Numpad *"), following
// keyboard::chord and Key::name in Rust, or null for a key that can't be a shortcut.
function shortcutName(chord) {
  const pad = chord.code.startsWith("Numpad") ? chord.code.slice(6) : "";
  const padKey = {Divide: "/", Multiply: "*", Subtract: "-", Add: "+", Decimal: ".", Enter: "Enter"}[pad] ??
    (/^\d$/.test(pad) ? pad : "");
  const named = (value) => {
    value = {" ": "Space", Escape: "Esc", Delete: "Del", BracketLeft: "[", BracketRight: "]", Slash: "/",
      Period: ".", Comma: ",", Minus: "-", Equal: "=", Backslash: "\\", Semicolon: ";", Quote: "'",
      Backquote: "`"}[value] ?? value.replace(/^Arrow/, "");
    if (/^[a-z]$/i.test(value)) return value.toUpperCase();
    return /^([0-9[\]/.,\-=\\;'`]|Space|Enter|Esc|Backspace|Del|Home|End|PageUp|PageDown|Insert|Tab|Left|Right|Up|Down|F([1-9]|1[0-2]))$/
      .test(value) ? value : "";
  };
  const key = padKey ? "Numpad " + padKey : named(chord.key) || named(chord.code.replace(/^(Key|Digit)/, ""));
  return key ? [chord.ctrl && "Ctrl", chord.shift && "Shift", chord.alt && "Alt", key].filter(Boolean).join("+") : null;
}

async function showShortcutEditor() {
  let state = await invoke("editor_shortcuts");
  // Command ids carry no group, so each row goes under the area F1 help lists it in.
  const groupOf = new Map();
  for (const [title, entries] of await invoke("editor_help")) {
    if (title.endsWith("(mouse)")) continue;
    for (const [, description] of entries) if (!groupOf.has(description)) groupOf.set(description, title);
  }
  // {id} while a row waits for a key; with chord, name and owner while it asks to take another command's key.
  let editing = null;
  let focusId = null;
  const rows = new Map();
  const search = document.createElement("input");
  search.type = "search";
  search.value = "";
  search.placeholder = "Search commands or keys";
  search.setAttribute("aria-label", "Search commands or keys");
  const presets = document.createElement("div");
  presets.className = "shortcut-presets";
  presets.setAttribute("role", "group");
  presets.setAttribute("aria-label", "Key preset");
  for (const [label, name] of [["DAW-like keys", "DAW"], ["osu! editor controls", "osu!"]]) {
    const button = makeButton(label, () => {
      if (state.preset !== name) refresh(() => update({ kind: "key_preset", name }));
    });
    button.dataset.preset = name;
    presets.append(button);
  }
  const toolbar = document.createElement("div");
  toolbar.className = "shortcut-toolbar";
  toolbar.append(presets, search);
  const note = paragraph("Click a command's keys, then press the key to add. It applies at once. Each preset keeps its own keys.");
  note.className = "shortcut-note";
  const table = document.createElement("div");
  table.className = "shortcut-table";
  const summary = document.createElement("p");
  summary.className = "dim";
  summary.setAttribute("role", "status");

  function keyChips(label) {
    const combo = document.createElement("span");
    combo.className = "key-combo";
    const [, ctrl, shift, alt, key] = /^(Ctrl\+)?(Shift\+)?(Alt\+)?(.+)$/.exec(label);
    [ctrl && "Ctrl", shift && "Shift", alt && "Alt", key].filter(Boolean).forEach((part, index) => {
      const chip = document.createElement("kbd");
      chip.textContent = part;
      combo.append(...(index ? ["+", chip] : [chip]));
    });
    return combo;
  }

  // Redraws the row being left and the row being edited; returns the edited row's control.
  function edit(id, next) {
    const previous = editing?.id;
    editing = next;
    if (previous && previous !== id) rows.get(previous)?.();
    return rows.get(id)();
  }

  function press(event, entry, capture) {
    if (["Control", "Meta", "Alt", "Shift"].includes(event.key) || event.repeat) return;
    const chord = keyInput(event);
    if (chord.key === "Tab" && !chord.ctrl && !chord.alt) return;
    event.preventDefault();
    const [id, , keys] = entry;
    // A plain Escape cancels waiting; it can't be assigned from the editor.
    if (chord.key === "Escape" && !chord.ctrl && !chord.shift && !chord.alt) return edit(id, null).focus();
    const name = shortcutName(chord);
    if (!name) {
      capture.textContent = "That key can't be a shortcut. Press another.";
      return;
    }
    if (keys.includes(name)) return edit(id, null).focus();
    const owner = state.entries.find(([, , other]) => other.includes(name));
    if (owner) return edit(id, { id, chord, name, owner }).focus();
    editing = null;
    focusId = id;
    refresh(() => update({ kind: "change_shortcut", id, chord }));
  }

  function fillRow(row, entry) {
    const [id, description, keys] = entry;
    const mine = editing?.id === id ? editing : null;
    row.className = "shortcut-row" + (mine ? (mine.owner ? " conflict" : " capturing") : "");
    const command = document.createElement("span");
    command.className = "shortcut-command";
    command.textContent = description;
    command.title = id;
    let target;
    if (mine?.owner) {
      const conflict = document.createElement("span");
      conflict.className = "shortcut-conflict";
      conflict.textContent = "Also " + mine.owner[1].replace(/^[A-Z](?=[a-z])/, (c) => c.toLowerCase()) + " · " + mine.name;
      target = makeButton("Move here", () => {
        editing = null;
        focusId = id;
        refresh(() => update({ kind: "change_shortcut", id, chord: mine.chord }));
      });
      target.setAttribute("aria-label", `Move ${mine.name} to ${description}`);
      row.replaceChildren(command, conflict, keyChips(mine.name), target, makeButton("Cancel", () => edit(id, null).focus()));
    } else if (mine) {
      target = document.createElement("span");
      target.className = "shortcut-capture";
      target.tabIndex = 0;
      target.setAttribute("role", "textbox");
      target.setAttribute("aria-label", "Press the new key for " + description);
      target.textContent = "Press keys… · Esc cancels";
      target.addEventListener("keydown", (event) => press(event, entry, target));
      target.addEventListener("blur", () => { if (editing === mine) edit(id, null); });
      row.replaceChildren(command, target);
    } else {
      target = document.createElement("button");
      target.className = "shortcut-keys";
      target.title = "Add a key";
      target.setAttribute("aria-label", `${description}: ${keys.join(", ") || "no keys"}. Add a key`);
      if (keys.length) target.append(...keys.map(keyChips));
      else target.textContent = "Unassigned";
      target.addEventListener("click", () => edit(id, { id }).focus());
      const clear = makeButton("Clear", () => {
        focusId = id;
        refresh(() => update({ kind: "change_shortcut", id, chord: null }));
      });
      clear.className = "shortcut-clear";
      clear.disabled = !keys.length;
      clear.setAttribute("aria-label", "Clear the keys of " + description);
      row.replaceChildren(command, target, clear);
    }
    return target;
  }

  function render() {
    for (const button of presets.children) button.setAttribute("aria-pressed", String(button.dataset.preset === state.preset));
    const terms = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const groups = new Map([...new Set(groupOf.values())].map((title) => [title, []]));
    rows.clear();
    for (const entry of state.entries) {
      const [id, description, keys] = entry;
      const text = [id, description, ...keys].join(" ").toLowerCase();
      if (!terms.every((term) => text.includes(term))) continue;
      const row = document.createElement("div");
      row.dataset.command = id;
      rows.set(id, () => fillRow(row, entry));
      rows.get(id)();
      const title = groupOf.get(description) ?? "Other";
      if (!groups.has(title)) groups.set(title, []);
      groups.get(title).push(row);
    }
    const sections = [...groups].filter(([, list]) => list.length).map(([title, list]) => {
      const section = document.createElement("section");
      section.className = "shortcut-group";
      const heading = document.createElement("h3");
      heading.textContent = title;
      section.append(heading, ...list);
      return section;
    });
    const body = $("dialog-body");
    const top = body.scrollTop;
    table.replaceChildren(...(sections.length ? sections : [paragraph("No commands match.")]));
    body.scrollTop = top;
    if (focusId && rows.has(focusId)) rows.get(focusId)().focus();
    focusId = null;
  }

  function refresh(task) {
    return run(async () => {
      try {
        await task();
        if (modalKind !== "shortcut-editor" || view.error) return;
        state = await invoke("editor_shortcuts");
        summary.textContent = view.status ?? "";
        editing = null;
        render();
      } catch (error) {
        summary.textContent = String(error);
      }
    });
  }

  search.addEventListener("input", render);
  render();
  modal("shortcut-editor", "Customise shortcuts", [toolbar, note, table, summary], [
    ["Import…", () => refresh(async () => { if (await invoke("import_key_file")) await update(null, false); })],
    ["Export…", () => refresh(async () => { if (await invoke("export_key_file")) await update(null, false); })],
    ["Close", closeModal, true],
  ]);
  $("dialog-body").scrollTop = 0;
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
  if (rulerDrag) {
    if (key === "Escape" && !ctrl && !shift && !alt) {
      event.preventDefault();
      endRulerDrag(event);
    }
    return;
  }
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
  const activatesButton = !ctrl && !shift && !alt &&
    ((key === " " && !scene) || (key === "Enter" && code !== "NumpadEnter"));
  if (view.busy || (activatesButton && event.target.closest("button"))) return;
  if (key === "Tab") {
    if (!ctrl && !alt && event.target.closest("[data-dock-control]")) return;
    const bound = view.tabRoutes?.some(([control, shifted, alternate, global]) =>
      control === ctrl && shifted === shift && alternate === alt && (scene || global));
    if (!bound) return;
  } else if (key.length !== 1 &&
      !/^(Arrow(Left|Right|Up|Down)|Delete|Backspace|Escape|Home|End|PageUp|PageDown|Insert|Enter|F([1-9]|1[0-2]))$/.test(key) &&
      !code.startsWith("Numpad")) return;

  event.preventDefault();
  const action = { kind: "key", key, code, ctrl, shift, alt };
  if (scene?.path && grid.clientWidth > 0 && scene.zoom > 0) {
    action.lane_view = {
      path: scene.path,
      rows: [...new Set(laneRows.filter(Number.isInteger))],
      span: grid.clientWidth / scene.zoom,
    };
  }
  act(action);
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
  if (view.fault) return;
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
  if (view.fault || requests || refreshing || now - lastRefresh < (view.playing || view.busy ? 85 : 1000)) return;
  refreshing = true;
  lastRefresh = now;
  await run(() => update(null, false));
  refreshing = false;
  // Save and quit waits while the Save hitsounds summary is open; its Save then quits.
  if (quitAfterSave && !view.busy && !view.saveProblemReview) {
    quitAfterSave = false;
    if (!view.dirty && !view.error) run(() => invoke("close_editor", { discard: false }));
  }
}, 85);
act(null);
