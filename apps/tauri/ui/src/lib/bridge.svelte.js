import { flushSync } from "svelte";
import { followGroup } from "./snap.js";

// The editor state app.js last showed. show() replaces both whole on every update (every 85 ms
// during playback), so parts derive what they need and keep no copy of their own.
export class Bridge {
  view = $state.raw(null);
  scene = $state.raw(null);
  // The lane and folder header rows app.js last built (renderLanes), replaced whole.
  lanes = $state.raw(null);
  // The Inspector's Selection, Lane and Difficulty model app.js last built (inspectorModel), replaced whole.
  inspector = $state.raw(null);
  // The Browser's tabs, tree and sample preview app.js last built (renderSampleBrowser), replaced whole.
  browser = $state.raw(null);
  // The open menu's rows menus.js last built from the command list (null while no menu is open).
  menu = $state.raw(null);
  // The hovered timeline hit app.js last handed over (hitCardAt), or null.
  hitCard = $state.raw(null);
  // Playhead group's Centre: kept in this window only, never saved; app.js reads it to follow.
  centre = $state(false);
  // The snap group the slider and the wheel step through (lazer's Common, Triplets or a custom one).
  // Worked out from each snap the editor shows, never saved.
  snapGroup = $state.raw(null);
  // The Mixer's lane strips app.js last built (mixerModel), replaced whole.
  mixer = $state.raw(null);
  // The bottom panel's tab ("volume" or "mixer"): kept in this window only, never saved; app.js
  // reads it to lay the panel out and to draw the volume canvas.
  bottomTab = $state("volume");
  // View › Map preview: kept in this window only, never saved.
  mapPreview = $state(false);
  // View › Node graph: kept in this window only, never saved.
  nodeGraphOpen = $state(false);
  nodeGraphViewer = $state.raw(null);

  // No map, a job running or a fault: the parts show but take no input.
  get disabled() {
    return !this.scene || !!this.view?.busy || !!this.view?.fault;
  }

  // app.js may scroll to a row right after renderLanes(), so the rows are written before this returns.
  handOverLanes(headers, flush = flushSync) {
    this.lanes = headers;
    flush();
  }

  // Handed over by app.js's renderSelection() and renderActiveLane(); written at once, like the lanes.
  handOverInspector(model, flush = flushSync) {
    this.inspector = model;
    flush();
  }

  // The rail icons focus a tab or the search field right after switching, so the model is written at once.
  handOverBrowser(model, flush = flushSync) {
    this.browser = model;
    flush();
  }

  handOverMixer(strips) {
    this.mixer = strips;
  }

  // menus.js measures the popup and focuses its first row right after handing the rows over.
  handOverMenu(model, flush = flushSync) {
    this.menu = model;
    flush();
  }

  set(view, scene) {
    this.snapGroup = followGroup(this.snapGroup, this.scene, scene);
    this.view = view;
    this.scene = scene;
  }
}

export const bridge = new Bridge();

// app.js declares act() at the top level of a classic script, so it is a global.
export const act = action => globalThis.act(action);
