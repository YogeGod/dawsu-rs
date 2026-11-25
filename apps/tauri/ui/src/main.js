import "./tokens.css";
import "./legacy-chrome.css";
import "./shell.css";
import "./parts/pill.css";
import "./parts/inspector.css";
import "./parts/browser.css";
import "./parts/bottom-panel.css";
import "./parts/node-graph.css";
import "./timeline.css";
import { mount } from "svelte";
import { createEditor } from "./editor/boot.js";
import { initMenus } from "./editor/menus.js";
import { invoke } from "./lib/ipc.js";
import { bridge } from "./lib/bridge.svelte.js";
import * as commandList from "./lib/commands.js";
import { hintZoneFromTarget } from "./lib/hints.js";
import { nextGroup, stepInGroup } from "./lib/snap.js";
import { applyColorConfig, readColorConfig } from "./lib/theme.js";
import Boundary from "./lib/Boundary.svelte";
import AuditionPill from "./parts/AuditionPill.svelte";
import BaseDiffStrip from "./parts/BaseDiffStrip.svelte";
import BrowserHead from "./parts/BrowserHead.svelte";
import BrowserList from "./parts/BrowserList.svelte";
import BrowserPreview from "./parts/BrowserPreview.svelte";
import BottomPanel from "./parts/BottomPanel.svelte";
import CommandPalette from "./parts/CommandPalette.svelte";
import LaneHeaders from "./parts/LaneHeaders.svelte";
import Mixer from "./parts/Mixer.svelte";
import PlayheadGroup from "./parts/PlayheadGroup.svelte";
import ExportWindow from "./parts/ExportWindow.svelte";
import ExportModal from "./parts/ExportModal.svelte";
import DrumDetectWindow from "./parts/DrumDetectWindow.svelte";
import DrumStudioHost from "./parts/DrumStudioHost.svelte";
import EditorMenu from "./parts/EditorMenu.svelte";
import GridOptions from "./parts/GridOptions.svelte";
import HitCard from "./parts/HitCard.svelte";
import InspectorBookmarks from "./parts/InspectorBookmarks.svelte";
import InspectorDifficulty from "./parts/InspectorDifficulty.svelte";
import InspectorHistory from "./parts/InspectorHistory.svelte";
import InspectorLane from "./parts/InspectorLane.svelte";
import InspectorSelection from "./parts/InspectorSelection.svelte";
import LaneOptions from "./parts/LaneOptions.svelte";
import LoadingOverlay from "./parts/LoadingOverlay.svelte";
import KeysoundsModal from "./parts/KeysoundsModal.svelte";
import MapPreview from "./parts/MapPreview.svelte";
import MicroHud from "./parts/MicroHud.svelte";
import MidiImportModal from "./parts/MidiImportModal.svelte";
import NodeGraphWindow from "./parts/NodeGraphWindow.svelte";
import OverviewDiagnostics from "./parts/OverviewDiagnostics.svelte";
import PreferencesModal from "./parts/PreferencesModal.svelte";
import PreviewWindow from "./parts/PreviewWindow.svelte";
import QuickStartTour from "./parts/QuickStartTour.svelte";
import SharedBaseModal from "./parts/SharedBaseModal.svelte";
import StemSeparationModal from "./parts/StemSeparationModal.svelte";
import SnapSlider from "./parts/SnapSlider.svelte";
import StartScreen from "./parts/StartScreen.svelte";
import StatusBar from "./parts/StatusBar.svelte";
import ToastStack from "./parts/ToastStack.svelte";
import ToolPills from "./parts/ToolPills.svelte";
import TransportBar from "./parts/TransportBar.svelte";
import VolumeBubble from "./parts/VolumeBubble.svelte";

