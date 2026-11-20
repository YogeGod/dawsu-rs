import "./tokens.css";
import "./parts/pill.css";
import { mount } from "svelte";
import { bridge } from "./lib/bridge.svelte.js";
import Boundary from "./lib/Boundary.svelte";
import AuditionPill from "./parts/AuditionPill.svelte";
import LaneHeaders from "./parts/LaneHeaders.svelte";
import PlayheadGroup from "./parts/PlayheadGroup.svelte";
import SnapSlider from "./parts/SnapSlider.svelte";
import ToolPills from "./parts/ToolPills.svelte";
import TransportBar from "./parts/TransportBar.svelte";

// Each part replaces the children of its [data-ui-mount="name"] host in index.html.
const parts = {
  tools: ToolPills, snap: SnapSlider, audition: AuditionPill, transport: TransportBar,
  playhead: PlayheadGroup, lanes: LaneHeaders,
};

// app.js may have built the lane rows before this bundle loaded.
bridge.lanes = globalThis.currentLaneHeaders?.() ?? null;

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
  bridge.lanes = headers;
}

// The Playhead group's Centre, read by app.js's followPlayhead().
export const centre = () => bridge.centre;
