import {
  actions, modals, registerAction, callAction, registerModal, openModal,
  setModalOpen, hasOpenModal, setShortcutCapturing, isShortcutCapturing,
  registerToast, showToast, registerPaletteMenu, getPaletteMenu,
} from "./registry.js";
import { flushSync } from "svelte";
import * as commandList from "./commands.js";
import { followGroup, nextGroup, stepInGroup } from "./snap.js";
import { samePlayhead, sameUiView } from "./view.js";

// The editor state app.js last showed. show() sends view and scene on every update (every 85 ms
// during playback); playhead isolates ~85 ms playback/time updates from view so unrelated parts
// keep their reference and do not re-render.
export class Bridge {
  _previewLink = false;
  _lastSent = "";

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
  // The floating volume bubble over the Hit volume lane (hover readout or inline typing), or null.
  volumeBubble = $state.raw(null);
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
  // View › Map preview: on when the editor opens, kept in this window only, never saved.
  mapPreview = $state(true);
  // Transient status notice (errors, fault, or missing UI part) pushed by app.js.
  statusNotice = $state("");
  // Active hover hint zone ("timeline", "ruler", "volume", "lanes", "mixer", "browser", "inspector", "overview"), or null.
  hint = $state(null);
  // Optimistic timeline zoom & lane height during changeView() drags before Rust replies.
  zoom = $state(null);
  rowHeight = $state(null);
  // Lane popover filter state owned by app.js's renderLanes() / map-load reset.
  usedOnly = $state(false);
  laneFilter = $state("");
  groupBySampleSet = $state(true);
  // Active viewer overlay pushed from the dedicated Node Graph window.
  nodeGraphViewer = $state.raw(null);
  // The map preview is open in its own window (Rust's preview-window event).
  previewWindow = $state(false);
  // The Quick start tour's step (1-5), or 0 while it is closed. Kept in this window only.
  quickStart = $state(0);
  // Active coverage review filter ("all", "empty", or "orphaned") and last jumped issue time.
  coverageFilter = $state("all");
  coverageFocusTime = $state(null);
  // Global Ctrl+K Command Palette open state and last loaded active shortcuts.
  commandPaletteOpen = $state(false);
  commandPaletteShortcuts = $state.raw(null);
  // Active editor workflow mode ("hitsounds" | "drums" | "keysounds") and floating Micro-HUD anchor.
  mode = $state("hitsounds");
  microHud = $state.raw(null);
  // Top-level 3-path Mapset Intake Card payload (shown outside #home so folder drops work while editing).
  intakeMapset = $state.raw(null);

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

  setLaneOptions(usedOnly, laneFilter, groupBySampleSet = true) {
    this.usedOnly = !!usedOnly;
    this.laneFilter = laneFilter ?? "";
    this.groupBySampleSet = groupBySampleSet !== false;
  }

  setHint(zone) {
    const next = zone || null;
    if (this.hint !== next) this.hint = next;
  }

  toggleMapPreview() {
    this.mapPreview = !this.mapPreview;
  }

  openQuickStart() {
    this.quickStart = 1;
  }

  setNodeGraphViewer(viewer) {
    this.nodeGraphViewer = viewer ?? null;
  }

  setHitCard(card) {
    this.hitCard = card;
  }

  setVolumeBubble(next) {
    if (this.volumeBubble?.editing && next && !next.editing) return;
    this.volumeBubble = next ?? null;
  }

  setStatusNotice(text) {
    this.statusNotice = text || "";
  }

  setCoverageFocus(time) {
    this.coverageFocusTime = Number.isFinite(time) ? time : null;
  }

  setMode(mode) {
    this.mode = mode === "drums" || mode === "keysounds" ? mode : "hitsounds";
  }

  setMicroHud(hud) {
    this.microHud = hud ?? null;
  }

  showMicroHud(x, y, extra = {}) {
    this.microHud = { x, y, mode: this.mode, ...extra };
  }

  openMapsetIntake(mapset) {
    this.intakeMapset = mapset ?? null;
  }

  closeMapsetIntake() {
    this.intakeMapset = null;
  }

  wheelSnap(finer) {
    const scene = this.scene;
    if (!scene) return null;
    return stepInGroup(nextGroup(this.snapGroup, scene.beats, scene.divisor), scene.beats, scene.divisor, finer);
  }

