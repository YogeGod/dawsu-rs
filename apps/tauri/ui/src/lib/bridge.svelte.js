// The editor state app.js last showed. show() replaces both whole on every update (every 85 ms
// during playback), so parts derive what they need and keep no copy of their own.
export class Bridge {
  view = $state.raw(null);
  scene = $state.raw(null);

  set(view, scene) {
    this.view = view;
    this.scene = scene;
  }
}

export const bridge = new Bridge();

// app.js declares act() at the top level of a classic script, so it is a global.
export const act = action => globalThis.act(action);
