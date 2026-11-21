import { flushSync } from "svelte";
import { followGroup } from "./snap.js";
import { samePlayhead, sameUiView } from "./view.js";

// The editor state app.js last showed. show() sends view and scene on every update (every 85 ms
// during playback); playhead isolates ~85 ms playback/time updates from view so unrelated parts
// keep their reference and do not re-render.
export class Bridge {
  view = $state.raw(null);
  playhead = $state.raw(null);
  latestView = null;
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
  // Transient status notice (errors, fault, or missing UI part) pushed by app.js.
  statusNotice = $state("");
  // Optimistic timeline zoom & lane height during changeView() drags before Rust replies.
  zoom = $state(null);
  rowHeight = $state(null);
  // Lane popover filter state owned by app.js's renderLanes() / map-load reset.
  usedOnly = $state(false);
  laneFilter = $state("");
  // Active viewer overlay pushed from the dedicated Node Graph window.
  nodeGraphViewer = $state.raw(null);
  // The map preview is open in its own window (Rust's preview-window event).
  previewWindow = $state(false);

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

  setViewport(zoom, rowHeight) {
    this.zoom = zoom;
    this.rowHeight = rowHeight;
  }

  setLaneOptions(usedOnly, laneFilter) {
    this.usedOnly = !!usedOnly;
    this.laneFilter = laneFilter ?? "";
  }

  set(view, scene) {
    this.latestView = view;
    if (scene !== this.scene) {
      this.snapGroup = followGroup(this.snapGroup, this.scene, scene);
      this.scene = scene;
      this.zoom = scene ? scene.zoom : null;
      this.rowHeight = scene ? scene.rowHeight : null;
    }
    if (!samePlayhead(this.playhead, view)) this.playhead = view;
    if (!sameUiView(this.view, view)) this.view = view;
  }
}

export const bridge = new Bridge();

// app.js declares act() at the top level of a classic script, so it is a global.
export const act = action => globalThis.act(action);