  _linkPreviewWindow() {
    this._previewLink = true;
    const events = globalThis.window?.__TAURI__?.event;
    if (!events) return;
    events.listen("preview-window", ({ payload }) => {
      this.previewWindow = payload === true;
      // With the window open the Inspector panel is on too, so its "Show it here" can be reached
      // (View › Map preview is not remembered, but the window is).
      if (this.previewWindow) this.mapPreview = true;
      if (this.previewWindow) this._sendPreview(true);
    });
    // The page being ready also says the window is open, should its open event have come too soon.
    events.listen("preview-ready", () => {
      this.previewWindow = true;
      this.mapPreview = true;
      this._sendPreview(true);
    });
  }

  _sendPreview(force = false) {
    const view = this.latestView ?? this.view;
    const scene = this.scene;
    const payload = {
      view: view && { position: view.position, playing: view.playing, speed: view.speed, duration: view.duration },
      path: scene?.path ?? null,
      previewKey: scene?.previewKey ?? null,
    };
    // Only what changed goes: paused and unmoved, the 1 s updates send nothing.
    const sent = JSON.stringify(payload);
    if (sent === this._lastSent && !force) return;
    this._lastSent = sent;
    globalThis.window?.__TAURI__?.event.emitTo("preview", "preview-view", payload).catch(() => {});
  }

  update(view, scene) {
    this.set(view, scene);
    if (!this._previewLink) this._linkPreviewWindow();
    if (this.previewWindow) this._sendPreview();
  }

  actions = actions;
  modals = modals;
  registerAction = registerAction;
  callAction = callAction;
  registerModal = registerModal;
  openModal = openModal;
  setModalOpen = setModalOpen;
  hasOpenModal = hasOpenModal;
  setShortcutCapturing = setShortcutCapturing;
  isShortcutCapturing = isShortcutCapturing;
  registerToast = registerToast;
  showToast = showToast;
  registerPaletteMenu = registerPaletteMenu;
  getPaletteMenu = getPaletteMenu;

  editorApi = {
    commands: commandList,
    update: (view, scene) => this.update(view, scene),
    lanes: headers => this.handOverLanes(headers),
    inspector: model => this.handOverInspector(model),
    browser: model => this.handOverBrowser(model),
    mixer: strips => this.handOverMixer(strips),
    menu: model => this.handOverMenu(model),
    toggleMapPreview: () => this.toggleMapPreview(),
    mapPreviewOn: () => this.mapPreview,
    openQuickStart: () => this.openQuickStart(),
    setNodeGraphViewer: viewer => this.setNodeGraphViewer(viewer),
    nodeGraphViewer: () => this.nodeGraphViewer,
    bottomTab: () => this.bottomTab,
    hitCard: card => this.setHitCard(card),
    volumeBubble: next => this.setVolumeBubble(next),
    centre: () => this.centre,
    wheelSnap: finer => this.wheelSnap(finer),
    snapGroup: () => this.snapGroup?.name,
    statusNotice: text => this.setStatusNotice(text),
    viewport: (zoom, rowHeight) => this.setViewport(zoom, rowHeight),
    laneOptions: (usedOnly, filter, groupBySampleSet = true) => this.setLaneOptions(usedOnly, filter, groupBySampleSet),
    coverageFilter: () => this.coverageFilter,
    setCoverageFocus: time => this.setCoverageFocus(time),
    hint: zone => this.setHint(zone),
    setMode: mode => this.setMode(mode),
    setMicroHud: hud => this.setMicroHud(hud),
    openMapsetIntake: mapset => this.openMapsetIntake(mapset),
    closeMapsetIntake: () => this.closeMapsetIntake(),
  };

  set(view, scene) {
    this.latestView = view;
    if (scene !== this.scene) {
      this.snapGroup = followGroup(this.snapGroup, this.scene, scene);
      this.scene = scene;
      this.zoom = scene ? scene.zoom : null;
      this.rowHeight = scene ? scene.rowHeight : null;
      if (!scene && this.hint !== null) this.hint = null;
    }
    if (!samePlayhead(this.playhead, view)) this.playhead = view;
    if (!sameUiView(this.view, view)) this.view = view;
  }
}

export const bridge = new Bridge();

// Routes editor actions through the bridge registry (falling back to globalThis for isolated tests).
export const act = action => bridge.callAction("act", action);
