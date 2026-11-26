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
import { bridge } from "./lib/bridge.svelte.js";
import * as commandList from "./lib/commands.js";
import { hintZoneFromTarget } from "./lib/hints.js";
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
import MapsetIntakeCard from "./parts/MapsetIntakeCard.svelte";
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
  // Unified 3-path Mapset Intake Card modal, at body level (works from Start Screen or while editing).
  "mapset-intake": MapsetIntakeCard,
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

// The one command list menus.js builds every menu from.
export const commands = commandList;
export { bridge };

// Boot the main editor runtime only on the editor window (never on export.html or drum.html).
if (document.getElementById("grid")) {
  bridge.registerAction("showMicroHud", (x, y, extra = {}) => bridge.showMicroHud(x, y, extra));
  const session = createEditor(globalThis, { ui: bridge.editorApi, exposeGlobals: false });
  initMenus(globalThis, { ui: bridge.editorApi, session });
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