// Each part replaces the children of its [data-ui-mount="name"] host in index.html.
const parts = {
  tools: ToolPills, snap: SnapSlider, audition: AuditionPill, transport: TransportBar,
  playhead: PlayheadGroup, lanes: LaneHeaders, "status-bar": StatusBar,
  "grid-options": GridOptions, "lane-options": LaneOptions, "start-screen": StartScreen,
  // The timeline's hit hover card, volume typing bubble, preferences modal, and toast stack, at body level.
  "base-diff-strip": BaseDiffStrip,
  "hit-card": HitCard,
  "volume-bubble": VolumeBubble,
  "micro-hud": MicroHud,
  preferences: PreferencesModal,
  // File › Import MIDI… and Map › Keysounds & VST3…, at body level.
  "midi-import": MidiImportModal,
  keysounds: KeysoundsModal,
  "drum-studio": DrumStudioHost,
  "shared-base": SharedBaseModal,
  "stem-separation": StemSeparationModal,
  toasts: ToastStack,
  // The Quick start tour card (View › Quick start tour), at body level.
  "quick-start": QuickStartTour,
  // Centred loading overlay shown if a map/audio load takes >300 ms.
  "loading-overlay": LoadingOverlay,
  // Global Ctrl+K Command Palette modal, at body level.
  "command-palette": CommandPalette,
  // The popup the menu bar, the right-click tool picker and the lane menu share (#editor-menu).
  menu: EditorMenu,
  "inspector-selection": InspectorSelection, "inspector-lane": InspectorLane, "inspector-difficulty": InspectorDifficulty,
  "inspector-bookmarks": InspectorBookmarks, "inspector-history": InspectorHistory,
  "overview-diagnostics": OverviewDiagnostics,
  // The map preview, at the top of the Inspector while View › Map preview is on.
  "map-preview": MapPreview,
  // The whole page of the node graph window (node-graph.html).
  "node-graph-window": NodeGraphWindow,
  // The Browser (#sample-browser): tabs and search, the sample tree, the preview with Use and Browse.
  "browser-head": BrowserHead, "browser-list": BrowserList, "browser-preview": BrowserPreview,
  // The bottom panel: its side column (tabs or the folded strip) and the Mixer tab.
  "bottom-panel": BottomPanel, mixer: Mixer,
  // The whole page of the Export window (export.html).
  "export-window": ExportWindow,
  "export-modal": ExportModal,
  // The whole page of the drum detection window (drum.html).
  "drum-window": DrumDetectWindow,
  // The whole page of the map preview's own window (preview.html).
  "preview-window": PreviewWindow,
};

export function update(view, scene) {
  bridge.set(view, scene);
  if (!previewLink) linkPreviewWindow(view);
  if (bridge.previewWindow) sendPreview();
}

// The map preview's own window: only the editor window calls update(), so it alone listens. It
// sends the playhead and the open difficulty on each update, when the window opens and when its
// page is ready, and opens it again at launch if it was open at the last close (view.previewWindow).
let previewLink = false;
function linkPreviewWindow(view) {
  previewLink = true;
  const events = globalThis.window?.__TAURI__?.event;
  if (!events) return;
  events.listen("preview-window", ({ payload }) => {
    bridge.previewWindow = payload === true;
    // With the window open the Inspector panel is on too, so its "Show it here" can be reached
    // (View › Map preview is not remembered, but the window is).
    if (bridge.previewWindow) bridge.mapPreview = true;
    if (bridge.previewWindow) sendPreview(true);
  });
  // The page being ready also says the window is open, should its open event have come too soon.
  events.listen("preview-ready", () => {
    bridge.previewWindow = true;
    bridge.mapPreview = true;
    sendPreview(true);
  });
}
let lastSent = "";
function sendPreview(force = false) {
  const view = bridge.latestView ?? bridge.view;
  const scene = bridge.scene;
  const payload = {
    view: view && { position: view.position, playing: view.playing, speed: view.speed, duration: view.duration },
    path: scene?.path ?? null,
    previewKey: scene?.previewKey ?? null,
  };
  // Only what changed goes: paused and unmoved, the 1 s updates send nothing.
  const sent = JSON.stringify(payload);
  if (sent === lastSent && !force) return;
  lastSent = sent;
  globalThis.window?.__TAURI__?.event.emitTo("preview", "preview-view", payload).catch(() => {});
}

// The editor runtime hands over the lane and folder header rows each time renderLanes() changes them.
export function lanes(headers) {
  bridge.handOverLanes(headers);
}

// The editor runtime hands over the Inspector's model each time renderSelection() or renderActiveLane() runs,
// and when a bookmark row is picked.
export function inspector(model) {
  bridge.handOverInspector(model);
}

// The editor runtime hands over the Browser's model each time renderSampleBrowser() changes it.
export function browser(model) {
  bridge.handOverBrowser(model);
}

// The editor runtime hands over the Mixer's lane strips each time renderLanes() changes them.
export function mixer(strips) {
  bridge.handOverMixer(strips);
}

// View › Map preview: menus.js flips it and reads it for the row's switch.
export function toggleMapPreview() {
  bridge.mapPreview = !bridge.mapPreview;
}
export const mapPreviewOn = () => bridge.mapPreview;

