import "./tokens.css";
import "./shell.css";
import "./parts/pill.css";
import "./parts/inspector.css";
import "./parts/browser.css";
import "./parts/bottom-panel.css";
import "./timeline.css";
import { mount } from "svelte";
import { invoke } from "./lib/ipc.js";
import { bridge } from "./lib/bridge.svelte.js";
import * as commandList from "./lib/commands.js";
import { nextGroup, stepInGroup } from "./lib/snap.js";
import Boundary from "./lib/Boundary.svelte";
import AuditionPill from "./parts/AuditionPill.svelte";
import BrowserHead from "./parts/BrowserHead.svelte";
import BrowserList from "./parts/BrowserList.svelte";
import BrowserPreview from "./parts/BrowserPreview.svelte";
import BottomPanel from "./parts/BottomPanel.svelte";
import LaneHeaders from "./parts/LaneHeaders.svelte";
import Mixer from "./parts/Mixer.svelte";
import PlayheadGroup from "./parts/PlayheadGroup.svelte";
import ExportWindow from "./parts/ExportWindow.svelte";
import DrumDetectWindow from "./parts/DrumDetectWindow.svelte";
import EditorMenu from "./parts/EditorMenu.svelte";
import HitCard from "./parts/HitCard.svelte";
import InspectorBookmarks from "./parts/InspectorBookmarks.svelte";
import InspectorDifficulty from "./parts/InspectorDifficulty.svelte";
import InspectorHistory from "./parts/InspectorHistory.svelte";
import InspectorLane from "./parts/InspectorLane.svelte";
import InspectorSelection from "./parts/InspectorSelection.svelte";
import MapPreview from "./parts/MapPreview.svelte";
import PreviewWindow from "./parts/PreviewWindow.svelte";
import SnapSlider from "./parts/SnapSlider.svelte";
import ToolPills from "./parts/ToolPills.svelte";
import TransportBar from "./parts/TransportBar.svelte";

// Each part replaces the children of its [data-ui-mount="name"] host in index.html.
const parts = {
  tools: ToolPills, snap: SnapSlider, audition: AuditionPill, transport: TransportBar,
  playhead: PlayheadGroup, lanes: LaneHeaders,
  // The timeline's hit hover card, at body level.
  "hit-card": HitCard,
  // The popup the menu bar, the right-click tool picker and the lane menu share (#editor-menu).
  menu: EditorMenu,
  "inspector-selection": InspectorSelection, "inspector-lane": InspectorLane, "inspector-difficulty": InspectorDifficulty,
  "inspector-bookmarks": InspectorBookmarks, "inspector-history": InspectorHistory,
  // The map preview, at the top of the Inspector while View › Map preview is on.
  "map-preview": MapPreview,
  // The Browser (#sample-browser): tabs and search, the sample tree, the preview with Use and Browse.
  "browser-head": BrowserHead, "browser-list": BrowserList, "browser-preview": BrowserPreview,
  // The bottom panel: its side column (tabs or the folded strip) and the Mixer tab.
  "bottom-panel": BottomPanel, mixer: Mixer,
  // The whole page of the Export window (export.html).
  "export-window": ExportWindow,
  // The whole page of the drum detection window (drum.html).
  "drum-window": DrumDetectWindow,
  // The whole page of the map preview's own window (preview.html).
  "preview-window": PreviewWindow,
};

// app.js may have built the lane rows before this bundle loaded.
bridge.lanes = globalThis.currentLaneHeaders?.() ?? null;
bridge.inspector = globalThis.currentInspector?.() ?? null;
bridge.browser = globalThis.currentBrowser?.() ?? null;
bridge.mixer = globalThis.currentMixer?.() ?? null;

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

export function update(view, scene) {
  bridge.set(view, scene);
  if (!previewLink) linkPreviewWindow(view);
  if (bridge.previewWindow) sendPreview();
}

// The map preview's own window: only app.js, in the editor window, calls update(), so the editor
// window alone listens. It sends the playhead and the open difficulty on each update, when the
// window opens and when its page is ready, and opens it again at launch if it was open at the
// last close (view.previewWindow).
let previewLink = false;
function linkPreviewWindow(view) {
  previewLink = true;
  const events = globalThis.window?.__TAURI__?.event;
  if (!events) return;
  events.listen("preview-window", ({ payload }) => {
    bridge.previewWindow = payload === true;
    if (bridge.previewWindow) sendPreview(true);
  });
  // The page being ready also says the window is open, should its open event have come too soon.
  events.listen("preview-ready", () => {
    bridge.previewWindow = true;
    sendPreview(true);
  });
  if (view?.previewWindow) invoke("preview_window_open").catch(error => console.error(error));
}
let lastSent = "";
function sendPreview(force = false) {
  const view = bridge.view;
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

// app.js hands over the lane and folder header rows each time renderLanes() changes them.
export function lanes(headers) {
  bridge.handOverLanes(headers);
}

// app.js hands over the Inspector's model each time renderSelection() or renderActiveLane() runs,
// and when a bookmark row is picked.
export function inspector(model) {
  bridge.handOverInspector(model);
}

// app.js hands over the Browser's model each time renderSampleBrowser() changes it.
export function browser(model) {
  bridge.handOverBrowser(model);
}

// app.js hands over the Mixer's lane strips each time renderLanes() changes them.
export function mixer(strips) {
  bridge.handOverMixer(strips);
}

// View › Map preview: menus.js flips it and reads it for the row's switch.
export function toggleMapPreview() {
  bridge.mapPreview = !bridge.mapPreview;
}
export const mapPreviewOn = () => bridge.mapPreview;

// The bottom panel's tab, read by app.js's renderPanelLayout() and paint().
export const bottomTab = () => bridge.bottomTab;

// app.js hands over the hovered timeline hit, or null, each time it changes.
export function hitCard(card) {
  bridge.hitCard = card;
}

// The one command list menus.js builds every menu from.
export const commands = commandList;

// menus.js hands over the open menu's rows (null when it closes).
export function menu(model) {
  bridge.handOverMenu(model);
}

// The Playhead group's Centre, read by app.js's followPlayhead().
export const centre = () => bridge.centre;

// Ctrl+wheel (Alt+wheel in the DAW preset) in app.js: the next 1/x in the snap group, or null at its end.
export function wheelSnap(finer) {
  const scene = bridge.scene;
  if (!scene) return null;
  return stepInGroup(nextGroup(bridge.snapGroup, scene.beats, scene.divisor), scene.beats, scene.divisor, finer);
}

// The Triplets box in the Snap menu shows the group.
export const snapGroup = () => bridge.snapGroup?.name;
