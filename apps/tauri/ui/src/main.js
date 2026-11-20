import "./tokens.css";
import "./parts/pill.css";
import "./parts/inspector.css";
import { mount } from "svelte";
import { bridge } from "./lib/bridge.svelte.js";
import Boundary from "./lib/Boundary.svelte";
import AuditionPill from "./parts/AuditionPill.svelte";
import LaneHeaders from "./parts/LaneHeaders.svelte";
import PlayheadGroup from "./parts/PlayheadGroup.svelte";
import ExportWindow from "./parts/ExportWindow.svelte";
import InspectorDifficulty from "./parts/InspectorDifficulty.svelte";
import InspectorLane from "./parts/InspectorLane.svelte";
import InspectorSelection from "./parts/InspectorSelection.svelte";
import SnapSlider from "./parts/SnapSlider.svelte";
import ToolPills from "./parts/ToolPills.svelte";
import TransportBar from "./parts/TransportBar.svelte";

// Each part replaces the children of its [data-ui-mount="name"] host in index.html.
const parts = {
  tools: ToolPills, snap: SnapSlider, audition: AuditionPill, transport: TransportBar,
  playhead: PlayheadGroup, lanes: LaneHeaders,
  "inspector-selection": InspectorSelection, "inspector-lane": InspectorLane, "inspector-difficulty": InspectorDifficulty,
  // The whole page of the Export window (export.html).
  "export-window": ExportWindow,
};

// app.js may have built the lane rows before this bundle loaded.
bridge.lanes = globalThis.currentLaneHeaders?.() ?? null;
bridge.inspector = globalThis.currentInspector?.() ?? null;

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
}

// app.js hands over the lane and folder header rows each time renderLanes() changes them.
export function lanes(headers) {
  bridge.handOverLanes(headers);
}

// app.js hands over the Inspector's model each time renderSelection() or renderActiveLane() runs.
export function inspector(model) {
  bridge.handOverInspector(model);
}

// The Playhead group's Centre, read by app.js's followPlayhead().
export const centre = () => bridge.centre;