// View › Quick start tour: opens the tour at step 1, done or not.
export function openQuickStart() {
  bridge.quickStart = 1;
}

// Synced timeline preview pushed from the dedicated Node Graph window.
export function setNodeGraphViewer(viewer) {
  bridge.nodeGraphViewer = viewer ?? null;
}
export const nodeGraphViewer = () => bridge.nodeGraphViewer;

// The bottom panel's tab, read by renderPanelLayout() and paint().
export const bottomTab = () => bridge.bottomTab;

// The editor runtime hands over the hovered timeline hit, or null, each time it changes.
export function hitCard(card) {
  bridge.hitCard = card;
}

// The one command list menus.js builds every menu from.
export const commands = commandList;

// menus.js hands over the open menu's rows (null when it closes).
export function menu(model) {
  bridge.handOverMenu(model);
}

// The Playhead group's Centre, read by followPlayhead().
export const centre = () => bridge.centre;

// Ctrl+wheel (Alt+wheel in the DAW preset): the next 1/x in the snap group, or null at its end.
export function wheelSnap(finer) {
  const scene = bridge.scene;
  if (!scene) return null;
  return stepInGroup(nextGroup(bridge.snapGroup, scene.beats, scene.divisor), scene.beats, scene.divisor, finer);
}

// The Triplets box in the Snap menu shows the group.
export const snapGroup = () => bridge.snapGroup?.name;

export function statusNotice(text) {
  bridge.statusNotice = text || "";
}

export function viewport(zoom, rowHeight) {
  bridge.setViewport(zoom, rowHeight);
}

export function laneOptions(usedOnly, filter, groupBySampleSet = true) {
  bridge.setLaneOptions(usedOnly, filter, groupBySampleSet);
}

// Active coverage review filter and focus time shared with boot.js for N / Shift+N stepping.
export const coverageFilter = () => bridge.coverageFilter;
export function setCoverageFocus(time) {
  bridge.coverageFocusTime = Number.isFinite(time) ? time : null;
}

export function hint(zone) {
  bridge.setHint(zone);
}

export function setMode(mode) {
  bridge.mode = mode === "drums" || mode === "keysounds" ? mode : "hitsounds";
}

export function setMicroHud(hud) {
  bridge.microHud = hud ?? null;
}

export function volumeBubble(next) {
  if (bridge.volumeBubble?.editing && next && !next.editing) return;
  bridge.volumeBubble = next ?? null;
}

// Boot the main editor runtime only on the editor window (never on export.html or drum.html).
const uiBridge = {
  update, lanes, inspector, browser, mixer, toggleMapPreview, mapPreviewOn, setNodeGraphViewer, nodeGraphViewer, bottomTab, hitCard, volumeBubble, commands, menu, centre, wheelSnap, snapGroup, statusNotice, viewport, laneOptions, coverageFilter, setCoverageFocus, hint, setMode, setMicroHud,
};
if (document.getElementById("grid")) {
  const showMicroHud = (x, y, extra = {}) => {
    bridge.microHud = { x, y, mode: bridge.mode, ...extra };
  };
  bridge.registerAction("showMicroHud", showMicroHud);
  globalThis.showMicroHud = showMicroHud;
  createEditor(globalThis, { ui: uiBridge });
  initMenus(globalThis, { ui: uiBridge });
  const workspace = document.getElementById("workspace");
  workspace?.addEventListener("pointerover", event => {
    bridge.setHint(hintZoneFromTarget(event.target));
  });
  workspace?.addEventListener("pointerleave", () => bridge.setHint(null));
  globalThis.window?.addEventListener("blur", () => bridge.setHint(null));
}
applyColorConfig(readColorConfig());

bridge.lanes = bridge.callAction("currentLaneHeaders") ?? null;
bridge.inspector = bridge.callAction("currentInspector") ?? null;
bridge.browser = bridge.callAction("currentBrowser") ?? null;
bridge.mixer = bridge.callAction("currentMixer") ?? null;

for (const host of document.querySelectorAll("[data-ui-mount]")) {
  const part = parts[host.dataset.uiMount];
  if (!part) continue;
  host.replaceChildren();
  host.dataset.uiReady = "";
  try {
    mount(Boundary, { target: host, props: { part, host } });
  } catch (error) {
    delete host.dataset.uiReady;
    console.error(error);
  }
}
