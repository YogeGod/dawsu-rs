import "./tokens.css";
import "./parts/pill.css";
import { mount } from "svelte";
import { bridge } from "./lib/bridge.svelte.js";
import Boundary from "./lib/Boundary.svelte";
import AuditionPill from "./parts/AuditionPill.svelte";
import SnapSlider from "./parts/SnapSlider.svelte";
import ToolPills from "./parts/ToolPills.svelte";
import TransportBar from "./parts/TransportBar.svelte";

// Each part replaces the children of its [data-ui-mount="name"] host in index.html.
const parts = { tools: ToolPills, snap: SnapSlider, audition: AuditionPill, transport: TransportBar };

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
