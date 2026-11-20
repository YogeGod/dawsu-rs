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
  // Playhead group's Centre: kept in this window only, never saved; app.js reads it to follow.
  centre = $state(false);
  // The snap group the slider and the wheel step through (lazer's Common, Triplets or a custom one).
  // Worked out from each snap the editor shows, never saved.
  snapGroup = $state.raw(null);

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

  set(view, scene) {
    this.snapGroup = followGroup(this.snapGroup, this.scene, scene);
    this.view = view;
    this.scene = scene;
  }
}

export const bridge = new Bridge();

// app.js declares act() at the top level of a classic script, so it is a global.
export const act = action => globalThis.act(action);
